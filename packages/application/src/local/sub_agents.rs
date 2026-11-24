//! A conversation's native sub-agents: what its snapshot shows of them, and
//! which conversation the running ones belong to.
use super::*;
use crate::conversation::{ConversationSnapshot, MessageBlock, SubAgent};
use std::{cmp::Reverse, collections::HashMap};

impl CoreRuntime {
    /// The native sub-agents of this conversation, newest first.
    /// Native work descriptions take priority over session titles.
    pub(super) fn sub_agents(&self, snapshot: &ConversationSnapshot) -> Vec<SubAgent> {
        let listed = self.listed_sources();
        let failures = self.sub_agent_failures.borrow();
        // Runtime descriptors carry the native role/task name. Session names
        // belong to the child tab and need not describe its delegated work.
        let native = &snapshot.sub_agents;
        let mut found = Vec::<SubAgent>::new();
        let mut add = |id: &str, title: &str| {
            if found.iter().any(|known| known.conversation_id == id) {
                return;
            }
            found.push(SubAgent {
                conversation_id: id.to_owned(),
                title: native
                    .iter()
                    .find(|agent| agent.conversation_id == id && !agent.title.trim().is_empty())
                    .map(|agent| agent.title.as_str())
                    .or_else(|| {
                        listed
                            .get(id)
                            .filter(|source| {
                                !matches!(
                                    source.summary.title,
                                    crate::conversation::ConversationTitle::Untitled
                                )
                            })
                            .map(|source| source.summary.title.display_text())
                    })
                    .unwrap_or(title)
                    .to_owned(),
                failed: native
                    .iter()
                    .any(|agent| agent.conversation_id == id && agent.failed)
                    || failures.get(id).copied().unwrap_or(false),
            });
        };
        let mut first_link_times = HashMap::new();
        for message in &snapshot.messages {
            for block in &message.blocks {
                if let MessageBlock::Tool {
                    title,
                    conversation_id: Some(id),
                    ..
                } = block
                {
                    if let Some(time) = message.timestamp_unix_ms {
                        first_link_times.entry(id.as_str()).or_insert(time);
                    }
                    add(id, title);
                }
            }
        }
        for agent in native {
            add(&agent.conversation_id, &agent.title);
        }
        for session in std::iter::once(&self.session).chain(&self.parked) {
            if session.conversation_id() == Some(snapshot.conversation.id.as_str()) {
                if let Ok(work) = session.work() {
                    for id in work
                        .iter()
                        .filter_map(|task| task.conversation_id.as_deref())
                    {
                        add(id, "子 Agent");
                    }
                }
                if let Some(live) = session.active_state.live()
                    && let Ok(background) = live.background_tasks()
                {
                    for task in &background.tasks {
                        if let Some(id) = task.conversation_id.as_deref() {
                            add(id, &task.title);
                        }
                    }
                }
            }
        }
        let children = listed
            .values()
            .map(|source| &source.summary)
            .filter(|summary| {
                summary.parent_conversation_id.as_deref() == Some(&snapshot.conversation.id)
            });
        for child in children {
            add(&child.id, child.title.display_text());
        }
        if found.len() > 1 {
            let open_creation_times: HashMap<_, _> = std::iter::once(&self.session)
                .chain(&self.parked)
                .filter_map(session::Session::projected)
                .filter_map(|view| {
                    let conversation = &view.conversation;
                    Some((conversation.id.as_str(), conversation.created_at_unix_ms?))
                })
                .collect();
            // Keep equal and unknown dates in reverse discovery order. An
            // unfinished child may be known only from its parent's transcript.
            found.reverse();
            found.sort_by_cached_key(|agent| {
                let id = agent.conversation_id.as_str();
                Reverse(
                    listed
                        .get(id)
                        .and_then(|source| source.summary.created_at_unix_ms)
                        .or_else(|| open_creation_times.get(id).copied())
                        .or_else(|| first_link_times.get(id).copied()),
                )
            });
        }
        found
    }

    /// The conversation a native sub-agent belongs to, as listed or as linked
    /// from an open conversation's transcript.
    pub(super) fn sub_agent_parent(&self, id: &str) -> Option<String> {
        if let Some(parent) = self
            .listed_sources()
            .get(id)
            .and_then(|source| source.summary.parent_conversation_id.clone())
        {
            return Some(parent);
        }
        if let Some(parent) = std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| {
                session.work().is_ok_and(|work| {
                    work.iter()
                        .any(|task| task.conversation_id.as_deref() == Some(id))
                }) || session.active_state.live().is_some_and(|live| {
                    live.background_tasks().is_ok_and(|background| {
                        background
                            .tasks
                            .iter()
                            .any(|task| task.conversation_id.as_deref() == Some(id))
                    })
                })
            })
        {
            return parent.conversation_id().map(str::to_owned);
        }
        std::iter::once(&self.session)
            .chain(&self.parked)
            .filter_map(|session| session.projected())
            .find(|view| {
                view.sub_agents.iter().any(|agent| agent.conversation_id == id)
                    || view.messages
                    .iter()
                    .flat_map(|message| &message.blocks)
                    .any(|block| {
                        matches!(block, MessageBlock::Tool { conversation_id: Some(linked), .. } if linked == id)
                    })
            })
            .map(|view| view.conversation.id.clone())
    }

    /// A child's live work is tracked by the runtime that spawned it. A
    /// history read of the child alone may be idle even before its first write.
    pub(super) fn child_run_state(&self, snapshot: &ConversationSnapshot) -> Option<RunState> {
        snapshot.conversation.parent_conversation_id.as_ref()?;
        let child_id = snapshot.conversation.id.as_str();
        let state = self
            .parked
            .iter()
            .filter_map(|session| session.work().ok())
            .find_map(|work| {
                work.iter()
                    .find(|task| task.conversation_id.as_deref() == Some(child_id))
                    .map(|task| task.state)
            })?;
        Some(match state {
            velune_agent_runtime::work::BackgroundState::Stopping => RunState::Stopping,
            _ => RunState::Elsewhere,
        })
    }
}

impl CoreRuntime {
    pub(super) fn apply_child_control(&self, snapshot: &mut ConversationSnapshot) {
        let Some(parent_id) = snapshot.conversation.parent_conversation_id.as_deref() else {
            return;
        };
        let Some(parent) = self
            .parked
            .iter()
            .find(|session| session.conversation_id() == Some(parent_id))
        else {
            return;
        };
        let child = &snapshot.conversation.id;
        let state = parent.work().ok().and_then(|work| {
            work.iter()
                .find(|task| task.conversation_id.as_ref() == Some(child))
                .map(|task| task.state)
        });
        if let Some(state) = state {
            let stopping = state == velune_agent_runtime::work::BackgroundState::Stopping;
            let controllable = parent
                .active_state
                .live()
                .is_some_and(|live| live.can_stop_child(child));
            if controllable && !stopping {
                snapshot.run_state = RunState::Running;
            }
            snapshot.actions.can_cancel = controllable && !stopping;
        } else {
            snapshot.actions.can_cancel = false;
        }
    }
}

#[cfg(all(test, unix))]
mod activity_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sub_agent_descriptors_win_over_session_names_and_link_to_the_parent() {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-native-descriptors-{:x}",
            u64::from_le_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut app = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let summary = ConversationSummary {
            id: "pi:/synthetic/parent.jsonl".into(),
            runtime_id: "pi".into(),
            title: crate::conversation::ConversationTitle::Native("Parent".into()),
            cwd: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            parent_conversation_id: None,
            native_read_state: None,
            can_rename: false,
            can_delete: false,
        };
        let mut view = PiProjection::new(summary.clone()).snapshot.unwrap();
        view.sub_agents.push(SubAgent {
            conversation_id: "pi:/synthetic/child.jsonl".into(),
            title: "worker：验证缓存归属".into(),
            failed: true,
        });
        let mut child = summary;
        child.id = view.sub_agents[0].conversation_id.clone();
        child.title =
            crate::conversation::ConversationTitle::Native("subagent-worker-machine-id".into());
        child.parent_conversation_id = Some(view.conversation.id.clone());
        app.runtime.last_listed.borrow_mut().entries.insert(
            child.id.clone(),
            organization::ListedSource {
                summary: child.clone().into(),
                archived_natively: false,
                provisional: false,
            },
        );
        let result = app.runtime.sub_agents(&view);
        assert_eq!(result, view.sub_agents);
        assert_eq!(
            app.runtime
                .listed_sources()
                .get(&child.id)
                .unwrap()
                .summary
                .title,
            child.title
        );
        let mut legacy = view.clone();
        legacy.sub_agents.clear();
        assert_eq!(
            app.runtime.sub_agents(&legacy)[0].title,
            child.title.display_text()
        );
        app.runtime.last_listed.borrow_mut().entries.clear();
        app.runtime.session.active_state = ActiveState::History(Box::new(view.clone().into()));
        assert_eq!(
            app.runtime.sub_agent_parent(&child.id).as_deref(),
            Some(view.conversation.id.as_str())
        );
        assert_eq!(app.runtime.sub_agents(&view), result);
        assert!(
            app.runtime.active_conversations().is_empty(),
            "known terminal identities do not imply active work"
        );
        // Two children have native creation dates. The older one was last
        // active most recently and is also linked late in the transcript.
        let mut older = child.clone();
        older.created_at_unix_ms = Some(10);
        older.updated_at_unix_ms = Some(1000);
        let mut newer = older.clone();
        newer.id = "pi:/synthetic/newer.jsonl".into();
        newer.created_at_unix_ms = Some(40);
        for summary in [older.clone(), newer.clone()] {
            app.runtime.last_listed.borrow_mut().entries.insert(
                summary.id.clone(),
                organization::ListedSource {
                    summary: summary.into(),
                    archived_natively: false,
                    provisional: false,
                },
            );
        }
        let linked_only = "pi:/synthetic/linked.jsonl";
        let unknown = "pi:/synthetic/unknown.jsonl";
        for (id, time) in [
            (linked_only, Some(25)),
            (older.id.as_str(), Some(200)),
            (linked_only, Some(500)),
            (unknown, None),
        ] {
            view.messages.push(crate::conversation::Message {
                id: format!("link-{id}-{time:?}"),
                role: crate::conversation::MessageRole::Assistant,
                blocks: vec![MessageBlock::Tool {
                    tool_id: None,
                    title: "Agent".into(),
                    state: crate::conversation::ToolState::Completed,
                    output: None,
                    conversation_id: Some(id.into()),
                }],
                timestamp_unix_ms: time,
                completed: true,
                turn_settings: None,
                turn_usage: None,
                branch_point: None,
            });
        }
        assert_eq!(
            app.runtime
                .sub_agents(&view)
                .into_iter()
                .map(|agent| agent.conversation_id)
                .collect::<Vec<_>>(),
            [newer.id.as_str(), linked_only, older.id.as_str(), unknown]
        );
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
