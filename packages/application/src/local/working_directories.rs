//! New-conversation directory suggestions use the complete project catalog,
//! never the sidebar's loaded conversations. No filesystem scan is needed.
use super::*;
use crate::{ConversationProject, ListedConversation, RuntimeTransport, WorkingDirectoryProject};
use std::collections::{BTreeSet, HashMap};

impl CoreRuntime {
    pub(super) fn working_directory_projects(
        &self,
        conversations: &[ListedConversation],
        projects: &[ConversationProject],
    ) -> HashMap<String, Vec<WorkingDirectoryProject>> {
        let locations: BTreeMap<_, _> = self
            .runtime_instances
            .iter()
            .filter(|runtime| runtime.enabled)
            .filter_map(|runtime| {
                runtime
                    .execution()
                    .ok()
                    .map(|execution| (runtime.id.as_str(), execution.transport))
            })
            .collect();
        let mut origins = BTreeMap::<&str, BTreeSet<&str>>::new();
        for conversation in conversations {
            if let Some(project) = &conversation.project {
                origins
                    .entry(project)
                    .or_default()
                    .insert(&conversation.conversation.runtime_id);
            }
        }
        locations
            .iter()
            .map(|(runtime, location)| {
                let choices = projects
                    .iter()
                    .filter(|project| {
                        if let Some(owner) = &project.native_runtime_id {
                            return locations.get(owner.as_str()) == Some(location);
                        }
                        if let Some(owners) = origins.get(project.path.as_str()) {
                            return owners
                                .iter()
                                .any(|owner| locations.get(owner) == Some(location));
                        }
                        // Folder-only records have no remote owner. A configured
                        // empty project belongs to the execution device itself.
                        matches!(location, RuntimeTransport::Local)
                            && self.project_record(&project.path).is_some()
                    })
                    .filter_map(|project| {
                        let mut seen = BTreeSet::new();
                        let directories: Vec<_> = project
                            .primary_folder
                            .iter()
                            .chain(&project.folders)
                            .filter(|path| seen.insert((*path).clone()))
                            .cloned()
                            .collect();
                        if directories.is_empty() {
                            return None;
                        }
                        let name = project.name.clone().unwrap_or_else(|| {
                            Path::new(&directories[0]).file_name().map_or_else(
                                || directories[0].clone(),
                                |name| name.to_string_lossy().into_owned(),
                            )
                        });
                        Some(WorkingDirectoryProject {
                            project: project.path.clone(),
                            name,
                            can_select_project: project.native_runtime_id.as_deref()
                                == Some(*runtime)
                                && self.runtime_instances.iter().any(|instance| {
                                    instance.id.as_str() == *runtime
                                        && instance.type_id == RuntimeKind::Codex166
                                }),
                            directories,
                        })
                    })
                    .collect();
                ((*runtime).to_owned(), choices)
            })
            .collect()
    }
}
