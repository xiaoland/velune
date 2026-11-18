//! A conversation's native sub-agents: what its snapshot shows of them, and
//! which conversation the running ones belong to.
use super::*;
use crate::conversation::{ConversationSnapshot, MessageBlock, SubAgent};

impl CoreRuntime {
    /// The native sub-agents of `snapshot`'s conversation: those its
    /// transcript links, in order, then the others its runtime lists with it
    /// as parent. Native work descriptions take priority over session titles.
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
        for block in snapshot.messages.iter().flat_map(|message| &message.blocks) {
            if let MessageBlock::Tool {
                title,
                conversation_id: Some(id),
                ..
            } = block
            {
                add(id, title);
            }
        }
        for agent in native {
            add(&agent.conversation_id, &agent.title);
        }
        for session in std::iter::once(&self.session).chain(&self.parked) {
            if session.conversation_id() == Some(snapshot.conversation.id.as_str()) {
                for task in session.work() {
                    if let Some(id) = task.conversation_id {
                        add(&id, "子 Agent");
                    }
                }
                if let Some(live) = session.active_state.live() {
                    for task in live.background_tasks().tasks {
                        if let Some(id) = task.conversation_id {
                            add(&id, &task.title);
                        }
                    }
                }
            }
        }
        let mut children: Vec<_> = listed
            .values()
            .map(|source| &source.summary)
            .filter(|summary| {
                summary.parent_conversation_id.as_deref() == Some(&snapshot.conversation.id)
            })
            .collect();
        children.sort_by_key(|summary| summary.created_at_unix_ms);
        for child in children {
            add(&child.id, child.title.display_text());
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
                session
                    .work()
                    .iter()
                    .any(|task| task.conversation_id.as_deref() == Some(id))
                    || session.active_state.live().is_some_and(|live| {
                        live.background_tasks()
                            .tasks
                            .iter()
                            .any(|task| task.conversation_id.as_deref() == Some(id))
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
        let task = self
            .parked
            .iter()
            .flat_map(session::Session::work)
            .find(|task| task.conversation_id.as_deref() == Some(child_id))?;
        Some(match task.state {
            velune_agent_runtime::work::BackgroundState::Stopping => RunState::Stopping,
            _ => RunState::Elsewhere,
        })
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
                summary: child.clone(),
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
        app.runtime.session.active_state = ActiveState::History(Box::new(view.clone()));
        assert_eq!(
            app.runtime.sub_agent_parent(&child.id).as_deref(),
            Some(view.conversation.id.as_str())
        );
        assert_eq!(app.runtime.sub_agents(&view), result);
        assert!(
            app.runtime.active_conversations().is_empty(),
            "known terminal identities do not imply active work"
        );
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
