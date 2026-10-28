//! Attachments enter a turn here: images through the harness's own image
//! input, other files as absolute path references in the message text.
use super::*;
use crate::RuntimeTransport;

#[derive(Default)]
pub(super) struct TurnAttachments {
    pub images: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

impl CoreRuntime {
    /// Checks a turn's attachments for `runtime` before anything changes, so
    /// a turn whose attachment cannot be delivered is refused, not trimmed.
    pub(super) fn turn_attachments(
        &self,
        runtime: &RuntimeInstance,
        paths: &[String],
    ) -> Result<TurnAttachments, RuntimeError> {
        let mut attachments = TurnAttachments::default();
        if paths.is_empty() {
            return Ok(attachments);
        }
        if !matches!(runtime.execution()?.transport, RuntimeTransport::Local) {
            return Err(RuntimeError::invalid("远端运行时暂不支持附件"));
        }
        for path in paths.iter().map(PathBuf::from) {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            if !path.is_absolute() {
                return Err(RuntimeError::Invalid(format!("附件 {name} 必须是绝对路径")));
            }
            let metadata = std::fs::metadata(&path)
                .map_err(|error| RuntimeError::Invalid(format!("无法读取附件 {name}：{error}")))?;
            if !metadata.is_file() {
                return Err(RuntimeError::Invalid(format!("附件 {name} 不是文件")));
            }
            match pi::ImageType::from_path(&path) {
                Some(kind) => {
                    pi::TurnImage::read(&path, kind).map_err(RuntimeError::Invalid)?;
                    attachments.images.push(path);
                }
                None => {
                    std::fs::File::open(&path).map_err(|error| {
                        RuntimeError::Invalid(format!("无法读取附件 {name}：{error}"))
                    })?;
                    attachments.files.push(path);
                }
            }
        }
        let variant = velune_agent_runtime::version::variant(runtime.type_id);
        if !attachments.images.is_empty() && !variant.accepts_images {
            return Err(RuntimeError::Invalid(format!(
                "{} 不接受图片",
                variant.name
            )));
        }
        Ok(attachments)
    }
}

/// The message text with non-image files listed by absolute path.
pub(super) fn compose_text(text: &str, files: &[PathBuf]) -> String {
    let paths = files
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>();
    velune_conversation::attachment_list::compose(text, paths.iter().map(|path| path.as_ref()))
}

/// Images a turn carries, read for delivery.
pub(super) fn turn_images(paths: &[PathBuf]) -> Result<Vec<pi::TurnImage>, RuntimeError> {
    paths
        .iter()
        .map(|path| {
            let kind = pi::ImageType::from_path(path)
                .ok_or_else(|| RuntimeError::invalid("附件不是支持的图片类型"))?;
            pi::TurnImage::read(path, kind).map_err(RuntimeError::Invalid)
        })
        .collect()
}
