//! Short-lived ownership of a newly written, not-yet-linked native import.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub type OwnedImport = (
    ImportedConversation,
    BTreeMap<String, Vec<String>>,
    ImportOwnership,
);

#[derive(Debug)]
pub struct ImportOwnership {
    path: PathBuf,
    imported: ImportedConversation,
    execution: ExecutionTarget,
    length: usize,
    digest: String,
}
impl ImportOwnership {
    pub(super) fn capture(
        execution: &ExecutionTarget,
        path: PathBuf,
        imported: ImportedConversation,
    ) -> Result<Self> {
        let bytes = execution.read_file(&path)?;
        Ok(Self {
            path,
            imported,
            execution: execution.clone(),
            length: bytes.len(),
            digest: format!("{:x}", Sha256::digest(&bytes)),
        })
    }
    pub fn source_path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn owns_codex(&self, id: &str) -> bool {
        matches!(&self.imported, ImportedConversation::Codex { thread_id } if thread_id == id)
    }

    /// The indexing child may append settings metadata but may never generate
    /// model input/output. A changed prefix or other appended records means
    /// another writer owns the source: leave it recoverable and refuse cleanup.
    pub(crate) fn refresh_after_materialize(
        &mut self,
        config: &crate::history::HistoryConfig,
    ) -> Result<()> {
        let bytes = config.execution.read_file(&self.path)?;
        if bytes.len() < self.length
            || format!("{:x}", Sha256::digest(&bytes[..self.length])) != self.digest
        {
            return Err(self.changed());
        }
        for line in bytes[self.length..]
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let entry: Value = serde_json::from_slice(line).map_err(|_| self.changed())?;
            if entry["type"] != "event_msg" || entry["payload"]["type"] != "thread_settings_applied"
            {
                return Err(self.changed());
            }
        }
        self.length = bytes.len();
        self.digest = format!("{:x}", Sha256::digest(&bytes));
        Ok(())
    }
    fn changed(&self) -> Error {
        Error::new(format!(
            "导入副本已被其它写入改变，保留来源以便恢复：{}",
            self.path.display()
        ))
    }
    pub(crate) fn verify(
        &self,
        config: &crate::history::HistoryConfig,
        unlink: bool,
    ) -> Result<String> {
        if config.execution != self.execution {
            return Err(Error::new("导入副本所有权不属于此执行设备"));
        }
        let mut command = config.execution.python();
        command.args(["-c", r#"
import hashlib, os, stat, sys
path, expected, action = sys.argv[1:]
info = os.lstat(path)
if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
    raise ValueError('owned import is not a single regular source')
fd = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0))
try:
    with os.fdopen(fd, 'rb') as source:
        opened = os.fstat(source.fileno())
        if (info.st_dev, info.st_ino) != (opened.st_dev, opened.st_ino) or hashlib.sha256(source.read()).hexdigest() != expected:
            raise ValueError('owned import changed; original copy was retained')
        current = os.lstat(path)
        if (current.st_dev, current.st_ino, current.st_size, current.st_mtime_ns) != (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns):
            raise ValueError('owned import changed during cleanup; original copy was retained')
        canonical = os.path.realpath(path)
        if action == 'unlink': os.unlink(path)
        print(canonical)
finally:
    pass
"#]).arg(&self.path).arg(&self.digest).arg(if unlink {"unlink"} else {"verify"});
        let output = config
            .execution
            .output(command, None, config.os_home.as_deref())
            .map_err(|error| {
                Error::new(format!(
                    "导入副本清理未完成，保留来源 {}：{error}",
                    self.path.display()
                ))
            })?;
        String::from_utf8(output)
            .map(|path| path.trim().to_owned())
            .map_err(|error| Error::new(error.to_string()))
    }
}

/// Call only before the new source's logical association is committed. On any
/// source change, retain it and return its recovery path instead of deleting it.
pub fn cleanup_owned_import(
    config: &crate::history::HistoryConfig,
    ownership: &ImportOwnership,
) -> Result<()> {
    let cleanup = (|| {
        match &ownership.imported {
            ImportedConversation::ClaudeCode { .. } => {
                ownership.verify(config, true)?;
            }
            ImportedConversation::Codex { thread_id } => {
                let canonical = ownership.verify(config, false)?;
                crate::native::cleanup_owned_codex_import(
                    config, thread_id, &canonical, ownership,
                )?;
            }
            ImportedConversation::Pi { .. } => {
                return Err(Error::new("Pi 原地操作没有导入副本所有权"));
            }
        }
        Ok(())
    })();
    cleanup.map_err(|error: Error| {
        Error::new(format!(
            "导入副本清理未完成，恢复来源 {}：{error}",
            ownership.source_path().display()
        ))
    })
}
