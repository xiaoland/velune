//! Shared fixtures for Velune's test suites: isolated directories,
//! fake executables and deadline-bounded waits. Dev-dependency only.
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// A unique temporary directory that is removed on drop, even after a panic.
///
/// The path is canonical, so native IDs, indexes and child-process working
/// directories agree on platforms whose temporary directory is an alias
/// (macOS `/var` → `/private/var`).
pub struct TempRoot {
    path: PathBuf,
    // Held only for its drop; `None` once kept for debugging.
    _dir: Option<tempfile::TempDir>,
}

impl TempRoot {
    /// Create `velune-<label>-<random>` in the system temporary directory.
    pub fn new(label: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("velune-{label}-"))
            .tempdir()
            .expect("create test temporary directory");
        let path = dir
            .path()
            .canonicalize()
            .expect("canonicalize test temporary directory");
        if std::env::var_os("VELUNE_TEST_KEEP").is_some() {
            eprintln!("VELUNE_TEST_KEEP: {}", path.display());
            let _ = dir.keep();
            return Self { path, _dir: None };
        }
        Self {
            path,
            _dir: Some(dir),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.path.join(relative)
    }

    /// Create a directory (and its parents) below the root.
    pub fn dir(&self, relative: impl AsRef<Path>) -> PathBuf {
        let path = self.join(relative);
        fs::create_dir_all(&path).expect("create test directory");
        path
    }

    /// Write a file below the root, creating parent directories.
    pub fn write(&self, relative: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create test file parent");
        }
        fs::write(&path, contents).expect("write test file");
        path
    }

    /// Write an owner-executable script, such as a fake Harness binary.
    pub fn executable(&self, relative: impl AsRef<Path>, script: impl AsRef<[u8]>) -> PathBuf {
        let path = self.write(relative, script);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("mark test script executable");
        }
        path
    }

    /// Create an empty marker file, e.g. to release a gated fake process.
    pub fn touch(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.write(relative, [])
    }

    pub fn exists(&self, relative: impl AsRef<Path>) -> bool {
        self.join(relative).exists()
    }

    /// Read a UTF-8 file below the root; panics with the path when missing.
    pub fn read(&self, relative: impl AsRef<Path>) -> String {
        let path = self.join(relative);
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    /// Wait until a marker file exists below the root.
    pub fn wait_for(&self, relative: impl AsRef<Path>, timeout: Duration) -> bool {
        let path = self.join(relative);
        wait_until(timeout, || path.exists())
    }
}

impl AsRef<Path> for TempRoot {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// Serializes as its path, so fixtures can embed the root in JSON configs.
impl serde::Serialize for TempRoot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.path.serialize(serializer)
    }
}

impl std::ops::Deref for TempRoot {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

/// Materialize a three-turn synthetic session written by the Pi 1.4 SDK.
/// Only the native session header's working directory varies per test.
pub fn write_pi_three_turn_session(agent: &Path, workspace: &Path, model_url: &str) -> PathBuf {
    use serde_json::{Value, json};

    let (header, entries) = include_str!("../fixtures/pi/three-turns.jsonl")
        .split_once('\n')
        .unwrap();
    let mut header: Value = serde_json::from_str(header).unwrap();
    header["cwd"] = json!(workspace);
    let sessions = agent.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let source = sessions.join("three-turns.jsonl");
    fs::write(&source, format!("{header}\n{entries}")).unwrap();
    let provider = json!({
        "baseUrl": model_url, "api": "openai-completions", "apiKey": "synthetic",
        "models": [{"id": "alpha", "name": "Synthetic", "reasoning": false, "input": ["text"],
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            "contextWindow": 32768, "maxTokens": 4096}]
    });
    fs::write(
        agent.join("models.json"),
        json!({"providers": {"synthetic": provider}}).to_string(),
    )
    .unwrap();
    source
}

/// Poll `ready` every few milliseconds until it holds or `timeout` elapses.
/// Returns whether it held, so callers assert with their own message.
pub fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if ready() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_unique_canonical_and_removed_on_drop() {
        let first = TempRoot::new("support");
        let second = TempRoot::new("support");
        assert_ne!(first.path(), second.path());
        assert_eq!(first.path(), first.path().canonicalize().unwrap());
        let path = first.path().to_owned();
        drop(first);
        assert!(!path.exists());
    }

    #[test]
    fn root_is_removed_after_a_panic() {
        let leaked = std::sync::Mutex::new(PathBuf::new());
        let result = std::panic::catch_unwind(|| {
            let root = TempRoot::new("support-panic");
            *leaked.lock().unwrap() = root.path().to_owned();
            panic!("test body failed");
        });
        assert!(result.is_err());
        assert!(!leaked.lock().unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "no GUI host here; the fixture crate cannot depend on agent-runtime"
    )]
    fn executable_runs_and_markers_gate_waits() {
        let root = TempRoot::new("support-exec");
        let script = root.executable("bin/fake", "#!/bin/sh\nprintf ready > \"$1\"\n");
        assert!(!root.wait_for("marker", Duration::from_millis(20)));
        let status = std::process::Command::new(&script)
            .arg(root.join("marker"))
            .status()
            .unwrap();
        assert!(status.success());
        assert!(root.wait_for("marker", Duration::from_secs(1)));
        assert_eq!(root.read("marker"), "ready");
    }
}
