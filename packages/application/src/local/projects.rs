//! Projects of the conversation list: the folders each holds, the Git
//! worktrees that belong to their repository's project, and the user's
//! shaping of a project. A project is identified by its primary folder; a
//! working directory no project holds is a project of its own.
use super::*;
use crate::config::{ConversationBrowserProject, ConversationGroupKey, ProjectRecord};
use std::{collections::BTreeSet, path::Component};

impl CoreRuntime {
    /// The project, by its primary folder, of conversations started in
    /// `folder`: the project holding it, else its worktree's repository's,
    /// else its own. `on_this_mac` lets a worktree be told by its files.
    pub(super) fn folder_project(&self, folder: &str, on_this_mac: bool) -> String {
        let holding = |folder: &str| {
            self.organization
                .projects
                .iter()
                .find(|project| project.all_folders().any(|held| held == folder))
                .map(|project| project.primary_folder.clone())
        };
        holding(folder).unwrap_or_else(|| match self.worktree_origin(folder, on_this_mac) {
            Some(repository) => holding(&repository).unwrap_or(repository),
            None => folder.to_owned(),
        })
    }

    pub(super) fn project_record(&self, primary_folder: &str) -> Option<&ProjectRecord> {
        self.organization
            .projects
            .iter()
            .find(|project| project.primary_folder == primary_folder)
    }

    /// The repository folder a Git worktree belongs to: by Claude Code's
    /// `.claude/worktrees/<name>` layout, which also holds over SSH and once
    /// the worktree is removed; else, on this Mac, by the worktree's `.git`
    /// file. Files are read once per folder.
    fn worktree_origin(&self, folder: &str, on_this_mac: bool) -> Option<String> {
        if let Some((repository, name)) = folder.split_once("/.claude/worktrees/")
            && !repository.is_empty()
            && !name.is_empty()
        {
            return Some(repository.to_owned());
        }
        if !on_this_mac {
            return None;
        }
        if let Some(known) = self.worktree_origins.borrow().get(folder) {
            return known.clone();
        }
        let found = linked_worktree_repository(Path::new(folder));
        self.worktree_origins
            .borrow_mut()
            .insert(folder.to_owned(), found.clone());
        found
    }

    pub(super) fn set_project_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let payload = &request["payload"];
        let absolute = |value: &Value| {
            value
                .as_str()
                .filter(|folder| Path::new(folder).is_absolute())
                .map(str::to_owned)
                .ok_or_else(|| RuntimeError::invalid("项目文件夹必须是绝对路径"))
        };
        let path = absolute(&payload["path"])?;
        let primary = absolute(&payload["primaryFolder"])?;
        let name = payload["name"]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        let mut folders = Vec::<String>::new();
        for folder in payload["folders"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("项目文件夹列表"))?
        {
            let folder = absolute(folder)?;
            if folder != primary && !folders.contains(&folder) {
                folders.push(folder);
            }
        }
        let mut next = self.organization.clone();
        next.projects
            .retain(|project| project.primary_folder != path);
        let wanted: Vec<&String> = std::iter::once(&primary).chain(&folders).collect();
        if let Some(other) = next.projects.iter().find(|project| {
            project
                .all_folders()
                .any(|held| wanted.iter().any(|f| *f == held))
        }) {
            return Err(RuntimeError::Invalid(format!(
                "文件夹已属于项目「{}」",
                project_title(other)
            )));
        }
        // The project as it was and each folder taken in, which may have
        // been projects of their own, are this project now.
        let joined: BTreeSet<String> = std::iter::once(path.clone())
            .chain(folders.iter().cloned())
            .filter(|folder| *folder != primary)
            .collect();
        for marks in &mut next.conversations {
            if marks
                .project
                .as_ref()
                .is_some_and(|project| joined.contains(project))
            {
                marks.project = Some(primary.clone());
            }
        }
        if name.is_some() || !folders.is_empty() {
            next.projects.push(ProjectRecord {
                primary_folder: primary.clone(),
                name,
                folders,
            });
        }
        self.store_organization(next)?;
        self.forget_redundant_moves()?;
        let mut preferences = self.conversation_browser_preferences.clone();
        let follow = |folder: &mut String| {
            if joined.contains(folder) {
                *folder = primary.clone();
            }
        };
        if let ConversationBrowserProject::Path { path } = &mut preferences.project {
            follow(path);
        }
        for key in preferences
            .collapsed
            .iter_mut()
            .chain(preferences.group_order.iter_mut())
        {
            if let ConversationGroupKey::Project { path: Some(path) } = key {
                follow(path);
            }
        }
        let mut seen = BTreeSet::new();
        preferences.collapsed.retain(|key| seen.insert(key.clone()));
        // A joined project keeps the place of the first of its folders.
        let mut seen = BTreeSet::new();
        preferences
            .group_order
            .retain(|key| seen.insert(key.clone()));
        if preferences != self.conversation_browser_preferences {
            let previous =
                std::mem::replace(&mut self.conversation_browser_preferences, preferences);
            if let Err(error) = self.persist() {
                self.conversation_browser_preferences = previous;
                return Err(error);
            }
            self.reset_browser_pages();
        }
        self.list()
    }

    /// A conversation moved into the project its working directory now
    /// belongs to is no longer moved.
    fn forget_redundant_moves(&mut self) -> Result<(), RuntimeError> {
        let places = self.projectless_places();
        let redundant: Vec<String> = {
            let listed = self.last_listed.borrow();
            self.organization
                .conversations
                .iter()
                .filter(|marks| {
                    marks.project.is_some()
                        && listed.get(&marks.conversation_id).is_some_and(|source| {
                            self.working_project(&source.summary, &places) == marks.project
                        })
                })
                .map(|marks| marks.conversation_id.clone())
                .collect()
        };
        if redundant.is_empty() {
            return Ok(());
        }
        let mut next = self.organization.clone();
        for marks in &mut next.conversations {
            if redundant.contains(&marks.conversation_id) {
                marks.project = None;
            }
        }
        next.conversations.retain(|marks| !marks.is_empty());
        self.store_organization(next)
    }
}

/// The project's name, else its primary folder's.
fn project_title(project: &ProjectRecord) -> String {
    project.name.clone().unwrap_or_else(|| {
        Path::new(&project.primary_folder).file_name().map_or_else(
            || project.primary_folder.clone(),
            |name| name.to_string_lossy().into_owned(),
        )
    })
}

/// The folder of the repository whose linked worktree `folder` is: its
/// `.git` file names the worktree's Git directory, inside the repository's
/// `worktrees`, and that directory's `commondir` names the repository's.
/// A bare repository has no folder of its own and gives none.
fn linked_worktree_repository(folder: &Path) -> Option<String> {
    let pointer = std::fs::read_to_string(folder.join(".git")).ok()?;
    let git_dir = lexical(&folder.join(pointer.strip_prefix("gitdir:")?.trim()));
    let common = std::fs::read_to_string(git_dir.join("commondir")).ok()?;
    let common = lexical(&git_dir.join(common.trim()));
    if git_dir.parent()? != common.join("worktrees") || common.file_name()? != ".git" {
        return None;
    }
    common.parent()?.to_str().map(str::to_owned)
}

/// `path` with `.` and `..` resolved by name, as Git writes them, so the
/// repository keeps the spelling its own conversations record.
fn lexical(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}
