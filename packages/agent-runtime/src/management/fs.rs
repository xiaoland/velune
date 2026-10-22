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
    let mut command = target.python();
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

mod local;
