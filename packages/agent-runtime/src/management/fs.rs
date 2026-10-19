use super::{Access, EntryState, Error, ErrorCode, ExpectedRevision, Node, Result};
use crate::execution::ExecutionTarget;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::path::Path;

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const REMOTE: &str = include_str!("../../resources/manage_resources.py");

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct NodeInfo {
    pub path: String,
    pub node: Node,
    pub revision: Option<String>,
    pub state: EntryState,
    pub access: Access,
    pub executable: bool,
    pub size: u64,
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::Invalid, message)
}

fn validate(root: &Path, path: &str) -> Result<()> {
    if !root.is_absolute() {
        return Err(invalid("资源根目录必须是绝对路径"));
    }
    if path.contains('\0')
        || Path::new(path).is_absolute()
        || (!path.is_empty() && path.split('/').any(|part| matches!(part, "" | "." | "..")))
    {
        return Err(invalid("资源路径必须是不含空段、. 或 .. 的相对路径"));
    }
    if path.split('/').count() > MAX_DEPTH {
        return Err(invalid("资源路径不能超过 64 层"));
    }
    Ok(())
}

fn remote<T: DeserializeOwned>(
    target: &ExecutionTarget,
    root: &Path,
    path: &str,
    mut request: Value,
) -> Result<T> {
    let root = root
        .to_str()
        .filter(|root| !root.contains('\0'))
        .ok_or_else(|| invalid("远程资源根目录必须是有效 UTF-8 路径"))?;
    request["root"] = json!(root);
    request["path"] = json!(path);
    let mut command = crate::process_tree::command("python3");
    command.args(["-c", REMOTE]);
    let bytes = serde_json::to_vec(&request).map_err(|_| invalid("资源请求无法编码"))?;
    let result = target
        .output(command, Some(bytes), None)
        .map_err(|error| Error::new(ErrorCode::Unavailable, error.to_string()))?;
    let mut response: Value = serde_json::from_slice(&result)
        .map_err(|_| Error::new(ErrorCode::Unavailable, "远程资源操作返回无效结果"))?;
    if let Some(error) = response.get("error") {
        let code = serde_json::from_value(error["code"].clone()).unwrap_or(ErrorCode::Unavailable);
        return Err(Error::new(
            code,
            error["message"].as_str().unwrap_or("远程资源操作失败"),
        ));
    }
    serde_json::from_value(response["ok"].take())
        .map_err(|_| Error::new(ErrorCode::Unavailable, "远程资源操作返回无效数据"))
}

pub(super) fn inspect(target: &ExecutionTarget, root: &Path, path: &str) -> Result<NodeInfo> {
    validate(root, path)?;
    if target.is_local() {
        local::inspect(root, path)
    } else {
        remote(target, root, path, json!({"operation":"inspect"}))
    }
}

pub(super) fn list(
    target: &ExecutionTarget,
    root: &Path,
    path: &str,
    recursive: bool,
) -> Result<Vec<NodeInfo>> {
    validate(root, path)?;
    if target.is_local() {
        local::list(root, path, recursive)
    } else {
        remote(
            target,
            root,
            path,
            json!({"operation":"list", "recursive":recursive}),
        )
    }
}

pub(super) fn read(
    target: &ExecutionTarget,
    root: &Path,
    path: &str,
) -> Result<(NodeInfo, Vec<u8>)> {
    validate(root, path)?;
    if target.is_local() {
        local::read(root, path)
    } else {
        #[derive(Deserialize)]
        struct Read {
            node: NodeInfo,
            hex: String,
        }
        let value: Read = remote(target, root, path, json!({"operation":"read"}))?;
        let bytes = decode_hex(&value.hex)?;
        Ok((value.node, bytes))
    }
}

pub(super) fn write(
    target: &ExecutionTarget,
    root: &Path,
    path: &str,
    expected: &ExpectedRevision,
    bytes: &[u8],
    executable: Option<bool>,
) -> Result<NodeInfo> {
    validate(root, path)?;
    if path.is_empty() {
        return Err(invalid("不能替换资源根目录"));
    }
    if bytes.len() > MAX_BYTES {
        return Err(invalid("资源文件不能超过 16 MiB"));
    }
    if target.is_local() {
        local::write(root, path, expected, bytes, executable)
    } else {
        remote(
            target,
            root,
            path,
            json!({"operation":"write", "expected":expected, "hex":hex(bytes), "executable":executable}),
        )
    }
}

pub(super) fn delete(
    target: &ExecutionTarget,
    root: &Path,
    path: &str,
    expected: &ExpectedRevision,
) -> Result<()> {
    validate(root, path)?;
    if path.is_empty() {
        return Err(invalid("不能删除资源根目录"));
    }
    if target.is_local() {
        local::delete(root, path, expected)
    } else {
        remote(
            target,
            root,
            path,
            json!({"operation":"delete", "expected":expected}),
        )
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[(byte >> 4) as usize]));
        result.push(char::from(HEX[(byte & 15) as usize]));
    }
    result
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) || value.len() > MAX_BYTES * 2 {
        return Err(invalid("远程资源内容大小或编码无效"));
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
            digit(pair[0])
                .zip(digit(pair[1]))
                .map(|(high, low)| high * 16 + low)
                .ok_or_else(|| invalid("远程资源内容编码无效"))
        })
        .collect()
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd"
))]
mod local {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        ffi::{CStr, CString, OsStr, OsString},
        fs::{self, File, Metadata, Permissions},
        io::{self, Read, Write},
        mem::MaybeUninit,
        os::{
            fd::{AsRawFd, FromRawFd, IntoRawFd},
            unix::{
                ffi::OsStrExt,
                fs::{MetadataExt, PermissionsExt},
            },
        },
    };

    const DIRECTORY: i32 = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const MAX_NODES: usize = 4096;

    fn io_error(error: io::Error) -> Error {
        let code = match error.raw_os_error() {
            Some(libc::ENOENT) => ErrorCode::Missing,
            Some(libc::ELOOP | libc::ENOTDIR | libc::ENOTEMPTY | libc::EISDIR) => {
                ErrorCode::Invalid
            }
            _ => ErrorCode::Unavailable,
        };
        Error::new(code, format!("资源文件操作失败：{error}"))
    }

    fn cstring(value: &OsStr) -> Result<CString> {
        CString::new(value.as_bytes()).map_err(|_| invalid("资源路径包含 NUL"))
    }

    fn open(parent: i32, name: &OsStr, flags: i32, mode: u32) -> Result<File> {
        let name = cstring(name)?;
        // The supplied directory descriptor anchors traversal; O_NOFOLLOW rejects its next link.
        let descriptor =
            unsafe { libc::openat(parent, name.as_ptr(), flags, mode as libc::c_uint) };
        if descriptor < 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }

    fn metadata(parent: &File, name: &OsStr) -> Result<Option<libc::stat>> {
        let name = cstring(name)?;
        let mut value = MaybeUninit::uninit();
        let status = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                value.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if status == 0 {
            Ok(Some(unsafe { value.assume_init() }))
        } else {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(io_error(error))
            }
        }
    }

    fn kind(mode: u32) -> Node {
        match mode & libc::S_IFMT as u32 {
            value if value == libc::S_IFREG as u32 => Node::File,
            value if value == libc::S_IFDIR as u32 => Node::Directory,
            value if value == libc::S_IFLNK as u32 => Node::Symlink,
            _ => Node::Other,
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

    fn directory(parent: &File, name: &OsStr, create: bool) -> Result<Option<File>> {
        let mut info = metadata(parent, name)?;
        if info.is_none() && create {
            let name_c = cstring(name)?;
            let status = unsafe { libc::mkdirat(parent.as_raw_fd(), name_c.as_ptr(), 0o700) };
            if status < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(io_error(error));
                }
            }
            info = metadata(parent, name)?;
        }
        match info {
            None => Ok(None),
            Some(info) if kind(info.st_mode as u32) == Node::Directory => {
                open(parent.as_raw_fd(), name, DIRECTORY, 0).map(Some)
            }
            Some(_) => Err(invalid("资源祖先不是目录或是符号链接")),
        }
    }

    fn external_parent(root: &Path, create: bool) -> Result<(Option<File>, OsString)> {
        let Some(name) = root.file_name() else {
            return open(libc::AT_FDCWD, root.as_os_str(), DIRECTORY, 0)
                .map(|file| (Some(file), ".".into()));
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
        // Symlink resolution is permitted only outside the caller's resource root.
        let mut descriptor = open(libc::AT_FDCWD, canonical.as_os_str(), DIRECTORY, 0)?;
        for child in pending.iter().rev() {
            let Some(next) = directory(&descriptor, child, create)? else {
                return Ok((None, name.to_owned()));
            };
            descriptor = next;
        }
        Ok((Some(descriptor), name.to_owned()))
    }

    fn locate(root: &Path, path: &str, create: bool) -> Result<(Option<File>, OsString)> {
        let (mut descriptor, root_name) = external_parent(root, create)?;
        if path.is_empty() {
            return Ok((descriptor, root_name));
        }
        let mut parts: Vec<_> = path.split('/').map(OsStr::new).collect();
        let name = parts.pop().expect("nonempty resource path").to_owned();
        for part in std::iter::once(root_name.as_os_str()).chain(parts) {
            let Some(parent) = descriptor else {
                return Ok((None, name));
            };
            descriptor = directory(&parent, part, create)?;
        }
        Ok((descriptor, name))
    }

    struct DirectoryStream(*mut libc::DIR);
    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn errno() -> *mut libc::c_int {
        unsafe { libc::__errno_location() }
    }
    #[cfg(target_os = "android")]
    fn errno() -> *mut libc::c_int {
        unsafe { libc::__errno() }
    }
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    fn errno() -> *mut libc::c_int {
        unsafe { libc::__error() }
    }

    fn children(parent: &File) -> Result<Vec<(String, Node)>> {
        let descriptor = open(parent.as_raw_fd(), OsStr::new("."), DIRECTORY, 0)?.into_raw_fd();
        let pointer = unsafe { libc::fdopendir(descriptor) };
        if pointer.is_null() {
            let error = io::Error::last_os_error();
            unsafe {
                libc::close(descriptor);
            }
            return Err(io_error(error));
        }
        let stream = DirectoryStream(pointer);
        let mut entries = Vec::new();
        loop {
            unsafe {
                *errno() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(io_error(error));
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            let name = name.to_str().map_err(|_| invalid("资源名称不是 UTF-8"))?;
            if matches!(name, "." | "..") {
                continue;
            }
            if entries.len() >= MAX_NODES {
                return Err(invalid("资源目录不能超过 4096 个节点"));
            }
            let info = metadata(parent, OsStr::new(name))?
                .ok_or_else(|| Error::new(ErrorCode::Conflict, "资源目录在读取期间发生变化"))?;
            entries.push((name.to_owned(), kind(info.st_mode as u32)));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    fn digest(node: Node, mode: u32) -> Sha256 {
        let mut digest = Sha256::new();
        digest.update(format!("{}\0{:o}\0", node_name(node), mode & 0o7777));
        digest
    }

    fn file_bytes(parent: &File, name: &OsStr) -> Result<(Metadata, Vec<u8>)> {
        let file = open(
            parent.as_raw_fd(),
            name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
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
        parent: Option<&File>,
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
            if !can_write(parent, None)? {
                node.access = Access::ReadOnly {
                    reason: "资源父目录不可写".into(),
                };
            }
            return Ok((node, None));
        };
        let mut mode = info.st_mode as u32;
        node.node = kind(mode);
        node.state = EntryState::Present;
        node.executable = mode & 0o111 != 0;
        node.size = info.st_size.max(0) as u64;
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
        let (hash, bytes) = if node.node == Node::File {
            let (info, bytes) = file_bytes(parent, name)?;
            mode = info.mode();
            let mut hash = digest(Node::File, mode);
            hash.update(&bytes);
            node.size = bytes.len() as u64;
            node.executable = info.mode() & 0o111 != 0;
            (hash, Some(bytes))
        } else {
            let directory = open(parent.as_raw_fd(), name, DIRECTORY, 0)?;
            let mut hash = digest(
                Node::Directory,
                directory.metadata().map_err(io_error)?.mode(),
            );
            for (child, kind) in children(&directory)? {
                hash.update(child.as_bytes());
                hash.update([0]);
                hash.update(node_name(kind));
                hash.update([0]);
            }
            (hash, None)
        };
        node.revision = Some(hex(&hash.finalize()));
        if mode & 0o222 == 0 || !can_write(parent, Some(name))? {
            node.access = Access::ReadOnly {
                reason: "资源或父目录不可写".into(),
            };
        }
        Ok((node, bytes))
    }

    fn can_write(parent: &File, name: Option<&OsStr>) -> Result<bool> {
        let check = |name: &OsStr, mode| -> Result<bool> {
            let name = cstring(name)?;
            let status = unsafe {
                libc::faccessat(
                    parent.as_raw_fd(),
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
        parent: &File,
        name: &OsStr,
        path: &str,
        recursive: bool,
        output: &mut Vec<NodeInfo>,
    ) -> Result<()> {
        let Some(info) = metadata(parent, name)? else {
            return Ok(());
        };
        if kind(info.st_mode as u32) != Node::Directory {
            return Err(invalid("资源目录不是目录或是符号链接"));
        }
        let descriptor = open(parent.as_raw_fd(), name, DIRECTORY, 0)?;
        for (child, kind) in children(&descriptor)? {
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
            output.push(inspect_at(Some(&descriptor), OsStr::new(&child), &child_path)?.0);
            if recursive && kind == Node::Directory {
                collect(&descriptor, OsStr::new(&child), &child_path, true, output)?;
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
        let mut mode = metadata(&parent, &name)?.map_or(0o600, |info| info.st_mode as u32 & 0o7777);
        if let Some(executable) = executable {
            mode = if executable {
                mode | 0o100
            } else {
                mode & !0o111
            };
        }
        let mut random = [0; 12];
        getrandom::fill(&mut random)
            .map_err(|_| Error::new(ErrorCode::Unavailable, "无法生成临时文件名"))?;
        let temporary = OsString::from(format!(".velune-resource-{}", hex(&random)));
        let mut file = open(
            parent.as_raw_fd(),
            &temporary,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )?;
        let outcome = (|| {
            file.write_all(bytes).map_err(io_error)?;
            file.set_permissions(Permissions::from_mode(mode))
                .map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
            mode = file.metadata().map_err(io_error)?.mode() & 0o7777;
            // This optimistic check does not exclude concurrent external edits.
            check(&inspect_at(Some(&parent), &name, path)?.0, expected)?;
            let temporary = cstring(&temporary)?;
            let name = cstring(&name)?;
            let status = unsafe {
                if matches!(expected, ExpectedRevision::Missing) {
                    libc::linkat(
                        parent.as_raw_fd(),
                        temporary.as_ptr(),
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        0,
                    )
                } else {
                    libc::renameat(
                        parent.as_raw_fd(),
                        temporary.as_ptr(),
                        parent.as_raw_fd(),
                        name.as_ptr(),
                    )
                }
            };
            if status < 0 {
                let error = io::Error::last_os_error();
                return Err(if error.kind() == io::ErrorKind::AlreadyExists {
                    Error::new(ErrorCode::Conflict, "资源已存在")
                } else {
                    io_error(error)
                });
            }
            parent.sync_all().map_err(io_error)?;
            let mut hash = digest(Node::File, mode);
            hash.update(bytes);
            Ok(NodeInfo {
                path: path.into(),
                node: Node::File,
                revision: Some(hex(&hash.finalize())),
                state: EntryState::Present,
                access: Access::Writable,
                executable: mode & 0o111 != 0,
                size: bytes.len() as u64,
            })
        })();
        let temporary = cstring(&temporary)?;
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0);
        }
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
        let flags = match node.node {
            Node::File => 0,
            Node::Directory => libc::AT_REMOVEDIR,
            _ => return Err(invalid("只能删除文件或空目录")),
        };
        let parent = parent.ok_or_else(|| Error::new(ErrorCode::Missing, "资源父目录不存在"))?;
        let name = cstring(&name)?;
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } < 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        parent.sync_all().map_err(io_error)
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd"
)))]
mod local {
    use super::*;
    fn unsupported<T>() -> Result<T> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "当前平台不支持受限资源文件与 Unix 权限管理",
        ))
    }
    pub(super) fn inspect(_: &Path, _: &str) -> Result<NodeInfo> {
        unsupported()
    }
    pub(super) fn list(_: &Path, _: &str, _: bool) -> Result<Vec<NodeInfo>> {
        unsupported()
    }
    pub(super) fn read(_: &Path, _: &str) -> Result<(NodeInfo, Vec<u8>)> {
        unsupported()
    }
    pub(super) fn write(
        _: &Path,
        _: &str,
        _: &ExpectedRevision,
        _: &[u8],
        _: Option<bool>,
    ) -> Result<NodeInfo> {
        unsupported()
    }
    pub(super) fn delete(_: &Path, _: &str, _: &ExpectedRevision) -> Result<()> {
        unsupported()
    }
}
