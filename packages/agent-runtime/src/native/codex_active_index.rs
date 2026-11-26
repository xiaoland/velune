//! Reuse Codex's active index after an authoritative read of unchanged rollouts.
use super::super::{
    Result,
    codex_history::{pages, thread_list_params},
    rpc::RpcClient,
};
use crate::{creation::Retained, history::HistoryConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const PROOF_SCHEMA_VERSION: u32 = 1;
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Shared by startup prewarming and list requests for one native source.
pub struct ActiveListIndex {
    state: Mutex<IndexState>,
    storage: Option<PathBuf>,
}

#[derive(Default)]
struct IndexState {
    loaded: bool,
    writable: bool,
    verified: Option<VerifiedList>,
}

impl Default for ActiveListIndex {
    fn default() -> Self {
        Self {
            state: Mutex::new(IndexState {
                loaded: true,
                writable: false,
                verified: None,
            }),
            storage: None,
        }
    }
}

impl ActiveListIndex {
    pub fn with_storage(path: PathBuf) -> Self {
        Self {
            state: Mutex::new(IndexState {
                loaded: false,
                writable: true,
                verified: None,
            }),
            storage: Some(path),
        }
    }

    pub fn supports(config: &HistoryConfig) -> bool {
        cfg!(unix)
            && config.kind == crate::version::RuntimeKind::Codex166
            && config.execution.is_local()
    }

    pub(super) fn prepare<'a>(&'a self, config: &'a HistoryConfig) -> Option<ActiveRead<'a>> {
        if !Self::supports(config) {
            return None;
        }
        let mut state = self.state.lock().ok()?;
        if !state.loaded {
            state.loaded = true;
            if let Some(path) = &self.storage {
                match load_persisted(path) {
                    LoadOutcome::Missing => {
                        state.writable = true;
                        state.verified = None;
                    }
                    LoadOutcome::Loaded(verified) => {
                        state.writable = true;
                        state.verified = Some(verified);
                    }
                    LoadOutcome::PreservedInvalid => {
                        state.writable = false;
                        state.verified = None;
                    }
                }
            }
        }
        Some(ActiveRead {
            state,
            storage: self.storage.as_deref(),
            config,
            config_hash: config_digest(config),
            before: Rollouts::capture(config)
                .ok()
                .map(|rollouts| rollouts.digest()),
        })
    }
}

pub(super) struct ActiveRead<'a> {
    state: MutexGuard<'a, IndexState>,
    storage: Option<&'a Path>,
    config: &'a HistoryConfig,
    config_hash: [u8; 32],
    before: Option<[u8; 32]>,
}

impl ActiveRead<'_> {
    pub(super) fn matches_verified(&self) -> bool {
        matches!(
            self.state.verified,
            Some(verified)
                if verified.config == self.config_hash && self.before == Some(verified.rollouts)
        )
    }

    pub(super) fn list_threads(&mut self, rpc: &mut RpcClient) -> Result<Retained<Vec<Value>>> {
        let params = thread_list_params(false);
        if let Some(verified) = self.state.verified
            && verified.config == self.config_hash
            && self.before == Some(verified.rollouts)
        {
            let mut indexed = params.clone();
            indexed["useStateDbOnly"] = json!(true);
            if let Ok(threads) = pages(rpc, "thread/list", indexed)
                // A replaced or partially rebuilt database can return a successful short list.
                && result_digest(&threads) == Some(verified.result)
                && Rollouts::capture(self.config)
                    .ok()
                    .map(|rollouts| rollouts.digest())
                    == Some(verified.rollouts)
            {
                tracing::debug!(event = "codex_active_index", mode = "indexed");
                return Ok(threads);
            }
        }

        self.state.verified = None;
        let threads = pages(rpc, "thread/list", params)?;
        if let Some(before) = self.before.take()
            && Rollouts::capture(self.config)
                .ok()
                .map(|rollouts| rollouts.digest())
                == Some(before)
            && let Some(result) = result_digest(&threads)
        {
            let verified = VerifiedList {
                config: self.config_hash,
                rollouts: before,
                result,
            };
            self.state.verified = Some(verified);
            if self.state.writable
                && let Some(path) = self.storage
            {
                save_persisted(path, &verified);
            }
        }
        tracing::debug!(event = "codex_active_index", mode = "repaired");
        Ok(threads)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VerifiedList {
    config: [u8; 32],
    rollouts: [u8; 32],
    result: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedProof {
    schema_version: u32,
    config_sha256: String,
    rollouts_sha256: String,
    result_sha256: String,
}

enum LoadOutcome {
    Missing,
    Loaded(VerifiedList),
    PreservedInvalid,
}

fn load_persisted(path: &Path) -> LoadOutcome {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return LoadOutcome::Missing,
        Err(error) => {
            tracing::warn!(
                target: "velune_agent_runtime",
                event = "codex_active_index_load_failed",
                path = %path.display(),
                detail = %error
            );
            return LoadOutcome::PreservedInvalid;
        }
    };
    let proof: PersistedProof = match serde_json::from_slice(&bytes) {
        Ok(proof) => proof,
        Err(error) => {
            tracing::warn!(
                target: "velune_agent_runtime",
                event = "codex_active_index_load_failed",
                path = %path.display(),
                detail = %error
            );
            return LoadOutcome::PreservedInvalid;
        }
    };
    if proof.schema_version != PROOF_SCHEMA_VERSION {
        tracing::warn!(
            target: "velune_agent_runtime",
            event = "codex_active_index_load_failed",
            path = %path.display(),
            schema_version = proof.schema_version,
            detail = "unsupported schema version"
        );
        return LoadOutcome::PreservedInvalid;
    }
    let (Some(config), Some(rollouts), Some(result)) = (
        hex_decode_32(&proof.config_sha256),
        hex_decode_32(&proof.rollouts_sha256),
        hex_decode_32(&proof.result_sha256),
    ) else {
        tracing::warn!(
            target: "velune_agent_runtime",
            event = "codex_active_index_load_failed",
            path = %path.display(),
            detail = "invalid sha256 hex digest"
        );
        return LoadOutcome::PreservedInvalid;
    };
    LoadOutcome::Loaded(VerifiedList {
        config,
        rollouts,
        result,
    })
}

fn save_persisted(path: &Path, verified: &VerifiedList) {
    if let Err(error) = write_persisted(path, verified) {
        tracing::warn!(
            target: "velune_agent_runtime",
            event = "codex_active_index_save_failed",
            path = %path.display(),
            detail = %error
        );
    }
}

fn write_persisted(path: &Path, verified: &VerifiedList) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
    }
    let proof = PersistedProof {
        schema_version: PROOF_SCHEMA_VERSION,
        config_sha256: hex_encode(&verified.config),
        rollouts_sha256: hex_encode(&verified.rollouts),
        result_sha256: hex_encode(&verified.result),
    };
    let encoded = serde_json::to_vec(&proof).map_err(io::Error::other)?;
    let seq = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("index");
    let temp = path.with_file_name(format!(".{file_name}.{}.{seq}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write_result = (|| -> io::Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(&encoded)?;
        file.flush()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode_32(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        *slot = (high << 4) | low;
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn result_digest(threads: &[Value]) -> Option<[u8; 32]> {
    let mut digest = Sha256::new();
    serde_json::to_writer(&mut digest, threads).ok()?;
    Some(digest.finalize().into())
}

fn hash_bytes(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn hash_path(digest: &mut Sha256, path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hash_bytes(digest, path.as_os_str().as_bytes());
    }
    #[cfg(not(unix))]
    {
        hash_bytes(digest, path.to_string_lossy().as_bytes());
    }
}

fn config_digest(config: &HistoryConfig) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"velune.codex_active_index.config.v1\0");
    hash_bytes(&mut digest, config.kind.id().as_bytes());
    digest.update([u8::from(config.execution.is_local())]);
    hash_bytes(&mut digest, config.provider.as_bytes());
    hash_path(&mut digest, &config.binary);
    hash_path(&mut digest, &config.node_binary);
    hash_path(&mut digest, &config.resources_directory);
    hash_path(&mut digest, &config.root);
    hash_path(&mut digest, &config.home);
    match &config.os_home {
        Some(os_home) => {
            digest.update([1]);
            hash_path(&mut digest, os_home);
        }
        None => digest.update([0]),
    }
    digest.finalize().into()
}

#[derive(PartialEq, Eq)]
struct Rollouts {
    binary: FileVersion,
    config: Option<FileVersion>,
    files: BTreeMap<PathBuf, FileVersion>,
}

impl Rollouts {
    fn digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"velune.codex_active_index.rollouts.v1\0");
        self.binary.hash_into(&mut digest);
        match &self.config {
            Some(config) => {
                digest.update([1]);
                config.hash_into(&mut digest);
            }
            None => digest.update([0]),
        }
        digest.update((self.files.len() as u64).to_le_bytes());
        for (path, version) in &self.files {
            hash_path(&mut digest, path);
            version.hash_into(&mut digest);
        }
        digest.finalize().into()
    }

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
    fn hash_into(&self, digest: &mut Sha256) {
        digest.update(self.size.to_le_bytes());
        match self.modified.duration_since(UNIX_EPOCH) {
            Ok(delta) => {
                digest.update([0]);
                digest.update(delta.as_secs().to_le_bytes());
                digest.update(delta.subsec_nanos().to_le_bytes());
            }
            Err(error) => {
                let delta = error.duration();
                digest.update([1]);
                digest.update(delta.as_secs().to_le_bytes());
                digest.update(delta.subsec_nanos().to_le_bytes());
            }
        }
        #[cfg(unix)]
        {
            digest.update(self.identity.0.to_le_bytes());
            digest.update(self.identity.1.to_le_bytes());
            digest.update(self.identity.2.to_le_bytes());
            digest.update(self.identity.3.to_le_bytes());
        }
    }

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
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_home(label: &str) -> (PathBuf, Cleanup) {
        let mut key = [0; 16];
        getrandom::fill(&mut key).unwrap();
        let home = std::env::temp_dir().join(format!("velune-{label}-{:032x}", u128::from_ne_bytes(key)));
        fs::create_dir(&home).unwrap();
        let cleanup = Cleanup(home.clone());
        (home, cleanup)
    }

    fn sample_config(home: &Path) -> HistoryConfig {
        let binary = home.join("codex");
        fs::write(&binary, []).unwrap();
        HistoryConfig {
            kind: crate::version::RuntimeKind::Codex166,
            execution: crate::execution::ExecutionTarget::Local,
            os_home: None,
            binary,
            node_binary: PathBuf::new(),
            resources_directory: home.to_path_buf(),
            root: home.to_path_buf(),
            home: home.to_path_buf(),
        }
    }

    #[test]
    fn rollout_versions_detect_compression_replacement_and_unknown_links() {
        let (home, _cleanup) = temp_home("rollout-versions");
        let config = sample_config(&home);

        let original = Rollouts::capture(&config).unwrap();
        let day = home.join("sessions/2025/01/01");
        fs::create_dir_all(&day).unwrap();
        let path = day.join("rollout-history.jsonl.zst");
        fs::write(&path, b"one").unwrap();
        let compressed = Rollouts::capture(&config).unwrap();
        assert!(original != compressed);
        assert_ne!(original.digest(), compressed.digest());

        let replacement = home.join("replacement");
        fs::write(&replacement, b"two").unwrap();
        // An atomic rewrite can preserve both the size and modification time.
        let times = fs::FileTimes::new().set_modified(path.metadata().unwrap().modified().unwrap());
        fs::File::open(&replacement)
            .unwrap()
            .set_times(times)
            .unwrap();
        fs::rename(replacement, &path).unwrap();
        let replaced = Rollouts::capture(&config).unwrap();
        assert!(compressed != replaced);
        assert_ne!(compressed.digest(), replaced.digest());

        fs::remove_file(&path).unwrap();
        assert!(original == Rollouts::capture(&config).unwrap());
        assert_eq!(original.digest(), Rollouts::capture(&config).unwrap().digest());
        symlink(&home, &path).unwrap();
        assert!(Rollouts::capture(&config).is_err());
    }

    #[test]
    fn persisted_proof_round_trips_and_detects_rollout_and_config_changes() {
        let (home, _cleanup) = temp_home("codex-proof");
        let config = sample_config(&home);
        let day = home.join("sessions/2026/11/26");
        fs::create_dir_all(&day).unwrap();
        let rollout = day.join("rollout-1.jsonl");
        fs::write(&rollout, b"{}\n").unwrap();

        let proof_path = home.join("state/codex-active-index/runtime.json");
        let verified = VerifiedList {
            config: config_digest(&config),
            rollouts: Rollouts::capture(&config).unwrap().digest(),
            result: [0x5a; 32],
        };
        write_persisted(&proof_path, &verified).unwrap();
        assert_eq!(
            fs::metadata(&proof_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let reloaded = ActiveListIndex::with_storage(proof_path.clone());
        {
            let read = reloaded.prepare(&config).unwrap();
            assert!(read.matches_verified());
            assert_eq!(read.state.verified, Some(verified));
        }

        // Changing a rollout invalidates matches_verified on the next prepare.
        fs::write(&rollout, b"{\"changed\":true}\n").unwrap();
        assert!(!reloaded.prepare(&config).unwrap().matches_verified());

        // Changing config.toml or binary invalidates matches_verified.
        let reloaded_again = ActiveListIndex::with_storage(proof_path.clone());
        let verified_after = VerifiedList {
            config: config_digest(&config),
            rollouts: Rollouts::capture(&config).unwrap().digest(),
            result: [0x3c; 32],
        };
        write_persisted(&proof_path, &verified_after).unwrap();
        fs::write(home.join("config.toml"), "model = 'gpt-5'\n").unwrap();
        assert!(!reloaded_again.prepare(&config).unwrap().matches_verified());

        let mut other_config = config.clone();
        other_config.provider = "other".into();
        let third = ActiveListIndex::with_storage(proof_path);
        assert!(!third.prepare(&other_config).unwrap().matches_verified());
    }

    #[test]
    fn unknown_and_corrupt_persisted_proofs_are_preserved_without_overwrite() {
        let (home, _cleanup) = temp_home("codex-proof-preserve");
        let config = sample_config(&home);

        for (name, bytes) in [
            ("unknown-schema.json", br#"{"schemaVersion":99,"future":true}"#.as_slice()),
            ("corrupt.json", br#"{"schemaVersion":1"#.as_slice()),
        ] {
            let path = home.join(name);
            fs::write(&path, bytes).unwrap();
            let index = ActiveListIndex::with_storage(path.clone());
            let read = index.prepare(&config).unwrap();
            assert!(!read.matches_verified());
            assert!(!read.state.writable);
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }
}
