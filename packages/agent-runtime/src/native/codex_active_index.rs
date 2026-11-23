//! Reuse Codex's active index after an authoritative read of unchanged rollouts.
use super::super::{
    Result,
    codex_history::{pages, thread_list_params},
    rpc::RpcClient,
};
use crate::{creation::Retained, history::HistoryConfig};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::SystemTime,
};

/// Shared by startup prewarming and list requests for one native source.
#[derive(Default)]
pub struct ActiveListIndex(Mutex<Option<VerifiedList>>);

impl ActiveListIndex {
    pub fn supports(config: &HistoryConfig) -> bool {
        cfg!(unix)
            && config.kind == crate::version::RuntimeKind::Codex166
            && config.execution.is_local()
    }

    pub(super) fn prepare<'a>(&'a self, config: &'a HistoryConfig) -> Option<ActiveRead<'a>> {
        if !Self::supports(config) {
            return None;
        }
        let verified = self.0.lock().ok()?;
        Some(ActiveRead {
            verified,
            config,
            before: Rollouts::capture(config).ok(),
        })
    }
}

pub(super) struct ActiveRead<'a> {
    verified: MutexGuard<'a, Option<VerifiedList>>,
    config: &'a HistoryConfig,
    before: Option<Rollouts>,
}

impl ActiveRead<'_> {
    pub(super) fn list_threads(&mut self, rpc: &mut RpcClient) -> Result<Retained<Vec<Value>>> {
        let params = thread_list_params(false);
        if let Some(verified) = self.verified.as_ref()
            && verified.config == *self.config
            && self.before.as_ref() == Some(&verified.rollouts)
        {
            let mut indexed = params.clone();
            indexed["useStateDbOnly"] = json!(true);
            if let Ok(threads) = pages(rpc, "thread/list", indexed)
                // A replaced or partially rebuilt database can return a successful short list.
                && result_digest(&threads) == Some(verified.result)
                && Rollouts::capture(self.config).ok().as_ref() == Some(&verified.rollouts)
            {
                tracing::debug!(event = "codex_active_index", mode = "indexed");
                return Ok(threads);
            }
        }

        *self.verified = None;
        let threads = pages(rpc, "thread/list", params)?;
        if let Some(before) = self.before.take()
            && Rollouts::capture(self.config).ok().as_ref() == Some(&before)
            && let Some(result) = result_digest(&threads)
        {
            *self.verified = Some(VerifiedList {
                config: self.config.clone(),
                rollouts: before,
                result,
            });
        }
        tracing::debug!(event = "codex_active_index", mode = "repaired");
        Ok(threads)
    }
}

struct VerifiedList {
    config: HistoryConfig,
    rollouts: Rollouts,
    result: [u8; 32],
}

fn result_digest(threads: &[Value]) -> Option<[u8; 32]> {
    let mut digest = Sha256::new();
    serde_json::to_writer(&mut digest, threads).ok()?;
    Some(digest.finalize().into())
}

#[derive(PartialEq, Eq)]
struct Rollouts {
    binary: FileVersion,
    config: Option<FileVersion>,
    files: BTreeMap<PathBuf, FileVersion>,
}

impl Rollouts {
    fn capture(config: &HistoryConfig) -> io::Result<Self> {
        let binary = FileVersion::read(&config.binary)?;
        let configuration = if_present(|| FileVersion::read(&config.home.join("config.toml")))?;
        let sessions = config.home.join("sessions");
        let mut files = BTreeMap::new();
        if let Some(metadata) = if_present(|| fs::symlink_metadata(&sessions))? {
            if !metadata.is_dir() {
                return Err(io::ErrorKind::Unsupported.into());
            }
            let mut directories = vec![sessions];
            while let Some(directory) = directories.pop() {
                for entry in fs::read_dir(directory)? {
                    let entry = entry?;
                    let kind = entry.file_type()?;
                    let path = entry.path();
                    if kind.is_symlink() {
                        return Err(io::ErrorKind::Unsupported.into());
                    }
                    if kind.is_dir() {
                        directories.push(path);
                    } else {
                        // Codex decides which files are rollouts. Include compressed and
                        // migrated files without interpreting their names or contents.
                        files.insert(path, FileVersion::from_metadata(entry.metadata()?)?);
                    }
                }
            }
        }
        Ok(Self {
            binary,
            config: configuration,
            files,
        })
    }
}

fn if_present<T>(read: impl FnOnce() -> io::Result<T>) -> io::Result<Option<T>> {
    match read() {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(PartialEq, Eq)]
struct FileVersion {
    size: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl FileVersion {
    fn read(path: &Path) -> io::Result<Self> {
        Self::from_metadata(fs::metadata(path)?)
    }

    fn from_metadata(metadata: fs::Metadata) -> io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_file() {
            return Err(io::ErrorKind::Unsupported.into());
        }
        Ok(Self {
            size: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn rollout_versions_detect_compression_replacement_and_unknown_links() {
        let mut key = [0; 16];
        getrandom::fill(&mut key).unwrap();
        let home = std::env::temp_dir().join(format!(
            "velune-rollout-versions-{:032x}",
            u128::from_ne_bytes(key)
        ));
        fs::create_dir(&home).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(home.clone());
        let binary = home.join("codex");
        fs::write(&binary, []).unwrap();
        let config = HistoryConfig {
            kind: crate::version::RuntimeKind::Codex166,
            execution: crate::execution::ExecutionTarget::Local,
            os_home: None,
            provider: "codex".into(),
            binary,
            node_binary: PathBuf::new(),
            resources_directory: home.clone(),
            root: home.clone(),
            home: home.clone(),
        };

        let original = Rollouts::capture(&config).unwrap();
        let day = home.join("sessions/2025/01/01");
        fs::create_dir_all(&day).unwrap();
        let path = day.join("rollout-history.jsonl.zst");
        fs::write(&path, b"one").unwrap();
        let compressed = Rollouts::capture(&config).unwrap();
        assert!(original != compressed);

        let replacement = home.join("replacement");
        fs::write(&replacement, b"two").unwrap();
        // An atomic rewrite can preserve both the size and modification time.
        let times = fs::FileTimes::new().set_modified(path.metadata().unwrap().modified().unwrap());
        fs::File::open(&replacement)
            .unwrap()
            .set_times(times)
            .unwrap();
        fs::rename(replacement, &path).unwrap();
        assert!(compressed != Rollouts::capture(&config).unwrap());

        fs::remove_file(&path).unwrap();
        assert!(original == Rollouts::capture(&config).unwrap());
        symlink(&home, &path).unwrap();
        assert!(Rollouts::capture(&config).is_err());
    }
}
