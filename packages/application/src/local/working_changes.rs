//! Uncommitted changes in a conversation's working directory, read through
//! the local `git` CLI. Git stays the authority; the output is parsed at this
//! boundary into typed changes and diff lines. Only reads: nothing here
//! stages, commits or takes the repository's optional locks.
use super::*;
use crate::{ChangeKind, ChangedFile, DiffLine, DiffLineKind, FileDiff, WorkingChanges};
use std::process::Command;

/// Diff text beyond this many bytes is cut; the rest stays in git.
const DIFF_LIMIT: usize = 1024 * 1024;

fn git(directory: &Path, args: &[&str]) -> Result<std::process::Output, RuntimeError> {
    Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(["-c", "core.quotePath=false", "-c", "color.ui=false"])
        .args(args)
        // Status may otherwise refresh the index and contend with a Harness
        // running git in the same repository.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| RuntimeError::context("git 启动失败", error))
}

fn git_text(directory: &Path, args: &[&str]) -> Result<String, RuntimeError> {
    let output = git(directory, args)?;
    if !output.status.success() {
        return Err(RuntimeError::Invalid(format!(
            "git {} 失败：{}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

impl CoreRuntime {
    /// The working directory of a conversation, from its open session or the
    /// runtime's latest listing.
    fn conversation_directory(&self, runtime_id: &str, id: &str) -> Result<PathBuf, RuntimeError> {
        let runtime = self.runtime_instance(runtime_id)?;
        if !self.execution_context(runtime)?.target.is_local() {
            return Err(RuntimeError::invalid("远端运行时的会话暂不支持查看变更"));
        }
        std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| session.presents(id))
            .and_then(|session| session.projected()?.conversation.cwd.clone())
            .or_else(|| self.listed_summary(runtime, id)?.cwd)
            .map(PathBuf::from)
            .ok_or_else(|| RuntimeError::invalid("此会话没有工作目录"))
    }

    /// The branch checked out in the conversation's working directory; absent
    /// outside a repository or on a detached HEAD.
    pub(super) fn working_branch(&self, request: &Value) -> Result<Option<String>, RuntimeError> {
        let (runtime_id, id) = conversation_address(request)?;
        let directory = self.conversation_directory(runtime_id, id)?;
        let output = git(&directory, &["symbolic-ref", "--short", "-q", "HEAD"])?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|branch| !branch.is_empty()))
    }

    pub(super) fn working_changes(&self, request: &Value) -> Result<WorkingChanges, RuntimeError> {
        let (runtime_id, id) = conversation_address(request)?;
        let directory = self.conversation_directory(runtime_id, id)?;
        let probe = git(&directory, &["rev-parse", "--show-toplevel"])?;
        if !probe.status.success() {
            return Ok(WorkingChanges {
                directory: directory.to_string_lossy().into_owned(),
                repository: None,
                branch: None,
                files: Vec::new(),
            });
        }
        let root = PathBuf::from(String::from_utf8_lossy(&probe.stdout).trim());
        let status = git_text(
            &root,
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
            ],
        )?;
        let (branch, mut files) = parse_status(&status);
        // Line counts against HEAD cover staged and unstaged edits together;
        // a repository without commits has no HEAD to count against.
        if let Ok(numstat) = git_text(&root, &["diff", "HEAD", "--numstat", "-z", "--no-renames"]) {
            let counts = parse_numstat(&numstat);
            for file in &mut files {
                if let Some(&(additions, deletions)) = counts.get(&file.path) {
                    file.additions = additions;
                    file.deletions = deletions;
                }
            }
        }
        Ok(WorkingChanges {
            directory: directory.to_string_lossy().into_owned(),
            repository: Some(root.to_string_lossy().into_owned()),
            branch,
            files,
        })
    }

    pub(super) fn file_diff(&self, request: &Value) -> Result<FileDiff, RuntimeError> {
        let (runtime_id, id) = conversation_address(request)?;
        let path = request["payload"]["path"]
            .as_str()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| RuntimeError::invalid("file path"))?;
        let directory = self.conversation_directory(runtime_id, id)?;
        let root = PathBuf::from(git_text(&directory, &["rev-parse", "--show-toplevel"])?.trim());
        // Paths come from this repository's status; anything else is refused.
        let relative = Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(RuntimeError::invalid("file path"));
        }
        let tracked = git(&root, &["ls-files", "--error-unmatch", "--", path])?
            .status
            .success();
        let output = if tracked {
            let head = git(&root, &["rev-parse", "--verify", "--quiet", "HEAD"])?;
            let base = if head.status.success() {
                "HEAD"
            } else {
                "--cached"
            };
            git(
                &root,
                &["diff", base, "--no-ext-diff", "--no-renames", "--", path],
            )?
        } else {
            // Exit status 1 means the files differ.
            git(
                &root,
                &[
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--",
                    "/dev/null",
                    path,
                ],
            )?
        };
        if output.status.code().is_none_or(|code| code > 1) {
            return Err(RuntimeError::Invalid(format!(
                "git diff 失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let truncated = output.stdout.len() > DIFF_LIMIT;
        let text = String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(DIFF_LIMIT)]);
        Ok(FileDiff {
            path: path.into(),
            lines: parse_diff(&text),
            truncated,
        })
    }
}

fn conversation_address(request: &Value) -> Result<(&str, &str), RuntimeError> {
    Ok((
        request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?,
        request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?,
    ))
}

/// `git status --porcelain=v2 --branch -z` records.
fn parse_status(output: &str) -> (Option<String>, Vec<ChangedFile>) {
    let mut branch = None;
    let mut files = Vec::new();
    let mut records = output.split('\0');
    while let Some(record) = records.next() {
        let file = |kind, path: &str| ChangedFile {
            path: path.into(),
            kind,
            staged: false,
            unstaged: false,
            additions: None,
            deletions: None,
        };
        let mut fields = record.splitn(2, ' ');
        match (fields.next(), fields.next()) {
            (Some("#"), Some(header)) => {
                if let Some(name) = header.strip_prefix("branch.head ") {
                    branch = (name != "(detached)").then(|| name.to_owned());
                }
            }
            (Some(kind @ ("1" | "2")), Some(rest)) => {
                // XY sub mH mI mW hH hI [Xscore] path; renames carry the
                // original path in the next record.
                let fields: Vec<&str> = rest.splitn(if kind == "1" { 8 } else { 9 }, ' ').collect();
                let Some(path) = fields.last() else { continue };
                let &[index, worktree] = fields[0].as_bytes() else {
                    continue;
                };
                let change = if kind == "2" {
                    ChangeKind::Renamed {
                        from: records.next().unwrap_or_default().into(),
                    }
                } else {
                    match (index, worktree) {
                        (b'A', _) => ChangeKind::Added,
                        (b'D', _) | (_, b'D') => ChangeKind::Deleted,
                        _ => ChangeKind::Modified,
                    }
                };
                let mut changed = file(change, path);
                changed.staged = index != b'.';
                changed.unstaged = worktree != b'.';
                files.push(changed);
            }
            (Some("u"), Some(rest)) => {
                if let Some(path) = rest.splitn(10, ' ').nth(9) {
                    let mut changed = file(ChangeKind::Conflicted, path);
                    changed.unstaged = true;
                    files.push(changed);
                }
            }
            (Some("?"), Some(path)) => {
                let mut changed = file(ChangeKind::Untracked, path);
                changed.unstaged = true;
                files.push(changed);
            }
            _ => {}
        }
    }
    (branch, files)
}

/// `git diff --numstat -z` without renames: `added\tdeleted\tpath`; binary
/// files count as `-`.
fn parse_numstat(output: &str) -> BTreeMap<String, (Option<u32>, Option<u32>)> {
    output
        .split('\0')
        .filter_map(|record| {
            let mut fields = record.splitn(3, '\t');
            let additions = fields.next()?.parse().ok();
            let deletions = fields.next()?.parse().ok();
            Some((fields.next()?.to_owned(), (additions, deletions)))
        })
        .collect()
}

fn parse_diff(text: &str) -> Vec<DiffLine> {
    let mut in_hunk = false;
    text.lines()
        .filter_map(|line| {
            let (kind, text) = if line.starts_with("@@") {
                in_hunk = true;
                (DiffLineKind::Hunk, line)
            } else if !in_hunk {
                // File headers repeat what the list shows; only notices such
                // as binary content or mode changes carry information.
                if line.starts_with("Binary files")
                    || line.starts_with("old mode")
                    || line.starts_with("new mode")
                {
                    (DiffLineKind::Notice, line)
                } else {
                    return None;
                }
            } else if let Some(rest) = line.strip_prefix('+') {
                (DiffLineKind::Added, rest)
            } else if let Some(rest) = line.strip_prefix('-') {
                (DiffLineKind::Removed, rest)
            } else if let Some(rest) = line.strip_prefix(' ') {
                (DiffLineKind::Context, rest)
            } else if line.starts_with('\\') {
                (DiffLineKind::Notice, line)
            } else if line.starts_with("diff ") {
                in_hunk = false;
                return None;
            } else {
                (DiffLineKind::Context, line)
            };
            Some(DiffLine {
                kind,
                text: text.into(),
            })
        })
        .collect()
}
