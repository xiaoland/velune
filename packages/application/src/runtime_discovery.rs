//! Explicit, non-secret runtime discovery. Detection never imports source credentials.
use crate::{Error, RuntimeDiscoveryHint};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) fn hints(
    user_home: &str,
    overrides: BTreeMap<String, String>,
) -> Result<Vec<RuntimeDiscoveryHint>, Error> {
    if !Path::new(user_home).is_absolute() || user_home.contains('\0') {
        return Err(Error::Invalid("用户目录必须为绝对路径".into()));
    }
    let home = Path::new(user_home);
    let directory = |variable: &str, default: &[&str]| -> Result<PathBuf, Error> {
        let path = match overrides.get(variable).filter(|v| !v.is_empty()) {
            Some(value) => PathBuf::from(value),
            None => default
                .iter()
                .fold(home.to_path_buf(), |path, part| path.join(part)),
        };
        if !path.is_absolute() || path.to_string_lossy().contains('\0') {
            return Err(Error::Invalid("运行时目录覆盖必须为绝对路径".into()));
        }
        Ok(path)
    };
    let local = directory("LOCALAPPDATA", &["AppData", "Local"])?;
    let roaming = directory("APPDATA", &["AppData", "Roaming"])?;
    [
        ("pi", "pi", "PI_CODING_AGENT_DIR", ".pi/agent"),
        ("codex", "codex", "CODEX_HOME", ".codex"),
        ("claude-code", "claude", "CLAUDE_CONFIG_DIR", ".claude"),
        ("deepseek-harness", "dsh", "DSH_HOME", ".dsh"),
    ]
    .into_iter()
    .map(|(family, command, variable, relative)| {
        let agent_directory = directory(variable, &[relative])?;
        let bundled = Bundled {
            home,
            local: &local,
            roaming: &roaming,
            agent_directory: &agent_directory,
        };
        Ok(RuntimeDiscoveryHint {
            family_id: family.into(),
            command: command.into(),
            directory_exists: agent_directory.is_dir(),
            bundled_binaries: bundled.binaries(family),
            agent_directory: agent_directory.to_string_lossy().into_owned(),
        })
    })
    .collect()
}

/// Where vendors' own installers, desktop apps and editor extensions put a
/// runtime's command, outside PATH. Only existing files are reported; the
/// probe that follows reads their versions.
struct Bundled<'a> {
    home: &'a Path,
    local: &'a Path,
    roaming: &'a Path,
    agent_directory: &'a Path,
}

/// Editors that install VS Code extensions under the user's home.
const EDITOR_EXTENSIONS: &[&str] = &[".vscode", ".vscode-insiders", ".cursor"];

impl Bundled<'_> {
    fn binaries(&self, family: &str) -> Vec<String> {
        let executable = |name: &str| {
            if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            }
        };
        // The extensions' per-platform directories.
        let platform = match (
            cfg!(windows),
            cfg!(target_os = "macos"),
            cfg!(target_arch = "aarch64"),
        ) {
            (true, _, false) => Some("windows-x86_64"),
            (true, _, true) => Some("windows-aarch64"),
            (_, true, true) => Some("macos-aarch64"),
            (_, true, false) => Some("macos-x86_64"),
            _ => None,
        };
        let mut found = Vec::new();
        match family {
            "codex" => {
                let codex = executable("codex");
                // OpenAI's standalone installer keeps each release under the Codex home.
                found.extend(each(
                    &self.agent_directory.join("packages/standalone/releases"),
                    "",
                    &["bin", &codex],
                ));
                if let Some(platform) = platform {
                    for editor in EDITOR_EXTENSIONS {
                        found.extend(each(
                            &self.home.join(editor).join("extensions"),
                            "openai.chatgpt-",
                            &["bin", platform, &codex],
                        ));
                    }
                }
                if cfg!(windows) {
                    // The Codex desktop app copies its command out of its package.
                    let bin = self.local.join("OpenAI").join("Codex").join("bin");
                    found.extend(each(&bin, "", &[&codex]));
                    found.extend(file(bin.join(&codex)));
                }
                if cfg!(target_os = "macos") {
                    // The ChatGPT desktop app carries a Codex CLI.
                    for applications in
                        [Path::new("/Applications"), &self.home.join("Applications")]
                    {
                        found.extend(file(
                            applications.join("ChatGPT.app/Contents/Resources/codex-cli/bin/codex"),
                        ));
                    }
                }
            }
            "claude-code" => {
                let claude = executable("claude");
                for editor in EDITOR_EXTENSIONS {
                    found.extend(each(
                        &self.home.join(editor).join("extensions"),
                        "anthropic.claude-code-",
                        &["resources", "native-binary", &claude],
                    ));
                }
                if cfg!(windows) {
                    // The Claude desktop app keeps a Claude Code per version.
                    found.extend(each(
                        &self.roaming.join("Claude").join("claude-code"),
                        "",
                        &[&claude],
                    ));
                }
            }
            _ => {}
        }
        found.dedup();
        found
    }
}

/// `parent/<prefix>*/<suffix>` for each such directory that is not hidden,
/// newest name first.
fn each(parent: &Path, prefix: &str, suffix: &[&str]) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with(prefix) && !name.starts_with('.'))
        .collect();
    names.sort_by(|a, b| b.cmp(a));
    names
        .into_iter()
        .filter_map(|name| {
            file(
                suffix
                    .iter()
                    .fold(parent.join(name), |path, part| path.join(part)),
            )
        })
        .collect()
}

fn file(path: PathBuf) -> Option<String> {
    path.is_file().then(|| path.to_string_lossy().into_owned())
}
