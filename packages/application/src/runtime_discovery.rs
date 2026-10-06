//! Explicit, non-secret runtime discovery. Detection never imports source credentials.
use crate::{Error, RuntimeDiscoveryHint};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn hints(
    user_home: &str,
    overrides: BTreeMap<String, String>,
) -> Result<Vec<RuntimeDiscoveryHint>, Error> {
    if !Path::new(user_home).is_absolute() || user_home.contains('\0') {
        return Err(Error::Invalid("用户目录必须为绝对路径".into()));
    }
    [
        ("pi", "pi", "PI_CODING_AGENT_DIR", ".pi/agent"),
        ("codex", "codex", "CODEX_HOME", ".codex"),
        ("deepseek-harness", "dsh", "DSH_HOME", ".dsh"),
    ]
    .into_iter()
    .map(|(family, command, variable, relative)| {
        let directory = overrides
            .get(variable)
            .filter(|v| !v.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                Path::new(user_home)
                    .join(relative)
                    .to_string_lossy()
                    .into_owned()
            });
        if !Path::new(&directory).is_absolute() || directory.contains('\0') {
            return Err(Error::Invalid("运行时目录覆盖必须为绝对路径".into()));
        }
        Ok(RuntimeDiscoveryHint {
            family_id: family.into(),
            command: command.into(),
            directory_exists: Path::new(&directory).is_dir(),
            agent_directory: directory,
        })
    })
    .collect()
}
