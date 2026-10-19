//! Finds installed commands the way `where` would, without running any launcher.
//!
//! npm and pnpm install three launchers per command: `x.cmd`, `x.ps1` and an
//! extensionless `sh` script. Pi's installation resolver reads only the `sh`
//! script's literal target, so Pi uses that one; Codex and Claude Code start
//! their binary directly, so they use a native `.exe`, else the `.cmd` launcher.
use std::{
    env,
    path::{Path, PathBuf},
};

fn search_path() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = env::var_os("PATH")
        .map(|path| env::split_paths(&path).collect())
        .unwrap_or_default();
    // Package managers' global directories, for an app started before PATH was updated.
    for (variable, relative) in [
        ("LOCALAPPDATA", "pnpm"),
        ("APPDATA", "npm"),
        ("LOCALAPPDATA", "Programs\\OpenAI\\Codex\\bin"),
    ] {
        if let Some(base) = env::var_os(variable) {
            directories.push(Path::new(&base).join(relative));
        }
    }
    let mut unique = Vec::new();
    for directory in directories {
        if directory.is_absolute() && !unique.contains(&directory) {
            unique.push(directory);
        }
    }
    unique
}

fn is_sh_launcher(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| bytes.starts_with(b"#!/bin/sh"))
}

/// Every installation of `command` a runtime family can be probed with, in PATH order.
pub fn candidates(family: &str, command: &str) -> Vec<String> {
    let mut found = Vec::new();
    for directory in search_path() {
        let executable = directory.join(format!("{command}.exe"));
        let launcher = directory.join(command);
        let script = directory.join(format!("{command}.cmd"));
        let chosen = if family == "pi" {
            [launcher]
                .into_iter()
                .find(|path| path.is_file() && is_sh_launcher(path))
        } else {
            [executable, script].into_iter().find(|path| path.is_file())
        };
        if let Some(path) = chosen {
            let path = path.to_string_lossy().into_owned();
            if !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// The first `node.exe` on PATH.
pub fn node() -> Option<String> {
    search_path()
        .into_iter()
        .map(|directory| directory.join("node.exe"))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn user_home() -> Option<String> {
    env::var("USERPROFILE")
        .ok()
        .or_else(|| env::var("HOME").ok())
}
