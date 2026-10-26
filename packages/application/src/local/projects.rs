//! Projects of the conversation list: the folders each holds, the Git
//! worktrees that belong to their repository's project, and the user's
//! shaping of a project. Velune projects use their primary folder as identity;
//! Codex projects keep a runtime-scoped native identity and canonical membership.
use super::*;
use crate::config::{ConversationBrowserProject, ConversationGroupKey, ProjectRecord};
use std::{collections::BTreeSet, path::Component};

pub(super) struct NativeProjects {
    runtime: RuntimeInstance,
    projects: Vec<velune_agent_runtime::native::codex_projects::NativeProject>,
    active_assignments: BTreeMap<String, String>,
    archived_assignments: BTreeMap<String, String>,
}

fn native_key(runtime: &str, project: &str) -> String {
    format!(
        "codex-project:{}",
        serde_json::to_string(&(runtime, project)).expect("string tuple")
    )
}

impl CoreRuntime {
    pub(super) fn read_codex_projects(
        &self,
        runtime: &RuntimeInstance,
        archived: bool,
    ) -> Result<Vec<ConversationSummary>, RuntimeError> {
        let listing = velune_agent_runtime::native::codex_projects::list_projects(
            &self.history_config(runtime)?,
            &runtime.id,
            archived,
        )
        .map_err(|error| RuntimeError::context("Codex 项目读取失败", error))?;
        let mut catalogs = self.native_projects.borrow_mut();
        let catalog = catalogs
            .entry(runtime.id.clone())
            .or_insert_with(|| NativeProjects {
                runtime: runtime.clone(),
                projects: Vec::new(),
                active_assignments: BTreeMap::new(),
                archived_assignments: BTreeMap::new(),
            });
        if catalog.runtime != *runtime {
            catalog.active_assignments.clear();
            catalog.archived_assignments.clear();
            catalog.runtime = runtime.clone();
        }
        catalog.projects = listing.projects;
        if archived {
            for summary in &listing.conversations {
                catalog.active_assignments.remove(&summary.id);
            }
            catalog.archived_assignments = listing.assignments;
        } else {
            for summary in &listing.conversations {
                catalog.archived_assignments.remove(&summary.id);
            }
            catalog.active_assignments = listing.assignments;
        }
        Ok(listing.conversations)
    }

    pub(super) fn native_project(
        &self,
        key: &str,
    ) -> Option<(
        String,
        velune_agent_runtime::native::codex_projects::NativeProject,
    )> {
        let catalogs = self.native_projects.borrow();
        self.runtime_instances
            .iter()
            .filter(|runtime| runtime.enabled)
            .find_map(|runtime| {
                let catalog = catalogs
                    .get(&runtime.id)
                    .filter(|catalog| catalog.runtime == *runtime)?;
                catalog
                    .projects
                    .iter()
                    .find(|project| native_key(&runtime.id, &project.id) == key)
                    .map(|project| (runtime.id.clone(), project.clone()))
            })
    }

    pub(super) fn native_project_keys(&self) -> Vec<String> {
        let catalogs = self.native_projects.borrow();
        self.runtime_instances
            .iter()
            .filter(|runtime| runtime.enabled)
            .flat_map(|runtime| {
                catalogs
                    .get(&runtime.id)
                    .filter(|catalog| catalog.runtime == *runtime)
                    .into_iter()
                    .flat_map(|catalog| {
                        catalog
                            .projects
                            .iter()
                            .map(|project| native_key(&runtime.id, &project.id))
                    })
            })
            .collect()
    }

    pub(super) fn native_conversation_project(
        &self,
        runtime: &str,
        conversation: &str,
    ) -> Option<String> {
        let catalogs = self.native_projects.borrow();
        let instance = self
            .runtime_instances
            .iter()
            .find(|instance| instance.id == runtime && instance.enabled)?;
        let catalog = catalogs
            .get(runtime)
            .filter(|catalog| catalog.runtime == *instance)?;
        let id = catalog
            .active_assignments
            .get(conversation)
            .or_else(|| catalog.archived_assignments.get(conversation))?;
        catalog
            .projects
            .iter()
            .any(|p| p.id == *id)
            .then(|| native_key(runtime, id))
    }

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
        if let Some((runtime_id, project)) = payload["path"]
            .as_str()
            .and_then(|key| self.native_project(key))
        {
            let name = payload["name"]
                .as_str()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| RuntimeError::invalid("Codex 项目名称不能为空"))?;
            let primary = payload["primaryFolder"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("项目文件夹"))?;
            let mut roots: Vec<String> = serde_json::from_value(payload["folders"].clone())?;
            if !primary.is_empty() {
                roots.insert(0, primary.to_owned());
            }
            let mut seen = BTreeSet::new();
            if roots
                .iter()
                .any(|root| root.is_empty() || !seen.insert(root.clone()))
            {
                return Err(RuntimeError::invalid("项目文件夹不能为空或重复"));
            }
            let runtime = self.runtime_instance(&runtime_id)?;
            // Absolute paths are validated by Codex on the executing OS, including SSH hosts.
            let updated = velune_agent_runtime::native::codex_projects::update_project(
                &self.history_config(runtime)?,
                &project.id,
                name,
                &roots,
            )
            .map_err(|error| RuntimeError::context("Codex 项目保存失败", error))?;
            if let Some(catalog) = self.native_projects.borrow_mut().get_mut(&runtime_id)
                && let Some(stored) = catalog
                    .projects
                    .iter_mut()
                    .find(|stored| stored.id == updated.id)
            {
                *stored = updated;
            }
            return self.list();
        }
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
