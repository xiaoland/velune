//! Velune's organization of the conversation list: the window of groups and
//! pages the list loads, and the user's pins, archive, read state, moves, and
//! project names. None of it is written to the runtimes' sessions, except an
//! archive the runtime keeps itself.
use super::*;
use crate::api::{
    ConversationGroup, ConversationOrganizationChange, ConversationProject, ConversationReference,
    ConversationSearch, ListedConversation,
};
use crate::config::{
    ConversationBrowserGrouping, ConversationBrowserProject, ConversationBrowserSort,
    ConversationGroupKey, ConversationMarks,
};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

/// What the list loads beyond its first pages; not persisted.
#[derive(Default)]
pub(super) struct BrowserWindow {
    /// Pages beyond the first that the user loaded, per group.
    extra_pages: BTreeMap<ConversationGroupKey, u32>,
    archived: bool,
}

/// A listed conversation and whether its runtime archived it.
#[derive(Clone)]
pub(super) struct ListedSource {
    pub(super) summary: ConversationSummary,
    pub(super) archived_natively: bool,
    /// Indexed but not read yet: identity, project, and file time only.
    pub(super) provisional: bool,
}

/// Where a runtime starts conversations that have no project: HOME, where
/// they land given no working directory, and the folders the runtime keeps
/// such conversations in. The filesystem root, where the app's own land,
/// counts for every runtime.
#[derive(Default)]
pub(super) struct ProjectlessPlaces {
    home: Option<PathBuf>,
    directories: Vec<PathBuf>,
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
    pub(super) conversations: Vec<ListedConversation>,
    pub(super) groups: Vec<ConversationGroup>,
    pub(super) projects: Vec<ConversationProject>,
}

pub(crate) fn unix_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

impl CoreRuntime {
    pub(super) fn shows_archived(&self) -> bool {
        self.browser_window.archived
    }

    /// The conversations every runtime archived itself, for the archive view.
    pub(super) fn natively_archived(&self) -> Vec<ConversationSummary> {
        let mut archived = Vec::new();
        for runtime in self.runtime_instances.iter().filter(|r| r.enabled) {
            let can_archive = velune_agent_runtime::version::variant(&runtime.type_id)
                .is_some_and(|variant| variant.can_archive_conversations);
            if !can_archive {
                continue;
            }
            match self
                .history_config(runtime)
                .and_then(|config| Ok(history::list_archived(&config, &runtime.id)?))
            {
                Ok(mut summaries) => archived.append(&mut summaries),
                Err(error) => {
                    tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="archived_list",runtime_id=%runtime.id,runtime_type_id=%runtime.type_id,detail=%error);
                }
            }
        }
        archived
    }

    /// The places each enabled runtime starts conversations without a project.
    pub(super) fn projectless_places(&self) -> BTreeMap<String, ProjectlessPlaces> {
        self.runtime_instances
            .iter()
            .filter(|runtime| runtime.enabled)
            .map(|runtime| {
                let home = self.user_home(runtime);
                let directories = velune_agent_runtime::version::variant(&runtime.type_id)
                    .zip(home.as_ref())
                    .map_or_else(Vec::new, |(variant, home)| {
                        variant
                            .projectless_directories
                            .iter()
                            .map(|directory| home.join(directory))
                            .collect()
                    });
                (runtime.id.clone(), ProjectlessPlaces { home, directories })
            })
            .collect()
    }

    /// The HOME a runtime's conversations see: the one it is given, else the
    /// app's on this Mac or the login's over SSH.
    fn user_home(&self, runtime: &RuntimeInstance) -> Option<PathBuf> {
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
                let mut command = std::process::Command::new("printenv");
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
    fn working_project(
        summary: &ConversationSummary,
        places: &BTreeMap<String, ProjectlessPlaces>,
    ) -> Option<String> {
        let unknown = ProjectlessPlaces::default();
        let places = places.get(&summary.runtime_id).unwrap_or(&unknown);
        summary
            .cwd
            .clone()
            .filter(|cwd| !places.contain(Path::new(cwd)))
    }

    fn marks(&self, id: &str) -> Option<&ConversationMarks> {
        self.organization
            .conversations
            .iter()
            .find(|marks| marks.conversation_id == id)
    }

    fn alias(&self, path: &str) -> Option<String> {
        self.organization
            .project_aliases
            .iter()
            .find(|alias| alias.path == path)
            .map(|alias| alias.alias.clone())
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

    fn listed(
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
            conversation: summary.clone(),
            native_id: self.native_segment(summary).1,
            pinned: !archived && marks.is_some_and(|m| m.pinned_at_unix_ms.is_some()),
            unread: !archived
                && (marks.is_some_and(|m| m.marked_unread)
                    || summary.updated_at_unix_ms.is_some_and(|at| at > seen)),
            archived,
            moved: moved.is_some(),
            failed: false,
            project: moved.or_else(|| Self::working_project(summary, places)),
            group: None,
        }
    }

    /// The window over `sources` with every conversation it shows read.
    /// Reading gives a conversation its native activity time, which may move
    /// it out of the window and others in, so the window is drawn again
    /// until it settles; whatever is still unread then is left out.
    pub(super) fn browse_read(&self, mut sources: Vec<ListedSource>, search: Option<&str>) -> Page {
        let mut rounds = 0;
        loop {
            let mut page = self.browse(sources, search);
            let unread = |item: &ListedConversation| {
                self.last_listed
                    .borrow()
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
            let mut failed = false;
            for (runtime_id, ids) in &wanted {
                let read = self
                    .runtime_instance(runtime_id)
                    .and_then(|runtime| self.read_indexed(runtime, ids));
                if let Err(error) = read {
                    failed = true;
                    tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="list_window",runtime_id=%runtime_id,detail=%error);
                }
            }
            if failed || rounds > 4 {
                page.conversations.retain(|item| !unread(item));
                return page;
            }
            sources = self.cached_sources();
        }
    }

    /// The list's window over `sources`: the pinned group, then the groups by
    /// recent activity, each loading its pages while the list's total lasts
    /// and any further page the user asked for. A search loads every match
    /// up to the total instead.
    pub(super) fn browse(&self, sources: Vec<ListedSource>, search: Option<&str>) -> Page {
        let preferences = &self.conversation_browser_preferences;
        let archived_view = self.browser_window.archived;
        let places = self.projectless_places();
        let all: Vec<ListedConversation> = sources
            .iter()
            .map(|source| self.listed(source, &places))
            .collect();
        *self.last_listed.borrow_mut() = sources
            .into_iter()
            .map(|source| (source.summary.id.clone(), source))
            .collect();
        let mut projects = BTreeMap::<String, Option<i64>>::new();
        for item in all.iter().filter(|item| {
            item.conversation.parent_conversation_id.is_none() && item.archived == archived_view
        }) {
            if let Some(path) = &item.project {
                let latest = projects.entry(path.clone()).or_default();
                *latest = (*latest).max(item.conversation.updated_at_unix_ms);
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
                    contains(path) || self.alias(path).is_some_and(|alias| contains(&alias))
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
        let page = self.conversation_browser_limits.page as usize;
        let mut remaining = self.conversation_browser_limits.total as usize;
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
                let first = if key == ConversationGroupKey::All {
                    remaining
                } else {
                    page.min(remaining)
                }
                .min(indices.len());
                remaining -= first;
                indices.len().min(first + page * extra)
            };
            for index in &indices[..loaded] {
                let mut item = all[*index].clone();
                item.group = Some(key.clone());
                conversations.push(item);
            }
            descriptors.push(ConversationGroup {
                alias: match &key {
                    ConversationGroupKey::Project { path: Some(path) } => self.alias(path),
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
            .map(|(path, latest)| ConversationProject {
                alias: self.alias(&path),
                path,
                latest_activity_unix_ms: latest,
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
            conversations,
            groups: descriptors,
            projects,
        }
    }

    pub(super) fn search_conversations(&self, request: &Value) -> Result<Value, RuntimeError> {
        let text = request["payload"]["text"]
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| RuntimeError::invalid("搜索文字不能为空"))?;
        // Searching follows typing, so it searches the latest index rather
        // than indexing the runtimes again; titles need every session read.
        for runtime in self.runtime_instances.iter().filter(|r| r.enabled) {
            if let Err(error) = self.read_unread(runtime) {
                tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="search",runtime_id=%runtime.id,runtime_type_id=%runtime.type_id,detail=%error);
            }
        }
        let page = self.browse_read(self.cached_sources(), Some(text));
        Ok(serde_json::to_value(ConversationSearch {
            conversations: page.conversations,
            groups: page.groups,
        })?)
    }

    pub(super) fn find_conversation(&self, request: &Value) -> Result<Value, RuntimeError> {
        let id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        if let Some(source) = self
            .last_listed
            .borrow()
            .get(id)
            .filter(|source| !source.provisional)
        {
            return Ok(serde_json::to_value(
                self.listed(source, &self.projectless_places()),
            )?);
        }
        let linked = self.conversation_links.iter().any(|link| link.id == id);
        let owner = self
            .runtime_instances
            .iter()
            .find(|r| r.enabled && id.starts_with(&format!("{}:", r.id)));
        let found = match owner {
            Some(runtime) if !linked => {
                self.conversation_summary(runtime, id)?
                    .map(|summary| ListedSource {
                        summary,
                        archived_natively: false,
                        provisional: false,
                    })
            }
            _ => None,
        }
        .or_else(|| {
            let (sources, _) = self.list_sources(false).ok()?;
            sources
                .into_iter()
                .find(|source| source.summary.id == id && !source.provisional)
        })
        .or_else(|| {
            self.natively_archived()
                .into_iter()
                .find(|summary| summary.id == id)
                .map(|summary| ListedSource {
                    summary,
                    archived_natively: true,
                    provisional: false,
                })
        });
        Ok(serde_json::to_value(found.as_ref().map(|source| {
            self.listed(source, &self.projectless_places())
        }))?)
    }

    pub(super) fn browser_window_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let payload = &request["payload"];
        match payload["operation"].as_str() {
            Some("loadMore") => {
                let key: ConversationGroupKey = serde_json::from_value(payload["group"].clone())?;
                *self.browser_window.extra_pages.entry(key).or_default() += 1;
            }
            Some("showArchived") => {
                let archived = payload["archived"]
                    .as_bool()
                    .ok_or_else(|| RuntimeError::invalid("archived view"))?;
                if archived != self.browser_window.archived {
                    self.browser_window = BrowserWindow {
                        archived,
                        ..Default::default()
                    };
                }
            }
            _ => return Err(RuntimeError::invalid("browser window operation")),
        }
        self.list()
    }

    /// New grouping, sorting, or filters start the list from its first pages.
    pub(super) fn reset_browser_pages(&mut self) {
        self.browser_window.extra_pages.clear();
    }

    pub(super) fn organization_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let references: Vec<ConversationReference> =
            serde_json::from_value(request["payload"]["conversations"].clone())?;
        let change: ConversationOrganizationChange =
            serde_json::from_value(request["payload"]["change"].clone())?;
        if references.is_empty() {
            return Err(RuntimeError::invalid("没有选择会话"));
        }
        let places = self.projectless_places();
        let mut targets = Vec::new();
        for reference in &references {
            let listed = self.last_listed.borrow();
            let source = listed
                .get(&reference.conversation_id)
                .filter(|source| source.summary.runtime_id == reference.runtime_id)
                .ok_or_else(|| RuntimeError::invalid("会话列表中没有这个会话，请刷新后重试"))?;
            targets.push((
                source.summary.clone(),
                source.archived_natively,
                self.listed(source, &places),
            ));
        }
        let mut next = self.organization.clone();
        let now = unix_ms_now();
        for (summary, archived_natively, listed) in &targets {
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
                    marks.project = path
                        .as_ref()
                        .filter(|path| {
                            Some(path.as_str())
                                != Self::working_project(summary, &places).as_deref()
                        })
                        .map(|path| {
                            if Path::new(path).is_absolute() {
                                Ok(path.clone())
                            } else {
                                Err(RuntimeError::invalid("项目路径必须是绝对路径"))
                            }
                        })
                        .transpose()?;
                }
            }
        }
        next.conversations.retain(|marks| !marks.is_empty());
        self.store_organization(next)?;
        let listing = self.list()?;
        // The list read again may already show newer activity than the mark.
        let listed = self.last_listed.borrow();
        for (summary, _, _) in &targets {
            let now = listed
                .get(&summary.id)
                .map(|source| self.listed(source, &places));
            tracing::info!(target:"velune_application", event="conversation_organized", change=?change, conversation_id=%summary.id, runtime_id=%summary.runtime_id, marked_activity_unix_ms=?summary.updated_at_unix_ms, seen_activity_unix_ms=?self.marks(&summary.id).and_then(|marks| marks.seen_activity_unix_ms), listed_activity_unix_ms=?now.as_ref().and_then(|item| item.conversation.updated_at_unix_ms), unread=?now.as_ref().map(|item| item.unread));
        }
        Ok(listing)
    }

    pub(super) fn project_alias_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let path = request["payload"]["path"]
            .as_str()
            .filter(|path| Path::new(path).is_absolute())
            .ok_or_else(|| RuntimeError::invalid("项目路径必须是绝对路径"))?;
        let alias = request["payload"]["alias"]
            .as_str()
            .map(str::trim)
            .filter(|alias| !alias.is_empty());
        let mut next = self.organization.clone();
        next.project_aliases.retain(|entry| entry.path != path);
        if let Some(alias) = alias {
            next.project_aliases.push(crate::config::ProjectAlias {
                path: path.into(),
                alias: alias.into(),
            });
        }
        self.store_organization(next)?;
        self.list()
    }

    /// A deleted conversation keeps no organization.
    pub(super) fn forget_conversation(&mut self, id: &str) {
        if !self
            .organization
            .conversations
            .iter()
            .any(|m| m.conversation_id == id)
        {
            return;
        }
        let mut next = self.organization.clone();
        next.conversations
            .retain(|marks| marks.conversation_id != id);
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

    fn store_organization(
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
                .and_then(|runtime| velune_agent_runtime::version::variant(&runtime.type_id))
                .is_some_and(|variant| variant.can_archive_conversations)
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
