//! Images attached to a turn, read and checked once where they enter.
use base64::Engine;
use std::{
    fmt,
    path::{Path, PathBuf},
};

/// An attached image larger than this is refused rather than piped to a harness.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageType {
    /// The image type a file name claims; other files are attached by path.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }

    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    fn matches(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            Self::Jpeg => bytes.starts_with(&[0xff, 0xd8, 0xff]),
            Self::Gif => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
            Self::Webp => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        }
    }
}

/// A local image whose content matches its type.
#[derive(Clone)]
pub struct TurnImage {
    pub path: PathBuf,
    pub kind: ImageType,
    data: Vec<u8>,
}

impl TurnImage {
    pub fn read(path: &Path, kind: ImageType) -> Result<Self, String> {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let size = std::fs::metadata(path)
            .map_err(|error| format!("无法读取图片 {name}：{error}"))?
            .len();
        if size > MAX_IMAGE_BYTES {
            return Err(format!("图片 {name} 超过 20 MB"));
        }
        let data = std::fs::read(path).map_err(|error| format!("无法读取图片 {name}：{error}"))?;
        if !kind.matches(&data) {
            return Err(format!("{name} 的内容与其图片类型不符"));
        }
        Ok(Self {
            path: path.into(),
            kind,
            data,
        })
    }

    pub fn base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.data)
    }
}

impl fmt::Debug for TurnImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TurnImage")
            .field("path", &self.path)
            .field("kind", &self.kind)
            .field("bytes", &self.data.len())
            .finish()
    }
}
