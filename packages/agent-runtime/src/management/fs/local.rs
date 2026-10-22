//! Resource files on this machine. Every step opens the next path component
//! relative to an already open directory and never follows a link, so a link
//! swapped in along the path cannot redirect a read or a write. cap-std gives
//! these directory-relative operations on Unix and Windows alike; what the
//! platforms do differently (permission bits, write access, directory sync)
//! is in `platform` at the end.
use super::*;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, Metadata, OpenOptions},
};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read, Write},
};

const MAX_NODES: usize = 4096;

fn io_error(error: io::Error) -> Error {
    use io::ErrorKind::*;
    let code = match error.kind() {
        NotFound => ErrorCode::Missing,
        NotADirectory | IsADirectory | DirectoryNotEmpty => ErrorCode::Invalid,
        _ if platform::is_link_refused(&error) => ErrorCode::Invalid,
        _ => ErrorCode::Unavailable,
    };
    Error::new(code, format!("资源文件操作失败：{error}"))
}

fn metadata(parent: &Dir, name: &OsStr) -> Result<Option<Metadata>> {
    match parent.symlink_metadata(name) {
        Ok(info) => Ok(Some(info)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}

fn kind(info: &Metadata) -> Node {
    let kind = info.file_type();
    if kind.is_symlink() {
        Node::Symlink
    } else if kind.is_dir() {
        Node::Directory
    } else if kind.is_file() {
        Node::File
    } else {
        Node::Other
    }
}

fn node_name(node: Node) -> &'static str {
    match node {
        Node::File => "file",
        Node::Directory => "directory",
        Node::Symlink => "symlink",
        Node::Other => "other",
    }
}

fn directory(parent: &Dir, name: &OsStr, create: bool) -> Result<Option<Dir>> {
    let mut info = metadata(parent, name)?;
    if info.is_none() && create {
        if let Err(error) = platform::create_dir(parent, name)
            && error.kind() != io::ErrorKind::AlreadyExists
        {
            return Err(io_error(error));
        }
        info = metadata(parent, name)?;
    }
    match info {
        None => Ok(None),
        Some(info) if kind(&info) == Node::Directory => {
            parent.open_dir_nofollow(name).map(Some).map_err(io_error)
        }
        Some(_) => Err(invalid("资源祖先不是目录或是符号链接")),
    }
}

fn external_parent(root: &Path, create: bool) -> Result<(Option<Dir>, OsString)> {
    let Some(name) = root.file_name() else {
        return Dir::open_ambient_dir(root, ambient_authority())
            .map(|dir| (Some(dir), ".".into()))
            .map_err(io_error);
    };
    let mut ancestor = root
        .parent()
        .ok_or_else(|| invalid("资源根目录缺少父目录"))?;
    let mut pending = Vec::new();
    let canonical = loop {
        match fs::canonicalize(ancestor) {
            Ok(path) => break path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                pending.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| invalid("资源根目录无法解析"))?
                        .to_owned(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| invalid("资源根目录无法解析"))?;
            }
            Err(error) => return Err(io_error(error)),
        }
    };
    // Links are resolved only outside the caller's resource root.
    let mut dir = Dir::open_ambient_dir(&canonical, ambient_authority()).map_err(io_error)?;
    for child in pending.iter().rev() {
        let Some(next) = directory(&dir, child, create)? else {
            return Ok((None, name.to_owned()));
        };
        dir = next;
    }
    Ok((Some(dir), name.to_owned()))
}

fn locate(root: &Path, path: &str, create: bool) -> Result<(Option<Dir>, OsString)> {
    let (mut dir, root_name) = external_parent(root, create)?;
    if path.is_empty() {
        return Ok((dir, root_name));
    }
    let mut parts: Vec<_> = path.split('/').map(OsStr::new).collect();
    let name = parts.pop().expect("nonempty resource path").to_owned();
    for part in std::iter::once(root_name.as_os_str()).chain(parts) {
        let Some(parent) = dir else {
            return Ok((None, name));
        };
        dir = directory(&parent, part, create)?;
    }
    Ok((dir, name))
}

fn children(dir: &Dir) -> Result<Vec<(String, Node)>> {
    let mut entries = Vec::new();
    for entry in dir.entries().map_err(io_error)? {
        let name = entry.map_err(io_error)?.file_name();
        let name = name
            .into_string()
            .map_err(|_| invalid("资源名称不是 UTF-8"))?;
        if matches!(name.as_str(), "." | "..") {
            continue;
        }
        if entries.len() >= MAX_NODES {
            return Err(invalid("资源目录不能超过 4096 个节点"));
        }
        let info = metadata(dir, OsStr::new(&name))?
            .ok_or_else(|| Error::new(ErrorCode::Conflict, "资源目录在读取期间发生变化"))?;
        entries.push((name, kind(&info)));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

fn digest(node: Node, mode: u32) -> Sha256 {
    let mut digest = Sha256::new();
    digest.update(format!("{}\0{:o}\0", node_name(node), mode & 0o7777));
    digest
}

fn file_bytes(parent: &Dir, name: &OsStr) -> Result<(Metadata, Vec<u8>)> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    platform::nonblocking(&mut options);
    let file = parent.open_with(name, &options).map_err(io_error)?;
    let info = file.metadata().map_err(io_error)?;
    if !info.is_file() {
        return Err(invalid("资源不是普通文件"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("资源文件不能超过 16 MiB"));
    }
    Ok((info, bytes))
}

fn inspect_at(
    parent: Option<&Dir>,
    name: &OsStr,
    path: &str,
) -> Result<(NodeInfo, Option<Vec<u8>>)> {
    let mut node = NodeInfo {
        path: path.into(),
        node: Node::Other,
        revision: None,
        state: EntryState::Missing,
        access: Access::Writable,
        executable: false,
        size: 0,
    };
    let Some(parent) = parent else {
        return Ok((node, None));
    };
    let Some(info) = metadata(parent, name)? else {
        if !platform::can_write(parent, None)? {
            node.access = Access::ReadOnly {
                reason: "资源父目录不可写".into(),
            };
        }
        return Ok((node, None));
    };
    node.node = kind(&info);
    node.state = EntryState::Present;
    node.executable = platform::executable(platform::mode(&info));
    node.size = info.len();
    let reason = match node.node {
        Node::Symlink => Some("符号链接可见，但不能通过资源管理修改"),
        Node::Other => Some("资源不是普通文件或目录"),
        Node::File if node.size > MAX_BYTES as u64 => Some("资源文件不能超过 16 MiB"),
        _ => None,
    };
    if let Some(reason) = reason {
        node.access = Access::ReadOnly {
            reason: reason.into(),
        };
        if node.node != Node::Symlink {
            node.state = EntryState::Invalid {
                reason: reason.into(),
            };
        }
        return Ok((node, None));
    }
    let (hash, bytes, mode) = if node.node == Node::File {
        let (info, bytes) = file_bytes(parent, name)?;
        let mode = platform::mode(&info);
        let mut hash = digest(Node::File, mode);
        hash.update(&bytes);
        node.size = bytes.len() as u64;
        node.executable = platform::executable(mode);
        (hash, Some(bytes), mode)
    } else {
        let directory = parent.open_dir_nofollow(name).map_err(io_error)?;
        let mode = platform::mode(&directory.dir_metadata().map_err(io_error)?);
        let mut hash = digest(Node::Directory, mode);
        for (child, kind) in children(&directory)? {
            hash.update(child.as_bytes());
            hash.update([0]);
            hash.update(node_name(kind));
            hash.update([0]);
        }
        (hash, None, mode)
    };
    node.revision = Some(hex(&hash.finalize()));
    if mode & 0o222 == 0 || !platform::can_write(parent, Some(name))? {
        node.access = Access::ReadOnly {
            reason: "资源或父目录不可写".into(),
        };
    }
    Ok((node, bytes))
}

fn check(node: &NodeInfo, expected: &ExpectedRevision) -> Result<()> {
    let matches = match expected {
        ExpectedRevision::Missing => matches!(node.state, EntryState::Missing),
        ExpectedRevision::Match(revision) => node.revision.as_ref() == Some(revision),
    };
    if matches {
        Ok(())
    } else {
        Err(Error::new(ErrorCode::Conflict, "资源已存在或版本已改变"))
    }
}

fn editable(node: &NodeInfo) -> Result<()> {
    match &node.access {
        Access::Writable => Ok(()),
        Access::ReadOnly { reason } => Err(invalid(reason)),
    }
}

pub(super) fn inspect(root: &Path, path: &str) -> Result<NodeInfo> {
    let (parent, name) = locate(root, path, false)?;
    inspect_at(parent.as_ref(), &name, path).map(|(node, _)| node)
}

pub(super) fn read(root: &Path, path: &str) -> Result<(NodeInfo, Vec<u8>)> {
    let (parent, name) = locate(root, path, false)?;
    let (node, bytes) = inspect_at(parent.as_ref(), &name, path)?;
    if matches!(node.state, EntryState::Missing) {
        return Err(Error::new(ErrorCode::Missing, "资源不存在"));
    }
    bytes
        .map(|bytes| (node, bytes))
        .ok_or_else(|| invalid("资源不是可读取的普通文件"))
}

fn collect(
    parent: &Dir,
    name: &OsStr,
    path: &str,
    recursive: bool,
    output: &mut Vec<NodeInfo>,
) -> Result<()> {
    let Some(info) = metadata(parent, name)? else {
        return Ok(());
    };
    if kind(&info) != Node::Directory {
        return Err(invalid("资源目录不是目录或是符号链接"));
    }
    let dir = parent.open_dir_nofollow(name).map_err(io_error)?;
    for (child, kind) in children(&dir)? {
        if output.len() >= MAX_NODES {
            return Err(invalid("资源目录不能超过 4096 个节点"));
        }
        let child_path = if path.is_empty() {
            child.clone()
        } else {
            format!("{path}/{child}")
        };
        if child_path.split('/').count() > MAX_DEPTH {
            return Err(invalid("资源目录不能超过 64 层"));
        }
        output.push(inspect_at(Some(&dir), OsStr::new(&child), &child_path)?.0);
        if recursive && kind == Node::Directory {
            collect(&dir, OsStr::new(&child), &child_path, true, output)?;
        }
    }
    Ok(())
}

pub(super) fn list(root: &Path, path: &str, recursive: bool) -> Result<Vec<NodeInfo>> {
    let (parent, name) = locate(root, path, false)?;
    let mut nodes = Vec::new();
    if let Some(parent) = parent {
        collect(&parent, &name, path, recursive, &mut nodes)?;
    }
    Ok(nodes)
}

pub(super) fn write(
    root: &Path,
    path: &str,
    expected: &ExpectedRevision,
    bytes: &[u8],
    executable: Option<bool>,
) -> Result<NodeInfo> {
    let (parent, name) = locate(root, path, true)?;
    let parent = parent.ok_or_else(|| Error::new(ErrorCode::Missing, "资源父目录不存在"))?;
    let (node, _) = inspect_at(Some(&parent), &name, path)?;
    check(&node, expected)?;
    editable(&node)?;
    if !matches!(node.state, EntryState::Missing) && node.node != Node::File {
        return Err(invalid("只能写入普通文件"));
    }
    let current = metadata(&parent, &name)?.map(|info| platform::mode(&info));
    let mut random = [0; 12];
    getrandom::fill(&mut random)
        .map_err(|_| Error::new(ErrorCode::Unavailable, "无法生成临时文件名"))?;
    let temporary = OsString::from(format!(".velune-resource-{}", hex(&random)));
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    platform::private(&mut options);
    let file = parent.open_with(&temporary, &options).map_err(io_error)?;
    let outcome = (|| {
        {
            // Closed before it is published: Windows does not rename or link
            // a file that is still open here.
            let mut file = file.into_std();
            file.write_all(bytes).map_err(io_error)?;
            platform::set_executable(&file, current, executable).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
        }
        let mode = platform::mode(
            &metadata(&parent, &temporary)?
                .ok_or_else(|| Error::new(ErrorCode::Conflict, "临时资源文件已消失"))?,
        );
        // This optimistic check does not exclude concurrent external edits.
        check(&inspect_at(Some(&parent), &name, path)?.0, expected)?;
        // A new file is linked, which fails if the name appeared meanwhile.
        let published = if matches!(expected, ExpectedRevision::Missing) {
            parent.hard_link(&temporary, &parent, &name)
        } else {
            parent.rename(&temporary, &parent, &name)
        };
        if let Err(error) = published {
            return Err(if error.kind() == io::ErrorKind::AlreadyExists {
                Error::new(ErrorCode::Conflict, "资源已存在")
            } else {
                io_error(error)
            });
        }
        platform::sync_dir(&parent).map_err(io_error)?;
        let mut hash = digest(Node::File, mode);
        hash.update(bytes);
        Ok(NodeInfo {
            path: path.into(),
            node: Node::File,
            revision: Some(hex(&hash.finalize())),
            state: EntryState::Present,
            access: Access::Writable,
            executable: platform::executable(mode),
            size: bytes.len() as u64,
        })
    })();
    let _ = parent.remove_file(&temporary);
    outcome
}

pub(super) fn delete(root: &Path, path: &str, expected: &ExpectedRevision) -> Result<()> {
    let (parent, name) = locate(root, path, false)?;
    let (node, _) = inspect_at(parent.as_ref(), &name, path)?;
    check(&node, expected)?;
    editable(&node)?;
    if matches!(node.state, EntryState::Missing) {
        return Err(Error::new(ErrorCode::Missing, "资源不存在"));
    }
    let parent = parent.ok_or_else(|| Error::new(ErrorCode::Missing, "资源父目录不存在"))?;
    match node.node {
        Node::File => parent.remove_file(&name),
        Node::Directory => parent.remove_dir(&name),
        _ => return Err(invalid("只能删除文件或空目录")),
    }
    .map_err(io_error)?;
    platform::sync_dir(&parent).map_err(io_error)
}

#[cfg(unix)]
mod platform {
    use cap_std::fs::{
        Dir, DirBuilder, DirBuilderExt, Metadata, MetadataExt, OpenOptions, OpenOptionsExt,
    };
    use std::{
        ffi::{CString, OsStr},
        io,
        os::{
            fd::{AsFd, AsRawFd},
            unix::{ffi::OsStrExt, fs::PermissionsExt},
        },
    };

    use super::{Error, ErrorCode, Result, io_error};

    pub fn mode(info: &Metadata) -> u32 {
        info.mode()
    }

    pub fn executable(mode: u32) -> bool {
        mode & 0o111 != 0
    }

    pub fn is_link_refused(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::ELOOP)
    }

    pub fn create_dir(parent: &Dir, name: &OsStr) -> io::Result<()> {
        parent.create_dir_with(name, DirBuilder::new().mode(0o700))
    }

    /// A FIFO swapped in for a file must not block the read.
    pub fn nonblocking(options: &mut OpenOptions) {
        options.custom_flags(libc::O_NONBLOCK);
    }

    pub fn private(options: &mut OpenOptions) {
        options.mode(0o600);
    }

    /// The new file keeps the replaced one's permissions, or is private; an
    /// explicit flag sets or clears the owner's execute bit.
    pub fn set_executable(
        file: &std::fs::File,
        current: Option<u32>,
        executable: Option<bool>,
    ) -> io::Result<()> {
        let mut mode = current.map_or(0o600, |mode| mode & 0o7777);
        match executable {
            Some(true) => mode |= 0o100,
            Some(false) => mode &= !0o111,
            None => {}
        }
        file.set_permissions(std::fs::Permissions::from_mode(mode))
    }

    /// Asked as the effective user, as the open itself would be.
    pub fn can_write(parent: &Dir, name: Option<&OsStr>) -> Result<bool> {
        let check = |name: &OsStr, mode| -> Result<bool> {
            let name =
                CString::new(name.as_bytes()).map_err(|_| super::invalid("资源路径包含 NUL"))?;
            let status = unsafe {
                libc::faccessat(
                    parent.as_fd().as_raw_fd(),
                    name.as_ptr(),
                    mode,
                    libc::AT_EACCESS | libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if status == 0 {
                return Ok(true);
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EACCES | libc::EROFS) => Ok(false),
                Some(libc::EINVAL | libc::ENOSYS | libc::ENOTSUP) => Err(Error::new(
                    ErrorCode::Unsupported,
                    "目标不能核查资源的实际写权限",
                )),
                _ => Err(io_error(error)),
            }
        };
        Ok(check(OsStr::new("."), libc::W_OK | libc::X_OK)?
            && match name {
                Some(name) => check(name, libc::W_OK)?,
                None => true,
            })
    }

    pub fn sync_dir(dir: &Dir) -> io::Result<()> {
        dir.try_clone()?.into_std_file().sync_all()
    }
}

#[cfg(windows)]
mod platform {
    use cap_std::fs::{Dir, Metadata, OpenOptions};
    use std::{ffi::OsStr, io};

    use super::{Result, metadata};

    /// Windows has no permission bits; the read-only attribute stands in for
    /// the write bits, so a revision still changes when it is set.
    pub fn mode(info: &Metadata) -> u32 {
        match (info.is_dir(), info.permissions().readonly()) {
            (true, _) => 0o755,
            (false, true) => 0o444,
            (false, false) => 0o644,
        }
    }

    /// Windows decides what runs by file type, not by a bit on the file.
    pub fn executable(_: u32) -> bool {
        false
    }

    pub fn is_link_refused(_: &io::Error) -> bool {
        false
    }

    pub fn create_dir(parent: &Dir, name: &OsStr) -> io::Result<()> {
        parent.create_dir(name)
    }

    pub fn nonblocking(_: &mut OpenOptions) {}

    pub fn private(_: &mut OpenOptions) {}

    pub fn set_executable(_: &std::fs::File, _: Option<u32>, _: Option<bool>) -> io::Result<()> {
        Ok(())
    }

    /// Access lists decide the real answer, and asking them means opening the
    /// file for writing, which fails while a harness has it open. The
    /// read-only attribute is the visible part; a denied write still fails
    /// with its own error.
    pub fn can_write(parent: &Dir, name: Option<&OsStr>) -> Result<bool> {
        Ok(match name {
            Some(name) => metadata(parent, name)?.is_none_or(|info| !info.permissions().readonly()),
            None => true,
        })
    }

    /// NTFS journals directory changes; a directory handle cannot be flushed
    /// without write access to it.
    pub fn sync_dir(_: &Dir) -> io::Result<()> {
        Ok(())
    }
}

/// The file semantics both platforms must share, run on each of them: these
/// keep a native config from being overwritten unseen or reached through a
/// link, and no type states them.
#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            let mut random = [0; 8];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!("velune-fs-{}", hex(&random)));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_only_over_the_revision_it_saw() {
        let scratch = Scratch::new();
        let root = scratch.0.join("home");
        let path = "skills/a/SKILL.md";
        let first = write(&root, path, &ExpectedRevision::Missing, b"one", None).unwrap();
        let again = write(&root, path, &ExpectedRevision::Missing, b"two", None);
        assert_eq!(again.unwrap_err().code(), ErrorCode::Conflict);

        let (read_back, bytes) = read(&root, path).unwrap();
        assert_eq!(bytes, b"one");
        assert_eq!(read_back.revision, first.revision);

        let seen = ExpectedRevision::Match(first.revision.clone().unwrap());
        let second = write(&root, path, &seen, b"two", None).unwrap();
        let stale = write(&root, path, &seen, b"three", None);
        assert_eq!(stale.unwrap_err().code(), ErrorCode::Conflict);
        assert_ne!(first.revision, second.revision);
        assert_eq!(read(&root, path).unwrap().1, b"two");
        // No temporary file is left beside the published one.
        let names: Vec<_> = list(&root, "skills/a", false)
            .unwrap()
            .into_iter()
            .map(|node| node.path)
            .collect();
        assert_eq!(names, [path]);
    }

    #[test]
    fn reading_a_missing_root_creates_nothing() {
        let scratch = Scratch::new();
        let root = scratch.0.join("absent").join("home");
        let node = inspect(&root, "CLAUDE.md").unwrap();
        assert!(matches!(node.state, EntryState::Missing));
        assert!(list(&root, "skills", true).unwrap().is_empty());
        assert!(!scratch.0.join("absent").exists());
    }

    #[test]
    fn lists_recursively_and_deletes_files_then_empty_directories() {
        let scratch = Scratch::new();
        let root = scratch.0.join("home");
        write(&root, "agents/a.md", &ExpectedRevision::Missing, b"a", None).unwrap();
        write(
            &root,
            "agents/nested/b.md",
            &ExpectedRevision::Missing,
            b"b",
            None,
        )
        .unwrap();
        let shown: Vec<_> = list(&root, "agents", true)
            .unwrap()
            .into_iter()
            .map(|node| (node.path, node.node))
            .collect();
        assert_eq!(
            shown,
            [
                ("agents/a.md".to_owned(), Node::File),
                ("agents/nested".to_owned(), Node::Directory),
                ("agents/nested/b.md".to_owned(), Node::File),
            ]
        );
        let seen = |path| ExpectedRevision::Match(inspect(&root, path).unwrap().revision.unwrap());
        let refused = delete(&root, "agents/nested", &seen("agents/nested"));
        assert_eq!(refused.unwrap_err().code(), ErrorCode::Invalid);
        delete(&root, "agents/nested/b.md", &seen("agents/nested/b.md")).unwrap();
        delete(&root, "agents/nested", &seen("agents/nested")).unwrap();
        assert!(!root.join("agents").join("nested").exists());
    }

    #[cfg(unix)]
    fn link_directory(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    /// A junction, as an unprivileged user can make one.
    #[cfg(windows)]
    fn link_directory(target: &Path, link: &Path) {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn does_not_go_through_a_link_inside_the_root() {
        let scratch = Scratch::new();
        let root = scratch.0.join("home");
        let outside = scratch.0.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        link_directory(&outside, &root.join("skills"));
        let written = write(
            &root,
            "skills/a/SKILL.md",
            &ExpectedRevision::Missing,
            b"x",
            None,
        );
        assert_eq!(written.unwrap_err().code(), ErrorCode::Invalid);
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        let link = inspect(&root, "skills").unwrap();
        assert_eq!(link.node, Node::Symlink);
        assert!(matches!(link.access, Access::ReadOnly { .. }));
    }
}
