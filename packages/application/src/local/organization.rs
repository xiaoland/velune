//! Velune's organization of the conversation list: the window of groups and
//! pages the list loads, and the user's pins, archive, read state, moves, and
//! project names. None of it is written to the runtimes' sessions, except an
//! archive the runtime keeps itself.
use super::*;
use crate::config::{
    ConversationBrowserGrouping, ConversationBrowserProject, ConversationBrowserSort,
    ConversationGroupKey, ConversationGroupSort, ConversationMarks,
};
use crate::{
    ConversationGroup, ConversationOrganizationChange, ConversationProject, ConversationReference,
    ConversationSearch, ListedConversation,
};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

/// What the list loads beyond its first pages; not persisted.
#[derive(Default, Clone)]
pub(super) struct BrowserWindow {
    /// Pages beyond the first that the user loaded, per group.
    extra_pages: BTreeMap<ConversationGroupKey, u32>,
    /// Initial capacity belongs to this client window, not the current group
    /// order: reading native dates can reorder groups without evicting rows.
    initial_loads: std::cell::RefCell<BTreeMap<ConversationGroupKey, usize>>,
    archived: bool,
}

/// A listed conversation and whether its runtime archived it.
#[derive(Clone)]
pub(super) struct ListedSource {
    pub(super) summary: browsing::SharedSummary,
    pub(super) archived_natively: bool,
    /// Indexed but not read yet: identity, project, and file time only.
    pub(super) provisional: bool,
}

/// A client keeps the provenance of the list it last saw when it parks its view.
#[derive(Clone, Default)]
pub(super) struct ListedProjection {
    pub(super) entries: BTreeMap<String, ListedSource>,
    pub(super) sources: Vec<runtime_source::RuntimeSourceKey>,
}

/// Where a runtime starts conversations that have no project: HOME, where
/// they land given no working directory, and the folders the runtime keeps
/// such conversations in. The filesystem root, where the app's own land,
/// counts for every runtime.
#[derive(Default)]
pub(super) struct ProjectlessPlaces {
    home: Option<PathBuf>,
    directories: Vec<PathBuf>,
    /// Its working directories are on this Mac, where their files can be read.
    on_this_mac: bool,
}

impl ProjectlessPlaces {
    fn contain(&self, cwd: &Path) -> bool {
        cwd.parent().is_none()
            || self.home.as_deref() == Some(cwd)
            || self
                .directories
                .iter()
                .any(|directory| cwd.starts_with(directory))
    }
}

pub(super) struct Page {
    pub(super) history_failures: Vec<crate::HistoryFailure>,
    pub(super) conversations: Vec<ListedConversation>,
    pub(super) groups: Vec<ConversationGroup>,
    pub(super) projects: Vec<ConversationProject>,
    pub(super) working_directory_projects: HashMap<String, Vec<crate::WorkingDirectoryProject>>,
}

pub(crate) fn unix_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

impl CoreRuntime {
    pub(super) fn listed_sources(&self) -> std::cell::Ref<'_, BTreeMap<String, ListedSource>> {
        let stale = self
            .last_listed
            .borrow()
            .sources
            .iter()
            .any(|source| !source.is_current(self));
        if stale {
            *self.last_listed.borrow_mut() = ListedProjection::default();
        }
        std::cell::Ref::map(self.last_listed.borrow(), |projection| &projection.entries)
    }

    pub(crate) fn shows_archived(&self) -> bool {
        self.browser_window.archived
    }

    /// The conversations every runtime archived itself, for the archive view.
    pub(super) fn natively_archived(
        &self,
    ) -> (
        BTreeMap<String, browsing::HistoryArchive>,
        Vec<crate::HistoryFailure>,
    ) {
        let mut archived = BTreeMap::new();
        let mut failures = Vec::new();
        for runtime in self.runtime_instances.iter().filter(|r| r.enabled) {
            if !velune_agent_runtime::version::variant(runtime.type_id).can_archive_conversations {
                continue;
            }
            let source = self.runtime_source_key(runtime);
            match source.and_then(|source| {
                self.history_config(runtime)
                    .and_then(|config| {
                        if config.provider == "codex" {
                            self.read_codex_projects(runtime, true)
                        } else {
                            Ok(browsing::shared_summaries(history::list_archived(
                                &config,
                                &runtime.id,
                            )?))
                        }
                    })
                    .map(|summaries| (source, summaries))
            }) {
                Ok((source, summaries)) if source.is_current(self) => {
                    archived.insert(
                        runtime.id.clone(),
                        browsing::HistoryArchive { source, summaries },
                    );
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="archived_list",runtime_id=%runtime.id,runtime_type_id=%runtime.type_id,detail=%error);
                    failures.push(crate::HistoryFailure {
                        runtime_id: runtime.id.clone(),
                        detail: format!(
                            "{}；原生归档无法读取：{error}",
                            browsing::runtime_context(runtime)
                        ),
                    });
                }
            }
        }
        (archived, failures)
    }

    /// The places each enabled runtime starts conversations without a project.
    pub(super) fn projectless_places(&self) -> BTreeMap<String, ProjectlessPlaces> {
        self.runtime_instances
            .iter()
            .filter(|runtime| runtime.enabled)
            .map(|runtime| {
                let home = self.user_home(runtime);
                let variant = velune_agent_runtime::version::variant(runtime.type_id);
                let directories = home.as_ref().map_or_else(Vec::new, |home| {
                    variant
                        .projectless_directories
                        .iter()
                        .map(|directory| home.join(directory))
                        .chain(std::iter::once(home.join("Documents/Velune")))
                        .collect()
                });
                let on_this_mac = self
                    .runtime_location(runtime)
                    .is_ok_and(|(target, _)| target.is_local());
                (
                    runtime.id.clone(),
                    ProjectlessPlaces {
                        home,
                        directories,
                        on_this_mac,
                    },
                )
            })
            .collect()
    }

    /// The HOME a runtime's conversations see: the one it is given, else the
    /// app's on this Mac or the login's over SSH.
    pub(super) fn user_home(&self, runtime: &RuntimeInstance) -> Option<PathBuf> {
        let (target, os_home) = self.runtime_location(runtime).ok()?;
        if os_home.is_some() {
            return os_home;
        }
        if target.is_local() {
            return std::env::home_dir();
        }
        let key = serde_json::to_string(&runtime.execution().ok()?).ok()?;
        self.remote_homes
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let mut command = velune_agent_runtime::process::command("printenv");
                command.arg("HOME");
                match target.output(command, None, None) {
                    Ok(bytes) => String::from_utf8(bytes)
                        .ok()
                        .map(|home| PathBuf::from(home.trim_end_matches(['\r', '\n'])))
                        .filter(|home| home.is_absolute()),
                    Err(error) => {
                        tracing::warn!(target:"velune_application",event="runtime_home_unavailable",runtime_id=%runtime.id,detail=%error);
                        None
                    }
                }
            })
            .clone()
    }

    /// The project a conversation's working directory makes it part of.
    pub(super) fn working_project(
        &self,
        summary: &ConversationSummary,
        places: &BTreeMap<String, ProjectlessPlaces>,
    ) -> Option<String> {
        let (runtime, conversation) = self.native_segment(summary);
        if let Some(key) =
            self.native_conversation_project(&runtime, &format!("{runtime}:{conversation}"))
        {
            return Some(key);
        }
        let unknown = ProjectlessPlaces::default();
        let places = places.get(&summary.runtime_id).unwrap_or(&unknown);
        summary
            .cwd
            .as_deref()
            .filter(|cwd| !places.contain(Path::new(cwd)))
            .map(|cwd| self.folder_project(cwd, places.on_this_mac))
    }

    /// Where a conversation moved into the project holding `folder` is filed.
    fn moved_project(
        &self,
        summary: &ConversationSummary,
        folder: &str,
        places: &BTreeMap<String, ProjectlessPlaces>,
    ) -> String {
        let on_this_mac = places
            .get(&summary.runtime_id)
            .is_some_and(|places| places.on_this_mac);
        self.folder_project(folder, on_this_mac)
    }

    fn marks(&self, id: &str) -> Option<&ConversationMarks> {
        self.organization
            .conversations
            .iter()
            .find(|marks| marks.conversation_id == id)
    }

    pub(super) fn locally_archived_ids(&self) -> BTreeSet<&str> {
        self.organization
            .conversations
            .iter()
            .filter(|marks| marks.archived_at_unix_ms.is_some())
            .map(|marks| marks.conversation_id.as_str())
            .collect()
    }

    pub(super) fn link_segment_ids(&self, marked: Option<&BTreeSet<&str>>) -> BTreeSet<&str> {
        self.conversation_links
            .iter()
            .filter(|link| marked.is_none_or(|ids| ids.contains(link.id.as_str())))
            .flat_map(|link| &link.segments)
            .map(|segment| segment.native_conversation_id.as_str())
            .collect()
    }

    fn project_name(&self, path: &str) -> Option<String> {
        self.native_project(path)
            .map(|(_, project)| project.name.clone())
            .or_else(|| {
                self.project_record(path)
                    .and_then(|project| project.name.clone())
            })
    }

    /// The runtime and identity it resumes the conversation by.
    fn native_segment(&self, summary: &ConversationSummary) -> (String, String) {
        let segment = self
            .conversation_links
            .iter()
            .find(|link| link.id == summary.id)
            .and_then(|link| link.segments.last());
        let (runtime_id, id) = segment.map_or((&summary.runtime_id, &summary.id), |segment| {
            (
                &segment.runtime_instance_id,
                &segment.native_conversation_id,
            )
        });
        let native = id.strip_prefix(&format!("{runtime_id}:")).unwrap_or(id);
        (runtime_id.clone(), native.to_owned())
    }

    /// Whether the latest turn of each conversation failed, from the native
    /// records on this machine; a record is read again only after it changes.
    /// A conversation without a record found is left out.
    fn failed_turns<'a>(
        &self,
        conversations: impl Iterator<Item = &'a ConversationSummary>,
    ) -> HashMap<String, bool> {
        use velune_agent_runtime::activity::{self, SessionFormat, SessionStore};
        let mut stores =
            BTreeMap::<String, Option<(SessionStore, HashMap<String, PathBuf>)>>::new();
        let mut found = HashMap::new();
        for conversation in conversations {
            let (runtime_id, native_id) = self.native_segment(conversation);
            let Some((store, paths)) = stores.entry(runtime_id).or_insert_with_key(|runtime_id| {
                let store = self.session_store(self.runtime_instance(runtime_id).ok()?)?;
                // Pi names its sessions by path.
                let paths = if store.format == SessionFormat::Pi {
                    HashMap::new()
                } else {
                    store.paths()
                };
                Some((store, paths))
            }) else {
                continue;
            };
            let path = match store.format {
                SessionFormat::Pi => PathBuf::from(&native_id),
                _ => match paths.get(&native_id) {
                    Some(path) => path.clone(),
                    None => continue,
                },
            };
            let Some(signature) = activity::signature(&path) else {
                continue;
            };
            let mut known = self.turn_failures.borrow_mut();
            let failed = match known.get(&path) {
                Some((at, failed)) if *at == signature => *failed,
                _ => {
                    let failed = activity::last_turn_failed(store.format, &path);
                    known.insert(path, (signature, failed));
                    failed
                }
            };
            found.insert(conversation.id.clone(), failed);
        }
        found
    }

    pub(super) fn listed(
        &self,
        source: &ListedSource,
        places: &BTreeMap<String, ProjectlessPlaces>,
    ) -> ListedConversation {
        let summary = &source.summary;
        let marks = self.marks(&summary.id);
        let archived =
            source.archived_natively || marks.is_some_and(|m| m.archived_at_unix_ms.is_some());
        let seen = marks
            .and_then(|m| m.seen_activity_unix_ms)
            .map_or(self.organization.read_baseline_unix_ms, |seen| {
                seen.max(self.organization.read_baseline_unix_ms)
            });
        let moved = marks.and_then(|m| m.project.clone());
        ListedConversation {
            conversation: {
                let mut summary = summary.as_ref().clone();
                self.apply_generated_title(&mut summary);
                summary
            },
            native_id: self.native_segment(summary).1,
            agents_uri: self.agents_uri(summary),
            pinned: !archived && marks.is_some_and(|m| m.pinned_at_unix_ms.is_some()),
            unread: !archived
                && summary.native_read_state.as_ref().map_or_else(
                    || {
                        marks.is_some_and(|m| m.marked_unread)
                            || summary.updated_at_unix_ms.is_some_and(|at| at > seen)
                    },
                    |receipt| receipt.unread,
                ),
            archived,
            moved: moved.is_some(),
            failed: false,
            project: moved.or_else(|| self.working_project(summary, places)),
            group: None,
        }
    }

    /// The window over `sources` with every conversation it shows read.
    /// Reading gives a conversation its native activity time, which may move
    /// it out of the window and others in, so the window is drawn again
    /// until it settles; whatever is still unread then is left out.
    pub(super) fn browse_read(
        &self,
        mut sources: Vec<ListedSource>,
        search: Option<&str>,
        extend_index: bool,
    ) -> Page {
        let mut rounds = 0;
        loop {
            let page = self.browse_window(sources, search, true, extend_index);
            let unread = |item: &ListedConversation| {
                self.listed_sources()
                    .get(&item.conversation.id)
                    .is_some_and(|source| source.provisional)
            };
            let mut wanted = BTreeMap::<&str, Vec<String>>::new();
            for item in page.conversations.iter().filter(|item| unread(item)) {
                wanted
                    .entry(item.conversation.runtime_id.as_str())
                    .or_default()
                    .push(item.conversation.id.clone());
            }
            if wanted.is_empty() {
                return page;
            }
            rounds += 1;
            let mut failures = Vec::new();
            for (runtime_id, ids) in &wanted {
                let read = self
                    .runtime_instance(runtime_id)
                    .and_then(|runtime| self.read_indexed(runtime, ids));
                if let Err(error) = read {
                    failures.push(crate::HistoryFailure {
                        runtime_id: (*runtime_id).into(),
                        detail: format!("会话历史无法读取：{error}"),
                    });
                    tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="list_window",runtime_id=%runtime_id,detail=%error);
                }
            }
            if !failures.is_empty() || rounds > 4 {
                // Use the summaries that did succeed, while leaving failed reads
                // provisional so another explicit request can retry them.
                let mut page = self.browse_window(self.cached_sources(), search, false, true);
                page.history_failures = failures;
                return page;
            }
            sources = self.cached_sources();
        }
    }

    /// The list's window over `sources`: the pinned group, then the groups by
    /// recent activity, each loading its pages while the list's total lasts
    /// and any further page the user asked for. A search loads every match
    /// up to the total instead.
    fn browse_window(
        &self,
        sources: Vec<ListedSource>,
        search: Option<&str>,
        include_provisional: bool,
        known_first: bool,
    ) -> Page {
        let preferences = &self.conversation_browser_preferences;
        let archived_view = self.browser_window.archived;
        let places = self.projectless_places();
        let marked = archived_view.then(|| self.locally_archived_ids());
        let all: Vec<ListedConversation> = sources
            .iter()
            .filter(|source| {
                source.archived_natively
                    || marked
                        .as_ref()
                        .is_none_or(|ids| ids.contains(source.summary.id.as_str()))
            })
            .map(|source| self.listed(source, &places))
            .collect();
        *self.last_listed.borrow_mut() = ListedProjection {
            entries: sources
                .into_iter()
                .map(|source| (source.summary.id.clone(), source))
                .collect(),
            sources: self
                .runtime_instances
                .iter()
                .filter(|runtime| runtime.enabled)
                .filter_map(|runtime| self.runtime_source_key(runtime).ok())
                .collect(),
        };
        // Each project's latest activity and the worktrees its conversations
        // ran in: working directories it holds by their repository alone.
        let mut projects = BTreeMap::<String, (Option<i64>, BTreeSet<String>)>::new();
        for key in self.native_project_keys() {
            projects.entry(key).or_default();
        }
        for project in &self.organization.projects {
            projects.entry(project.primary_folder.clone()).or_default();
        }
        for item in all.iter().filter(|item| {
            item.conversation.parent_conversation_id.is_none() && item.archived == archived_view
        }) {
            if let Some(path) = &item.project {
                let (latest, worktrees) = projects.entry(path.clone()).or_default();
                *latest = (*latest).max(item.conversation.updated_at_unix_ms);
                if let Some(cwd) = item.conversation.cwd.as_ref().filter(|cwd| {
                    !item.moved
                        && self.native_project(path).is_none()
                        && *cwd != path
                        && !self
                            .project_record(path)
                            .is_some_and(|project| project.folders.contains(cwd))
                }) {
                    worktrees.insert(cwd.clone());
                }
            }
        }
        let needle = search.map(str::to_lowercase);
        let runtime_names: BTreeMap<&str, String> = self
            .runtime_instances
            .iter()
            .map(|r| (r.id.as_str(), r.name.to_lowercase()))
            .collect();
        let matches = |item: &ListedConversation| {
            let Some(needle) = &needle else { return true };
            let contains = |text: &str| text.to_lowercase().contains(needle.as_str());
            contains(item.conversation.title.display_text())
                || item.project.as_deref().is_some_and(|path| {
                    contains(path) || self.project_name(path).is_some_and(|name| contains(&name))
                })
                || item.conversation.cwd.as_deref().is_some_and(contains)
                || runtime_names
                    .get(item.conversation.runtime_id.as_str())
                    .is_some_and(|name| name.contains(needle.as_str()))
        };
        let shown = |item: &ListedConversation| {
            item.conversation.parent_conversation_id.is_none()
                && item.archived == archived_view
                && preferences
                    .runtime_id
                    .as_ref()
                    .is_none_or(|id| *id == item.conversation.runtime_id)
                && match &preferences.project {
                    ConversationBrowserProject::All => true,
                    ConversationBrowserProject::Unspecified => item.project.is_none(),
                    ConversationBrowserProject::Path { path } => {
                        item.project.as_ref() == Some(path)
                    }
                }
                && matches(item)
        };
        let mut members = BTreeMap::<ConversationGroupKey, Vec<usize>>::new();
        for (index, item) in all.iter().enumerate().filter(|(_, item)| shown(item)) {
            let key = if item.pinned {
                ConversationGroupKey::Pinned
            } else {
                match preferences.grouping {
                    ConversationBrowserGrouping::None => ConversationGroupKey::All,
                    ConversationBrowserGrouping::Runtime => ConversationGroupKey::Runtime {
                        runtime_id: item.conversation.runtime_id.clone(),
                    },
                    ConversationBrowserGrouping::Project => ConversationGroupKey::Project {
                        path: item.project.clone(),
                    },
                }
            };
            members.entry(key).or_default().push(index);
        }
        let timestamp = |item: &ListedConversation| match preferences.sort {
            ConversationBrowserSort::Updated => item.conversation.updated_at_unix_ms,
            ConversationBrowserSort::Created => item.conversation.created_at_unix_ms,
        };
        let order = |left: &usize, right: &usize| {
            let (l, r) = (&all[*left], &all[*right]);
            let by_time = match (timestamp(l), timestamp(r)) {
                // Unknown native dates stay last in either direction.
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (Some(a), Some(b)) if preferences.oldest_first => a.cmp(&b),
                (Some(a), Some(b)) => b.cmp(&a),
                (None, None) => Ordering::Equal,
            };
            by_time
                .then_with(|| {
                    l.conversation
                        .title
                        .display_text()
                        .cmp(r.conversation.title.display_text())
                })
                .then_with(|| l.conversation.id.cmp(&r.conversation.id))
        };
        let mut groups: Vec<(ConversationGroupKey, Vec<usize>, Option<i64>)> = members
            .into_iter()
            .map(|(key, mut indices)| {
                indices.sort_by(order);
                let latest = indices
                    .iter()
                    .filter_map(|i| all[*i].conversation.updated_at_unix_ms)
                    .max();
                (key, indices, latest)
            })
            .collect();
        // Pinned first and conversations without a project last; the rest by
        // their latest activity.
        let rank = |key: &ConversationGroupKey| match key {
            ConversationGroupKey::Pinned => 0,
            ConversationGroupKey::Project { path: None } => 2,
            _ => 1,
        };
        groups.sort_by(|(lk, _, ll), (rk, _, rl)| {
            rank(lk)
                .cmp(&rank(rk))
                .then_with(|| match (ll, rl) {
                    (Some(a), Some(b)) => b.cmp(a),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                })
                .then_with(|| lk.cmp(rk))
        });
        // In the user's order no activity moves a group: pinned stays first,
        // and groups new since follow the ordered ones by key.
        if preferences.group_sort == ConversationGroupSort::Manual {
            let position = |key: &ConversationGroupKey| {
                preferences
                    .group_order
                    .iter()
                    .position(|ordered| ordered == key)
            };
            groups.sort_by(|(lk, ..), (rk, ..)| {
                let place = |key: &ConversationGroupKey| match key {
                    ConversationGroupKey::Pinned => (0, 0),
                    key => position(key).map_or((2, 0), |at| (1, at)),
                };
                place(lk).cmp(&place(rk)).then_with(|| lk.cmp(rk))
            });
        }
        let page = self.conversation_browser_limits.page as usize;
        let mut remaining = self.conversation_browser_limits.total as usize;
        if needle.is_none() {
            let mut initial = self.browser_window.initial_loads.borrow_mut();
            let present: BTreeSet<_> = groups.iter().map(|(key, ..)| key).collect();
            initial.retain(|key, _| present.contains(key));
            remaining = remaining.saturating_sub(initial.values().sum());
        }
        let mut conversations = Vec::new();
        let mut descriptors = Vec::new();
        for (key, indices, latest) in groups {
            let collapsed = needle.is_none() && preferences.collapsed.contains(&key);
            let loaded = if collapsed {
                0
            } else if needle.is_some() {
                let loaded = indices.len().min(remaining);
                remaining -= loaded;
                loaded
            } else if key == ConversationGroupKey::Pinned {
                indices.len()
            } else {
                let extra = self
                    .browser_window
                    .extra_pages
                    .get(&key)
                    .copied()
                    .unwrap_or(0) as usize;
                let mut initial = self.browser_window.initial_loads.borrow_mut();
                let first = *initial.entry(key.clone()).or_insert_with(|| {
                    let first = if key == ConversationGroupKey::All {
                        remaining
                    } else {
                        page.min(remaining)
                    }
                    .min(indices.len());
                    remaining -= first;
                    first
                });
                indices.len().min(first + page * extra)
            };
            // Extending an indexed window keeps known rows available. Only a
            // native refresh lets file-date hints displace them; paging another
            // group must not chase cold candidates in an already loaded group.
            let known = |index: &&usize| {
                self.listed_sources()
                    .get(&all[**index].conversation.id)
                    .is_some_and(|source| !source.provisional)
            };
            let mut visible: Vec<_> = if known_first {
                indices.iter().filter(known).take(loaded).collect()
            } else {
                Vec::new()
            };
            if include_provisional {
                let remaining = loaded - visible.len();
                visible.extend(
                    indices
                        .iter()
                        .filter(|index| !known_first || !known(index))
                        .take(remaining),
                );
            }
            // Every selected row still uses native dates and the same tie-breaks.
            visible.sort_by(|left, right| order(left, right));
            let loaded = visible.len();
            for index in visible {
                let mut item = all[*index].clone();
                item.group = Some(key.clone());
                conversations.push(item);
            }
            descriptors.push(ConversationGroup {
                name: match &key {
                    ConversationGroupKey::Project { path: Some(path) } => self.project_name(path),
                    _ => None,
                },
                key,
                total: indices.len() as u32,
                loaded: loaded as u32,
                latest_activity_unix_ms: latest,
                collapsed,
            });
        }
        // Open conversations stay identifiable even where the list does not
        // show them: a sub-agent's, one opened by link, or one beyond the window.
        let open: Vec<String> = std::iter::once(&self.session)
            .chain(&self.parked)
            .filter_map(|session| session.conversation_id().map(str::to_owned))
            .collect();
        for item in &all {
            if open.contains(&item.conversation.id)
                && !conversations
                    .iter()
                    .any(|listed| listed.conversation.id == item.conversation.id)
            {
                conversations.push(item.clone());
            }
        }
        let mut projects: Vec<ConversationProject> = projects
            .into_iter()
            .map(|(path, (latest, worktrees))| {
                if let Some((runtime_id, project)) = self.native_project(&path) {
                    return ConversationProject {
                        path,
                        primary_folder: project.roots.first().map(|root| root.path.clone()),
                        native_runtime_id: Some(runtime_id),
                        name: Some(project.name.clone()),
                        folders: project
                            .roots
                            .iter()
                            .skip(1)
                            .map(|root| root.path.clone())
                            .collect(),
                        worktrees: Vec::new(),
                        latest_activity_unix_ms: latest,
                    };
                }
                let record = self.project_record(&path);
                ConversationProject {
                    primary_folder: Some(path.clone()),
                    native_runtime_id: None,
                    name: record.and_then(|project| project.name.clone()),
                    folders: record.map_or_else(Vec::new, |project| project.folders.clone()),
                    worktrees: worktrees.into_iter().collect(),
                    path,
                    latest_activity_unix_ms: latest,
                }
            })
            .collect();
        projects.sort_by(|l, r| {
            r.latest_activity_unix_ms
                .cmp(&l.latest_activity_unix_ms)
                .then_with(|| l.path.cmp(&r.path))
        });
        // Unread conversations show a failed last turn; so do the native
        // sub-agents of the listed ones, in their parent's sub-agents.
        let parents: BTreeSet<&str> = conversations
            .iter()
            .map(|item| item.conversation.id.as_str())
            .collect();
        let sub_agents: Vec<&ConversationSummary> = all
            .iter()
            .map(|item| &item.conversation)
            .filter(|summary| {
                summary
                    .parent_conversation_id
                    .as_deref()
                    .is_some_and(|parent| parents.contains(parent))
            })
            .collect();
        let failed = self.failed_turns(
            conversations
                .iter()
                .filter(|item| item.unread)
                .map(|item| &item.conversation)
                .chain(sub_agents.iter().copied()),
        );
        self.sub_agent_failures.borrow_mut().extend(
            sub_agents
                .iter()
                .filter_map(|summary| Some((summary.id.clone(), *failed.get(&summary.id)?))),
        );
        for item in &mut conversations {
            if let Some(failed) = failed.get(&item.conversation.id) {
                item.failed = *failed;
            }
        }
        Page {
            history_failures: Vec::new(),
            working_directory_projects: self.working_directory_projects(&all, &projects),
            conversations,
            groups: descriptors,
            projects,
        }
    }

    pub(crate) fn search_conversations(
        &self,
        text: String,
    ) -> Result<ConversationSearch, RuntimeError> {
        let text = Some(text.trim())
            .filter(|text| !text.is_empty())
            .ok_or_else(|| RuntimeError::invalid("搜索文字不能为空"))?;
        // Searching follows typing, so it searches the latest index rather
        // than indexing the runtimes again. Only candidates in this view
        // need titles; native archives already include them.
        let marked = self.shows_archived().then(|| self.locally_archived_ids());
        let segments = marked.as_ref().map(|ids| self.link_segment_ids(Some(ids)));
        for runtime in self.runtime_instances.iter().filter(|r| r.enabled) {
            if let Err(error) = self.read_unread_where(runtime, |id| {
                marked.as_ref().is_none_or(|ids| ids.contains(id))
                    || segments.as_ref().is_some_and(|ids| ids.contains(id))
            }) {
                tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="search",runtime_id=%runtime.id,runtime_type_id=%runtime.type_id,detail=%error);
            }
        }
        let page = self.browse_read(self.cached_sources(), Some(text), false);
        Ok(ConversationSearch {
            conversations: page.conversations,
            groups: page.groups,
        })
    }

    pub(crate) fn find_conversation(
        &self,
        conversation_id: String,
    ) -> Result<Option<ListedConversation>, RuntimeError> {
        let id = conversation_id.as_str();
        if let Some(source) = self
            .listed_sources()
            .get(id)
            .filter(|source| !source.provisional)
        {
            return Ok(Some(self.listed(source, &self.projectless_places())));
        }
        let linked = self.conversation_links.iter().any(|link| link.id == id);
        let owner = self
            .runtime_instances
            .iter()
            .find(|r| r.enabled && id.starts_with(&format!("{}:", r.id)));
        let found = match owner {
            Some(runtime) if !linked => self
                .conversation_summary(runtime, id)?
                .map(|summary| ListedSource {
                    summary,
                    archived_natively: false,
                    provisional: false,
                })
                .or_else(|| self.reach(runtime, id)),
            _ => None,
        }
        .or_else(|| {
            let (sources, _) = self.list_sources(false).ok()?;
            sources
                .into_iter()
                .find(|source| source.summary.id == id && !source.provisional)
        });
        Ok(found
            .as_ref()
            .map(|source| self.listed(source, &self.projectless_places())))
    }

    /// A conversation its runtime describes by identity, outside every
    /// listing in use, kept so that opening it finds it: reading one thread
    /// rather than the runtime's whole archive.
    pub(super) fn reach(&self, runtime: &RuntimeInstance, id: &str) -> Option<ListedSource> {
        let native_id = id.strip_prefix(&format!("{}:", runtime.id))?;
        let source_key = self.runtime_source_key(runtime).ok()?;
        let read = self
            .history_config(runtime)
            .and_then(|config| Ok(history::read_summary(&config, &runtime.id, native_id)?));
        match read {
            Ok(Some(summary)) if source_key.is_current(self) => {
                let (summary, archived) =
                    summary.split_map(|(summary, archived)| (summary, archived));
                let archived_natively = *archived;
                let summary = browsing::SharedSummary::retained(summary);
                let source = ListedSource {
                    summary,
                    archived_natively,
                    provisional: false,
                };
                self.reached.borrow_mut().insert(
                    id.to_owned(),
                    browsing::CachedSource {
                        source: source_key,
                        value: source.clone(),
                    },
                );
                Some(source)
            }
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="reach",runtime_id=%runtime.id,runtime_type_id=%runtime.type_id,detail=%error);
                None
            }
        }
    }

    pub(crate) fn load_more_conversations(
        &mut self,
        group: ConversationGroupKey,
    ) -> Result<crate::ConfigurationSnapshot, RuntimeError> {
        *self.browser_window.extra_pages.entry(group).or_default() += 1;
        // Paging extends the current index, not the set of native sources.
        // cached_list still reads newly visible provisional summaries.
        Ok(self.cached_list())
    }

    pub(crate) fn show_archived_conversations(
        &mut self,
        archived: bool,
    ) -> Result<crate::ConfigurationSnapshot, RuntimeError> {
        if archived != self.browser_window.archived {
            self.browser_window = BrowserWindow {
                archived,
                ..Default::default()
            };
        }
        self.list()
    }

    pub(super) fn release_collapsed_browser_capacity(&self, group: &ConversationGroupKey) {
        let mut initial = self.browser_window.initial_loads.borrow_mut();
        if initial.remove(group).is_some_and(|capacity| capacity > 0) {
            // Keep every other granted capacity. Only groups with no initial
            // capacity may compete for the space this explicit fold released.
            initial.retain(|_, capacity| *capacity > 0);
        }
    }

    pub(super) fn reset_browser_allocation(&self) {
        self.browser_window.initial_loads.borrow_mut().clear();
    }

    /// New grouping, sorting, or filters start the list from its first pages.
    pub(super) fn reset_browser_pages(&mut self) {
        self.browser_window.extra_pages.clear();
        self.browser_window.initial_loads.get_mut().clear();
    }

    /// Changes Velune's organization of the conversations; the runtime's own
    /// archive is used where it has one.
    pub(crate) fn organize_conversations(
        &mut self,
        references: Vec<ConversationReference>,
        change: ConversationOrganizationChange,
    ) -> Result<crate::ConfigurationSnapshot, RuntimeError> {
        if references.is_empty() {
            return Err(RuntimeError::invalid("没有选择会话"));
        }
        let places = self.projectless_places();
        let mut targets = Vec::new();
        for reference in &references {
            // Open archived threads remain manageable even when the ordinary
            // browser no longer includes them. Resolve within their native owner.
            let source = self
                .listed_sources()
                .get(&reference.conversation_id)
                .cloned()
                .or_else(|| {
                    self.runtime_instance(&reference.runtime_id)
                        .ok()
                        .and_then(|runtime| self.reach(runtime, &reference.conversation_id))
                })
                .filter(|source| source.summary.runtime_id == reference.runtime_id)
                .ok_or_else(|| RuntimeError::invalid("会话列表中没有这个会话，请刷新后重试"))?;
            targets.push((
                source.summary.clone(),
                source.archived_natively,
                self.listed(&source, &places),
            ));
        }
        let mut native_updated = BTreeMap::new();
        // Native CAS uses the receipt actually seen by the caller. Never
        // substitute a newer cache revision or retry an acknowledgement.
        if matches!(
            change,
            ConversationOrganizationChange::MarkRead | ConversationOrganizationChange::MarkUnread
        ) {
            for ((summary, _, _), reference) in targets.iter().zip(&references) {
                if summary.native_read_state.is_some() || reference.native_read_state.is_some() {
                    let (owner, native) = self.native_segment(summary);
                    let expected_id = format!("{owner}:{native}");
                    let receipt = reference
                        .native_read_state
                        .as_ref()
                        .filter(|r| r.conversation_id == expected_id)
                        .ok_or_else(|| RuntimeError::invalid("会话已读状态已改变，请刷新后重试"))?;
                    let (runtime_id, _) = receipt
                        .conversation_id
                        .split_once(':')
                        .ok_or_else(|| RuntimeError::invalid("原生会话身份无效"))?;
                    let runtime = self.runtime_instance(runtime_id)?;
                    if runtime.type_id != RuntimeKind::Codex166 {
                        return Err(RuntimeError::invalid("此运行时没有原生已读状态"));
                    }
                    let result = velune_agent_runtime::native::update_read_state(
                        &self.history_config(runtime)?,
                        runtime_id,
                        receipt,
                        matches!(change, ConversationOrganizationChange::MarkRead),
                    )
                    .map_err(|error| {
                        if error.code() == "read_state_conflict" {
                            let mut failure = crate::Failure::new(
                                crate::FailureKind::Conflict,
                                "会话已更新，未覆盖新的已读状态；请查看后重试",
                            );
                            failure.code = error.code().into();
                            RuntimeError::Contract(failure)
                        } else {
                            RuntimeError::context("修改原生已读状态", error)
                        }
                    })?;
                    self.keep_native_read_state(&result);
                    native_updated.insert(
                        summary.id.clone(),
                        (summary.native_read_state.clone(), result),
                    );
                }
            }
        }
        let mut next = self.organization.clone();
        let now = unix_ms_now();
        for (summary, archived_natively, listed) in &targets {
            if native_updated.contains_key(&summary.id) {
                continue;
            }
            let native_archive = matches!(
                change,
                ConversationOrganizationChange::Archive | ConversationOrganizationChange::Unarchive
            ) && self.archives_natively(summary);
            if native_archive {
                let archiving = matches!(change, ConversationOrganizationChange::Archive);
                if archiving == *archived_natively {
                    continue;
                }
                if archiving
                    && std::iter::once(&self.session)
                        .chain(&self.parked)
                        .any(|session| session.presents(&summary.id) && session.executing())
                {
                    return Err(RuntimeError::invalid("请先停止正在执行的任务，再归档会话"));
                }
                if archiving {
                    self.for_each_session(|core| {
                        if core.session.presents(&summary.id)
                            && core.session.active_state.live().is_some()
                        {
                            core.invalidate_execution()?;
                            core.session.source = None;
                            if let Ok(Some(target)) = core.control_target() {
                                core.control_observations.invalidate(&target);
                            }
                        }
                        Ok(())
                    })?;
                }
                self.change_native_archive(summary, archiving)?;
            }
            let position = match next
                .conversations
                .iter()
                .position(|marks| marks.conversation_id == summary.id)
            {
                Some(position) => position,
                None => {
                    next.conversations.push(ConversationMarks {
                        conversation_id: summary.id.clone(),
                        ..Default::default()
                    });
                    next.conversations.len() - 1
                }
            };
            let marks = &mut next.conversations[position];
            match &change {
                ConversationOrganizationChange::Pin => {
                    if listed.archived {
                        return Err(RuntimeError::invalid("已归档的会话不能置顶"));
                    }
                    marks.pinned_at_unix_ms.get_or_insert(now);
                }
                ConversationOrganizationChange::Unpin => marks.pinned_at_unix_ms = None,
                ConversationOrganizationChange::Archive => {
                    marks.pinned_at_unix_ms = None;
                    if !native_archive {
                        marks.archived_at_unix_ms.get_or_insert(now);
                    }
                }
                ConversationOrganizationChange::Unarchive => marks.archived_at_unix_ms = None,
                ConversationOrganizationChange::MarkRead => {
                    marks.marked_unread = false;
                    if let Some(at) = summary.updated_at_unix_ms {
                        marks.seen_activity_unix_ms = Some(at);
                    }
                }
                ConversationOrganizationChange::MarkUnread => marks.marked_unread = true,
                ConversationOrganizationChange::MoveToProject { path } => {
                    let project = path
                        .as_ref()
                        .map(|path| {
                            if Path::new(path).is_absolute() || self.native_project(path).is_some()
                            {
                                Ok(self.moved_project(summary, path, &places))
                            } else {
                                Err(RuntimeError::invalid("项目路径必须是绝对路径"))
                            }
                        })
                        .transpose()?;
                    marks.project = project.filter(|project| {
                        Some(project) != self.working_project(summary, &places).as_ref()
                    });
                }
            }
        }
        next.conversations.retain(|marks| !marks.is_empty());
        if native_updated.len() != targets.len() {
            self.store_organization(next)?;
        }
        let mut listing = if matches!(
            change,
            ConversationOrganizationChange::Archive | ConversationOrganizationChange::Unarchive
        ) {
            self.list()?
        } else {
            self.cached_list()
        };
        // Use the committed native receipt in this response even when a
        // separate retained cache could not accept a copy. Keep any newer
        // receipt already read from the runtime while building the list.
        for item in &mut listing.conversations {
            if let Some((previous, receipt)) = native_updated.get(&item.conversation.id)
                && (item.conversation.native_read_state.is_none()
                    || item.conversation.native_read_state == *previous)
            {
                item.conversation.native_read_state = Some(receipt.as_ref().clone());
                item.unread = !item.archived && receipt.unread;
            }
        }
        // The list read again may already show newer activity than the mark.
        let listed = self.listed_sources();
        for (summary, _, _) in &targets {
            let now = listed
                .get(&summary.id)
                .map(|source| self.listed(source, &places));
            tracing::info!(target:"velune_application", event="conversation_organized", change=?change, conversation_id=%summary.id, runtime_id=%summary.runtime_id, marked_activity_unix_ms=?summary.updated_at_unix_ms, seen_activity_unix_ms=?self.marks(&summary.id).and_then(|marks| marks.seen_activity_unix_ms), listed_activity_unix_ms=?now.as_ref().and_then(|item| item.conversation.updated_at_unix_ms), unread=?now.as_ref().map(|item| item.unread));
        }
        Ok(listing)
    }

    /// A deleted conversation keeps no organization.
    pub(super) fn forget_conversation(&mut self, id: &str) {
        if !self
            .organization
            .conversations
            .iter()
            .any(|m| m.conversation_id == id)
            && !self.organization.generated_titles.contains_key(id)
        {
            return;
        }
        let mut next = self.organization.clone();
        next.conversations
            .retain(|marks| marks.conversation_id != id);
        next.generated_titles.remove(id);
        if let Err(error) = self.store_organization(next) {
            tracing::warn!(target:"velune_application",event="conversation_organization_store_failed",detail=%error);
        }
    }

    /// Sending to an archived conversation brings it back to the list.
    pub(super) fn restore_archived(&mut self, id: &str) {
        if !self
            .marks(id)
            .is_some_and(|marks| marks.archived_at_unix_ms.is_some())
        {
            return;
        }
        let mut next = self.organization.clone();
        for marks in next
            .conversations
            .iter_mut()
            .filter(|m| m.conversation_id == id)
        {
            marks.archived_at_unix_ms = None;
        }
        next.conversations.retain(|marks| !marks.is_empty());
        if let Err(error) = self.store_organization(next) {
            tracing::warn!(target:"velune_application",event="conversation_organization_store_failed",detail=%error);
        }
    }

    pub(super) fn store_organization(
        &mut self,
        next: crate::config::ConversationOrganization,
    ) -> Result<(), RuntimeError> {
        self.repository.store_conversation_organization(&next)?;
        self.organization = next;
        Ok(())
    }

    /// Only a conversation that is one native thread of a runtime with an
    /// archive is archived there; a continued one spans runtimes.
    fn archives_natively(&self, summary: &ConversationSummary) -> bool {
        !self
            .conversation_links
            .iter()
            .any(|link| link.id == summary.id)
            && self
                .runtime_instances
                .iter()
                .find(|runtime| runtime.id == summary.runtime_id)
                .is_some_and(|runtime| {
                    velune_agent_runtime::version::variant(runtime.type_id)
                        .can_archive_conversations
                })
    }

    fn change_native_archive(
        &self,
        summary: &ConversationSummary,
        archiving: bool,
    ) -> Result<(), RuntimeError> {
        let runtime = self.runtime_instance(&summary.runtime_id)?;
        let context = self.execution_context(runtime)?;
        let native_id = summary
            .id
            .strip_prefix(&format!("{}:", runtime.id))
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        velune_agent_runtime::native::manage_thread_at(
            runtime.type_id,
            &setting_path(runtime, "binary")?,
            &context.agent_dir,
            native_id,
            if archiving {
                velune_agent_runtime::native::ThreadChange::Archive
            } else {
                velune_agent_runtime::native::ThreadChange::Unarchive
            },
            &context.target,
            context.os_home.as_deref(),
        )
        .map_err(|error| {
            RuntimeError::context("Codex 原生归档失败；请检查运行时版本与目录后重试", error)
        })
    }
}

#[cfg(all(test, unix))]
mod pagination_tests {
    use super::*;
    use crate::{ApplicationService, ClientViewId, Command, Options, Response};
    use std::time::Instant;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(collapsed: bool) -> Self {
            let root =
                std::env::temp_dir().join(format!("velune-pagination-{}", crate::new_record_key()));
            fs::create_dir(&root).unwrap();
            // Deliberately slow unrelated history reader. Paging an existing index
            // must neither restart it nor require it to remain available.
            fs::write(
                root.join("huihua_sessions.mjs"),
                r#"
import json, pathlib, sys, time
root = pathlib.Path(__file__).parent
request = json.load(sys.stdin)
with (root / 'scans').open('a') as log: log.write(request['operation'] + '\n')
time.sleep(0.15)
if (root / 'fail').exists(): sys.exit(1)
print(json.dumps({'contractVersion':1,'sessions':[
 {'nativeId':f'{group}-{row}','title':{'source':'native','text':f'Group {group} row {row}'},
  'cwd':f'/synthetic/project-{group}','updatedAtUnixMs':100000-group*100-row,'createdAtUnixMs':1}
 for group in range(6) for row in range(4)]}))
"#,
            )
            .unwrap();
            let runtime = RuntimeInstance {
                enabled: true,
                id: "synthetic".into(),
                name: "Synthetic".into(),
                type_id: RuntimeKind::DeepSeek,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    ("binary".into(), "/synthetic/unused-agent".into()),
                    ("nodeBinary".into(), "/usr/bin/python3".into()),
                    (
                        "agentDir".into(),
                        root.join("state").to_string_lossy().into_owned(),
                    ),
                ]),
            };
            let mut config = crate::repository::PersistedConfig {
                runtime_instances: vec![runtime],
                ..Default::default()
            };
            config.gateways.push(GatewayConfig {
                id: "default".into(),
                name: "Default".into(),
                providers: Vec::new(),
                routes: Vec::new(),
            });
            config.conversation_browser_group_limit = 2;
            config.conversation_browser_total_limit = 4;
            config.conversation_browser_preferences.grouping = ConversationBrowserGrouping::Project;
            config.conversation_browser_preferences.group_sort = ConversationGroupSort::Manual;
            config.conversation_browser_preferences.group_order = (0..6).map(Self::key).collect();
            if collapsed {
                config
                    .conversation_browser_preferences
                    .collapsed
                    .extend([Self::key(3), Self::key(4)]);
            }
            fs::write(
                root.join("generic-config.json"),
                serde_json::to_vec(&config).unwrap(),
            )
            .unwrap();
            Self(root)
        }
        fn key(group: usize) -> ConversationGroupKey {
            ConversationGroupKey::Project {
                path: Some(format!("/synthetic/project-{group}")),
            }
        }
        fn open(&self) -> ApplicationService {
            ApplicationService::open(Options {
                home_directory: self.0.clone(),
                resources_directory: self.0.clone(),
            })
            .unwrap()
        }
        fn scan_count(&self) -> usize {
            fs::read_to_string(self.0.join("scans"))
                .unwrap()
                .lines()
                .count()
        }
        fn read(service: &ApplicationService, view: &str) -> crate::ConfigurationSnapshot {
            let Response::ReadConfiguration(value) = service
                .execute(ClientViewId(view.into()), Command::ReadConfiguration)
                .unwrap()
            else {
                panic!("configuration response")
            };
            value
        }
        fn more(
            service: &ApplicationService,
            view: &str,
            group: usize,
        ) -> crate::ConfigurationSnapshot {
            let Response::LoadMoreConversations(value) = service
                .execute(
                    ClientViewId(view.into()),
                    Command::LoadMoreConversations {
                        group: Self::key(group),
                    },
                )
                .unwrap()
            else {
                panic!("pagination response")
            };
            value
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn archive_reads_native_pages_and_only_locally_archived_titles() {
        use std::os::unix::fs::PermissionsExt;

        let root =
            std::env::temp_dir().join(format!("velune-archive-list-{}", crate::new_record_key()));
        fs::create_dir(&root).unwrap();
        let fixture = Fixture(root);
        let root = &fixture.0;
        let codex = root.join("codex");
        fs::write(
            &codex,
            r#"#!/usr/bin/python3
import json, pathlib, sys, time
root = pathlib.Path(__file__).parent
if "--version" in sys.argv:
    print("codex-cli 0.166.0")
    raise SystemExit(0)
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request["method"]
    if method == "initialize":
        result = {}
    elif method == "project/list":
        result = {"data": [{"id": "work", "name": "Work", "roots": []}], "nextCursor": None}
    else:
        assert method == "thread/list", method
        params = request["params"]
        archived = params["archived"]
        with (root / "codex-scans").open("a") as log:
            log.write(("archive" if archived else "active") + "\n")
        start = int(params.get("cursor", "0"))
        total = 305 if archived else 1200
        end = min(total, start + params["limit"])
        time.sleep(0.015)
        if archived and (root / "fail-archive").exists():
            print(json.dumps({"id": request["id"], "error": {"code": -32603, "message": "synthetic archive outage"}}), flush=True)
            continue
        result = {"data": [
            {"id": f"thread-{i}", "name": f"Codex session {i}", "projectId": "work",
             "createdAt": 1, "updatedAt": 10000-i, "historyMode": "paginated"}
            for i in range(start, end)
        ], "nextCursor": str(end) if end < total else None}
    print(json.dumps({"id": request["id"], "result": result}), flush=True)
"#,
        )
        .unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            root.join("pi_sessions.mjs"),
            r#"
import json, pathlib, sys
root = pathlib.Path(__file__).parent
want = json.load(sys.stdin)["want"]
with (root / "pi-scans").open("a") as log:
    log.write(",".join(pathlib.Path(path).stem for path in want) + "\n")
print(json.dumps({"contractVersion": 3, "files": [
    {"path": str(root / f"{i}.jsonl"), "size": "1", "mtimeNs": "1",
     "createdAtUnixMs": 1, "modifiedAtUnixMs": 1000-i}
    for i in range(80)
], "sessions": [
    {"path": path, "name": f"Pi session {pathlib.Path(path).stem}", "modifiedUnixMs": 1000}
    for path in want
]}))
"#,
        )
        .unwrap();
        let instance = |id: &str, type_id, binary: &Path| RuntimeInstance {
            enabled: true,
            id: id.into(),
            name: id.into(),
            type_id,
            gateway_id: "default".into(),
            settings: BTreeMap::from([
                ("binary".into(), binary.to_string_lossy().into_owned()),
                ("nodeBinary".into(), "/usr/bin/python3".into()),
                (
                    "agentDir".into(),
                    root.join("homes").join(id).display().to_string(),
                ),
            ]),
        };
        let config = crate::repository::PersistedConfig {
            runtime_instances: vec![
                instance("codex", RuntimeKind::Codex166, &codex),
                instance("pi", RuntimeKind::Pi, &root.join("unused-pi")),
            ],
            gateways: vec![GatewayConfig {
                id: "default".into(),
                name: "Default".into(),
                providers: Vec::new(),
                routes: Vec::new(),
            }],
            conversation_browser_group_limit: 2,
            conversation_browser_total_limit: 4,
            conversation_browser_preferences: crate::config::ConversationBrowserPreferences {
                grouping: ConversationBrowserGrouping::None,
                ..Default::default()
            },
            ..Default::default()
        };
        fs::write(
            root.join("generic-config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let pi_id = |index| format!("pi:{}/{index}.jsonl", root.display());
        let (repository, _) = crate::repository::Repository::open(root).unwrap();
        repository
            .store_conversation_organization(&crate::config::ConversationOrganization {
                conversations: [79, 78]
                    .into_iter()
                    .map(|index| ConversationMarks {
                        conversation_id: pi_id(index),
                        archived_at_unix_ms: Some(1),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .unwrap();
        repository
            .store_conversation_links(&[crate::config::ConversationLink {
                id: pi_id(78),
                segments: [78, 77]
                    .into_iter()
                    .map(|index| {
                        serde_json::from_value(serde_json::json!({
                            "runtimeInstanceId": "pi",
                            "runtimeTypeId": RuntimeKind::Pi,
                            "nativeConversationId": pi_id(index),
                            "cutoff": (index == 78).then(|| serde_json::json!({
                                "messageCount": 1,
                                "prefixDigest": "a".repeat(64),
                            })),
                            "deleted": false,
                        }))
                        .unwrap()
                    })
                    .collect(),
            }])
            .unwrap();
        drop(repository);
        let app = fixture.open();
        let view = ClientViewId("archive".into());
        let show = || {
            let Response::ShowArchivedConversations(value) = app
                .execute(
                    view.clone(),
                    Command::ShowArchivedConversations { archived: true },
                )
                .unwrap()
            else {
                panic!("archive response")
            };
            value
        };
        let started = Instant::now();
        let snapshot = show();
        assert!(
            snapshot.history_failures.is_empty(),
            "{:?}",
            snapshot.history_failures
        );
        let codex_scans = fs::read_to_string(root.join("codex-scans")).unwrap();
        eprintln!(
            "cold archive: {:?}; {} Codex pages",
            started.elapsed(),
            codex_scans.lines().count()
        );
        assert_eq!(codex_scans.lines().collect::<Vec<_>>(), ["archive"; 4]);
        let group = snapshot
            .conversation_groups
            .iter()
            .find(|group| group.key == ConversationGroupKey::All)
            .unwrap();
        assert_eq!(group.total, 307);
        assert!((1..group.total).contains(&group.loaded));
        assert!(snapshot.conversations.iter().all(|item| item.archived));
        let Response::LoadMoreConversations(next) = app
            .execute(
                view.clone(),
                Command::LoadMoreConversations {
                    group: ConversationGroupKey::All,
                },
            )
            .unwrap()
        else {
            panic!("pagination response")
        };
        assert!(next.conversations.len() > snapshot.conversations.len());

        for (query, expected) in [
            ("Codex session 304", "codex:thread-304".into()),
            ("Pi session 79", pi_id(79)),
            ("Pi session 78", pi_id(78)),
        ] {
            let Response::SearchConversations(result) = app
                .execute(
                    view.clone(),
                    Command::SearchConversations { text: query.into() },
                )
                .unwrap()
            else {
                panic!("search response")
            };
            assert_eq!(result.conversations.len(), 1);
            assert_eq!(result.conversations[0].conversation.id, expected);
        }
        assert_eq!(
            fs::read_to_string(root.join("codex-scans")).unwrap(),
            codex_scans
        );
        let pi_scans = fs::read_to_string(root.join("pi-scans")).unwrap();
        assert_eq!(
            pi_scans
                .lines()
                .skip(1)
                .flat_map(|ids| ids.split(','))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["77", "78", "79"])
        );

        fs::write(root.join("fail-archive"), "").unwrap();
        let partial = show();
        assert_eq!(partial.history_failures.len(), 1);
        assert_eq!(partial.history_failures[0].runtime_id, "codex");
        assert_eq!(partial.conversations.len(), 2);
        assert!(
            partial
                .conversations
                .iter()
                .all(|item| item.conversation.runtime_id == "pi")
        );
    }

    #[test]
    fn pagination_beyond_initial_budget_preserves_rows_and_client_windows_without_rescanning() {
        let fixture = Fixture::new(false);
        let app = fixture.open();
        let started = Instant::now();
        let first = Fixture::read(&app, "first");
        let scan_elapsed = started.elapsed();
        assert_eq!(
            first
                .conversation_groups
                .iter()
                .map(|g| g.loaded)
                .collect::<Vec<_>>(),
            [2, 2, 0, 0, 0, 0]
        );
        let original: BTreeSet<_> = first
            .conversations
            .iter()
            .map(|c| c.conversation.id.clone())
            .collect();
        let scans = fixture.scan_count();
        fs::write(fixture.0.join("fail"), "transient history outage").unwrap();
        let started = Instant::now();
        let next = Fixture::more(&app, "first", 4);
        let page_elapsed = started.elapsed();
        assert_eq!(
            next.conversation_groups
                .iter()
                .map(|g| g.loaded)
                .collect::<Vec<_>>(),
            [2, 2, 0, 0, 2, 0]
        );
        assert!(
            original.is_subset(
                &next
                    .conversations
                    .iter()
                    .map(|c| c.conversation.id.clone())
                    .collect()
            )
        );
        assert_eq!(
            fixture.scan_count(),
            scans,
            "paging must not scan an unrelated/native runtime again"
        );
        let last = Fixture::more(&app, "first", 5);
        assert_eq!(
            last.conversation_groups
                .iter()
                .map(|g| g.loaded)
                .collect::<Vec<_>>(),
            [2, 2, 0, 0, 2, 2]
        );
        let retry = Fixture::more(&app, "first", 4);
        assert_eq!(retry.conversation_groups[4].loaded, 4);
        // A second real service view starts at its own initial window. Its read
        // also demonstrates that explicit refresh still scans the native owner.
        fs::remove_file(fixture.0.join("fail")).unwrap();
        let other = Fixture::read(&app, "second");
        assert_eq!(other.conversation_groups[4].loaded, 0);
        let restored = Fixture::more(&app, "first", 3);
        assert_eq!(restored.conversation_groups[4].loaded, 4);
        assert_eq!(restored.conversation_groups[5].loaded, 2);
        eprintln!(
            "pagination benchmark: synthetic native scan={scan_elapsed:?}, indexed page={page_elapsed:?}, page rescans=0"
        );
        app.close(false).unwrap();
    }

    #[test]
    fn pagination_respects_source_collapsed_state_until_explicitly_expanded() {
        let fixture = Fixture::new(true);
        let app = fixture.open();
        let initial = Fixture::read(&app, "view");
        assert!(initial.conversation_groups[4].collapsed);
        let next = Fixture::more(&app, "view", 4);
        assert_eq!(next.conversation_groups[4].loaded, 0);
        let mut preferences = next.conversation_browser_preferences;
        preferences.collapsed.retain(|key| *key != Fixture::key(4));
        let scans = fixture.scan_count();
        fs::write(fixture.0.join("fail"), "unrelated native outage").unwrap();
        app.execute(
            ClientViewId("view".into()),
            Command::SetConversationBrowserPreferences { preferences },
        )
        .unwrap();
        let expanded = Fixture::more(&app, "view", 4);
        assert_eq!(expanded.conversation_groups[4].loaded, 4);
        assert!(!expanded.conversation_groups[4].collapsed);
        assert!(expanded.conversation_groups[3].collapsed);
        assert_eq!(fixture.scan_count(), scans);
        assert_eq!(expanded.conversations.len(), 8);
        app.close(false).unwrap();
    }

    fn cold_fixture(rows: usize) -> Fixture {
        let fixture = Fixture::new(false);
        let config_path = fixture.0.join("generic-config.json");
        let mut config: crate::repository::PersistedConfig =
            serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config.runtime_instances[0].type_id = RuntimeKind::ClaudeCode;
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        for group in 0..6 {
            let folder = fixture.0.join(format!("state/projects/group-{group}"));
            fs::create_dir_all(&folder).unwrap();
            for row in 0..rows {
                let id = format!("00000000-0000-4000-8000-{group:06}{row:06}");
                let record = serde_json::json!({"type":"user", "uuid":format!("u-{group}-{row}"), "parentUuid":null,
                    "sessionId":id, "cwd":format!("/synthetic/project-{group}"), "timestamp":"2026-01-01T00:00:00Z",
                    "message":{"role":"user", "content":format!("Synthetic group {group} row {row}")}});
                fs::write(folder.join(format!("{id}.jsonl")), format!("{record}\n")).unwrap();
            }
        }
        fixture
    }

    #[test]
    fn pagination_cold_summaries_report_failure_and_retry_without_losing_loaded_groups() {
        let fixture = cold_fixture(4);
        let app = fixture.open();
        let initial = Fixture::read(&app, "view");
        assert_eq!(initial.conversations.len(), 4);
        assert_eq!(initial.conversation_groups[4].loaded, 0);
        let original: BTreeSet<_> = initial
            .conversations
            .iter()
            .map(|c| c.conversation.id.clone())
            .collect();
        // Ambiguous duplicate native identity makes the production Python reader
        // fail before reading summaries, without changing source configuration.
        let duplicate = fixture.0.join("state/projects/duplicate");
        fs::create_dir(&duplicate).unwrap();
        fs::write(
            duplicate.join("00000000-0000-4000-8000-000000000000.jsonl"),
            "{}\n",
        )
        .unwrap();
        let failed = Fixture::more(&app, "view", 4);
        assert_eq!(
            failed.conversation_groups[4].loaded, 0,
            "loaded must describe actual rows after a read failure"
        );
        assert!(
            failed
                .history_failures
                .iter()
                .any(|f| f.runtime_id == "synthetic")
        );
        assert!(
            original.is_subset(
                &failed
                    .conversations
                    .iter()
                    .map(|c| c.conversation.id.clone())
                    .collect()
            )
        );
        fs::remove_dir_all(duplicate).unwrap();
        let retried = Fixture::more(&app, "view", 4);
        assert!(retried.conversation_groups[4].loaded >= 2);
        assert!(retried.history_failures.is_empty());
        assert!(
            original.is_subset(
                &retried
                    .conversations
                    .iter()
                    .map(|c| c.conversation.id.clone())
                    .collect()
            )
        );
        assert!(
            retried
                .conversations
                .iter()
                .filter(|c| c.group.as_ref() == Some(&Fixture::key(4)))
                .all(|c| c
                    .conversation
                    .title
                    .display_text()
                    .starts_with("Synthetic group 4"))
        );
        app.close(false).unwrap();
    }
    #[test]
    fn pagination_large_cold_group_makes_progress_with_stale_file_times() {
        let fixture = cold_fixture(30);
        let app = fixture.open();
        let initial = Fixture::read(&app, "view");
        let original: BTreeSet<_> = initial
            .conversations
            .iter()
            .map(|c| c.conversation.id.clone())
            .collect();
        let loaded = Fixture::more(&app, "view", 4);
        assert_eq!(loaded.conversation_groups[4].total, 30);
        assert!(
            original.is_subset(
                &loaded
                    .conversations
                    .iter()
                    .map(|c| c.conversation.id.clone())
                    .collect()
            )
        );
        assert!(
            loaded.conversation_groups[4].loaded > 0,
            "a completed page must make visible progress even when native dates precede file times"
        );
        app.close(false).unwrap();
    }

    #[test]
    fn pagination_activity_order_preserves_seen_rows_and_target_progress() {
        let fixture = cold_fixture(30);
        let config_path = fixture.0.join("generic-config.json");
        let mut config: crate::repository::PersistedConfig =
            serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config.conversation_browser_preferences.group_sort = Default::default();
        config.conversation_browser_preferences.group_order.clear();
        fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let app = fixture.open();
        let initial = Fixture::read(&app, "view");
        let mut seen: BTreeSet<_> = initial
            .conversations
            .iter()
            .map(|item| item.conversation.id.clone())
            .collect();
        let targets: Vec<_> = initial
            .conversation_groups
            .iter()
            .map(|group| group.key.clone())
            .collect();
        for key in targets {
            let mut before = initial
                .conversation_groups
                .iter()
                .find(|group| group.key == key)
                .unwrap()
                .loaded;
            for _ in 0..20 {
                let Response::LoadMoreConversations(next) = app
                    .execute(
                        ClientViewId("view".into()),
                        Command::LoadMoreConversations { group: key.clone() },
                    )
                    .unwrap()
                else {
                    panic!("pagination response")
                };
                let current = next
                    .conversation_groups
                    .iter()
                    .find(|group| group.key == key)
                    .unwrap();
                let visible: BTreeSet<_> = next
                    .conversations
                    .iter()
                    .map(|item| item.conversation.id.clone())
                    .collect();
                eprintln!(
                    "activity pagination {key:?}: before={before}, after={}, total={}, visible={}",
                    current.loaded,
                    current.total,
                    visible.len()
                );
                assert!(
                    seen.is_subset(&visible),
                    "paging {key:?} lost previously seen rows: {:?}",
                    seen.difference(&visible).collect::<Vec<_>>()
                );
                assert!(
                    current.loaded > before || before == current.total,
                    "paging {key:?} did not advance from {before}"
                );
                seen.extend(visible);
                before = current.loaded;
                if current.loaded == current.total {
                    break;
                }
            }
        }
        // A native refresh starts a new initial allocation so newly discovered
        // active projects can enter the first window instead of staying at 0/n.
        let folder = fixture.0.join("state/projects/new-project");
        fs::create_dir(&folder).unwrap();
        let record = serde_json::json!({"type":"user", "uuid":"new-user", "parentUuid":null,
            "sessionId":"00000000-0000-4000-8000-000006000000", "cwd":"/synthetic/project-6",
            "timestamp":"2026-11-16T00:00:00Z", "message":{"role":"user", "content":"New project"}});
        fs::write(
            folder.join("00000000-0000-4000-8000-000006000000.jsonl"),
            format!("{record}\n"),
        )
        .unwrap();
        let refreshed = Fixture::read(&app, "view");
        assert_eq!(
            refreshed
                .conversation_groups
                .iter()
                .find(|group| group.key == Fixture::key(6))
                .unwrap()
                .loaded,
            1
        );
        app.close(false).unwrap();
    }

    #[test]
    fn pagination_collapsing_a_group_releases_only_its_initial_capacity() {
        let fixture = Fixture::new(false);
        let app = fixture.open();
        let initial = Fixture::read(&app, "view");
        assert_eq!(
            initial
                .conversation_groups
                .iter()
                .map(|g| g.loaded)
                .collect::<Vec<_>>(),
            [2, 2, 0, 0, 0, 0]
        );
        let kept: BTreeSet<_> = initial
            .conversations
            .iter()
            .filter(|item| item.group.as_ref() == Some(&Fixture::key(1)))
            .map(|item| item.conversation.id.clone())
            .collect();
        let mut preferences = initial.conversation_browser_preferences;
        preferences.collapsed.push(Fixture::key(0));
        let scans = fixture.scan_count();
        let Response::SetConversationBrowserPreferences(folded) = app
            .execute(
                ClientViewId("view".into()),
                Command::SetConversationBrowserPreferences { preferences },
            )
            .unwrap()
        else {
            panic!("preference response")
        };
        assert_eq!(
            folded
                .conversation_groups
                .iter()
                .map(|g| g.loaded)
                .collect::<Vec<_>>(),
            [0, 2, 2, 0, 0, 0]
        );
        assert!(
            kept.is_subset(
                &folded
                    .conversations
                    .iter()
                    .map(|item| item.conversation.id.clone())
                    .collect()
            )
        );
        assert_eq!(fixture.scan_count(), scans);
        app.close(false).unwrap();
    }

    #[test]
    fn pagination_expanding_first_source_group_does_not_take_other_groups_capacity() {
        let fixture = Fixture::new(false);
        let config_path = fixture.0.join("generic-config.json");
        let mut config: crate::repository::PersistedConfig =
            serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config
            .conversation_browser_preferences
            .collapsed
            .push(Fixture::key(0));
        fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let app = fixture.open();
        let initial = Fixture::read(&app, "view");
        assert_eq!(
            initial
                .conversation_groups
                .iter()
                .map(|g| g.loaded)
                .collect::<Vec<_>>(),
            [0, 2, 2, 0, 0, 0]
        );
        let kept: BTreeSet<_> = initial
            .conversations
            .iter()
            .map(|item| item.conversation.id.clone())
            .collect();
        let mut preferences = initial.conversation_browser_preferences;
        preferences.collapsed.retain(|key| *key != Fixture::key(0));
        let scans = fixture.scan_count();
        let Response::SetConversationBrowserPreferences(expanded) = app
            .execute(
                ClientViewId("view".into()),
                Command::SetConversationBrowserPreferences { preferences },
            )
            .unwrap()
        else {
            panic!("preference response")
        };
        assert_eq!(
            expanded
                .conversation_groups
                .iter()
                .map(|g| g.loaded)
                .collect::<Vec<_>>(),
            [0, 2, 2, 0, 0, 0]
        );
        assert!(
            kept.is_subset(
                &expanded
                    .conversations
                    .iter()
                    .map(|item| item.conversation.id.clone())
                    .collect()
            )
        );
        let paged = Fixture::more(&app, "view", 0);
        assert_eq!(
            paged
                .conversation_groups
                .iter()
                .map(|g| g.loaded)
                .collect::<Vec<_>>(),
            [2, 2, 2, 0, 0, 0]
        );
        assert!(
            kept.is_subset(
                &paged
                    .conversations
                    .iter()
                    .map(|item| item.conversation.id.clone())
                    .collect()
            )
        );
        assert_eq!(fixture.scan_count(), scans);
        app.close(false).unwrap();
    }
}
