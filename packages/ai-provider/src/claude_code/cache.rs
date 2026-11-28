//! Short-lived, private Claude transcripts that can be resumed after the child exits.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const RETENTION: Duration = Duration::from_secs(3600);
const SWEEP_INTERVAL: Duration = Duration::from_secs(600);
const MAX_TRANSCRIPT: u64 = 64 * 1024 * 1024;
const TAIL_BYTES: u64 = 256 * 1024;

pub(super) struct Cache {
    root: PathBuf,
    entries: Mutex<Entries>,
}

#[derive(Default)]
struct Entries {
    active: HashSet<PathBuf>,
    last_sweep: Option<Instant>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Checkpoint {
    session: uuid::Uuid,
    signature: String,
    messages: usize,
    history: String,
    end: u64,
    assistant: String,
}

pub(super) struct Transcript {
    cache: Arc<Cache>,
    entry: PathBuf,
    pub(super) file: PathBuf,
    pub(super) prefix: usize,
    session: uuid::Uuid,
    signature: String,
    sealed: bool,
}

impl Cache {
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root: root.join("v1"),
            entries: Mutex::default(),
        }
    }

    pub(super) fn work_dir(&self) -> io::Result<PathBuf> {
        let directory = self.root.join("work");
        private_directory(&directory)?;
        Ok(directory)
    }

    pub(super) fn prepare(
        self: &Arc<Self>,
        owner: &impl Serialize,
        signature: &Value,
        history: &[Value],
        cwd: &Path,
    ) -> io::Result<Option<Transcript>> {
        let cwd = cwd.to_str().ok_or(io::ErrorKind::InvalidInput)?;
        let entry = self.root.join("sessions").join(digest(owner));
        {
            let mut entries = self.entries.lock().expect("Claude bridge cache entries");
            if entries
                .last_sweep
                .is_none_or(|last| last.elapsed() >= SWEEP_INTERVAL)
            {
                Self::sweep(&self.root.join("sessions"), &entries.active);
                entries.last_sweep = Some(Instant::now());
            }
            if !entries.active.insert(entry.clone()) {
                return Ok(None);
            }
        }
        let session = uuid::Uuid::new_v4();
        let mut transcript = Transcript {
            cache: self.clone(),
            file: transcript_path(&entry, session),
            entry,
            prefix: 0,
            session,
            signature: digest(signature),
            sealed: false,
        };
        private_directory(&transcript.entry)?;
        let previous = transcript.read_checkpoint(history);
        remove_file(&transcript.marker())?;
        if let Some(previous) = previous {
            transcript.session = previous.session;
            transcript.file = transcript_path(&transcript.entry, previous.session);
            OpenOptions::new()
                .write(true)
                .open(&transcript.file)?
                .set_len(previous.end)?;
            transcript.prefix = previous.messages;
        } else {
            fs::remove_dir_all(&transcript.entry)?;
            private_directory(&transcript.entry)?;
            let mut file = private_file(&transcript.file)?;
            // A system record gives --resume a local session ID without
            // introducing a user or assistant message into the model input.
            serde_json::to_writer(
                &mut file,
                &json!({
                    "type": "system", "subtype": "velune_bridge",
                    "uuid": uuid::Uuid::new_v4(), "parentUuid": null,
                    "sessionId": session, "cwd": cwd,
                    "timestamp": "1970-01-01T00:00:00.000Z",
                }),
            )?;
            file.write_all(b"\n")?;
        }
        Ok(Some(transcript))
    }

    fn sweep(directory: &Path, active: &HashSet<PathBuf>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if active.contains(&path) || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            // Incomplete runs have no marker. A checksum or format mismatch
            // is handled when its owner next tries to resume.
            if !fresh(&path.join("checkpoint.json")) {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}

impl Transcript {
    fn marker(&self) -> PathBuf {
        self.entry.join("checkpoint.json")
    }

    fn read_checkpoint(&self, history: &[Value]) -> Option<Checkpoint> {
        let marker = self.marker();
        if !fresh(&marker) {
            return None;
        }
        let checkpoint: Checkpoint =
            serde_json::from_reader(File::open(marker).ok()?.take(4096)).ok()?;
        let prefix = history.get(..checkpoint.messages)?;
        if checkpoint.messages == 0
            || history.len() - checkpoint.messages != 1
            || history.last()?["role"] != "user"
            || checkpoint.signature != self.signature
            || checkpoint.history != digest(prefix)
            || !ends_with_assistant(
                &transcript_path(&self.entry, checkpoint.session),
                checkpoint.end,
                &checkpoint.assistant,
            )
            .ok()?
        {
            return None;
        }
        Some(checkpoint)
    }

    pub(super) fn begin(&mut self) -> io::Result<()> {
        if self.sealed {
            remove_file(&self.marker())?;
            self.sealed = false;
        }
        Ok(())
    }

    pub(super) async fn checkpoint(
        &self,
        history: &[Value],
        assistant: &str,
    ) -> Option<Checkpoint> {
        // The CLI can emit result before flushing its native transcript.
        // A dropped or slow flush leaves this optional cache uncommitted.
        for attempt in 0..8 {
            if attempt != 0 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let end = fs::metadata(&self.file).ok()?.len();
            if end > MAX_TRANSCRIPT {
                return None;
            }
            if ends_with_assistant(&self.file, end, assistant).unwrap_or(false) {
                return Some(Checkpoint {
                    session: self.session,
                    signature: self.signature.clone(),
                    messages: history.len(),
                    history: digest(history),
                    end,
                    assistant: assistant.to_owned(),
                });
            }
        }
        None
    }

    pub(super) fn seal(&mut self, checkpoint: Checkpoint) {
        self.sealed = self.publish(checkpoint).is_ok();
    }

    fn publish(&self, checkpoint: Checkpoint) -> io::Result<()> {
        let pending = self.entry.join("pending.json");
        remove_file(&pending)?;
        let mut file = private_file(&pending)?;
        serde_json::to_writer(&mut file, &checkpoint)?;
        file.sync_all()?;
        drop(file);
        fs::rename(pending, self.marker())
    }
}

impl Drop for Transcript {
    fn drop(&mut self) {
        let mut entries = self
            .cache
            .entries
            .lock()
            .expect("Claude bridge cache entries");
        if !self.sealed {
            let _ = fs::remove_dir_all(&self.entry);
        }
        entries.active.remove(&self.entry);
    }
}

fn transcript_path(entry: &Path, session: uuid::Uuid) -> PathBuf {
    entry.join(format!("{session}.jsonl"))
}

fn ends_with_assistant(path: &Path, end: u64, uuid: &str) -> io::Result<bool> {
    if end == 0 || end > MAX_TRANSCRIPT {
        return Ok(false);
    }
    let mut file = File::open(path)?;
    let begin = end.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(begin))?;
    let mut bytes = vec![0; (end - begin) as usize];
    file.read_exact(&mut bytes)?;
    if bytes.last() != Some(&b'\n') {
        return Ok(false);
    }
    // Any leading fragment of the bounded tail is not a complete record.
    if begin != 0 {
        let Some(line_end) = bytes.iter().position(|byte| *byte == b'\n') else {
            return Ok(false);
        };
        bytes.drain(..=line_end);
    }
    for line in bytes
        .rsplit(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: Value = serde_json::from_slice(line)?;
        if entry["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        match entry["type"].as_str() {
            Some("assistant") => return Ok(entry["uuid"].as_str() == Some(uuid)),
            Some("user" | "summary") => return Ok(false),
            _ => {}
        }
    }
    Ok(false)
}

fn digest(value: &(impl Serialize + ?Sized)) -> String {
    let mut hash = Sha256::new();
    serde_json::to_writer(&mut hash, value).expect("serializable Claude bridge cache identity");
    format!("{:x}", hash.finalize())
}

fn fresh(path: &Path) -> bool {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age < RETENTION))
}

fn private_directory(path: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
