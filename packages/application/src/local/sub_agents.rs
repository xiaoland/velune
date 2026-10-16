//! A conversation's native sub-agents: what its snapshot shows of them, and
//! which conversation the running ones belong to.
use super::*;
use crate::conversation::{ConversationSnapshot, MessageBlock, SubAgent};

impl CoreRuntime {
    /// The native sub-agents of `snapshot`'s conversation: those its
    /// transcript links, in order, then the others its runtime lists with it
    /// as parent. A listed one takes its listed title.
    pub(super) fn sub_agents(&self, snapshot: &ConversationSnapshot) -> Vec<SubAgent> {
        let listed = self.last_listed.borrow();
        let failures = self.sub_agent_failures.borrow();
        let mut found = Vec::<SubAgent>::new();
        let mut add = |id: &str, title: &str| {
            if found.iter().any(|known| known.conversation_id == id) {
                return;
            }
            found.push(SubAgent {
                conversation_id: id.to_owned(),
                title: listed
                    .get(id)
                    .map_or(title, |source| source.summary.title.display_text())
                    .to_owned(),
                failed: failures.get(id).copied().unwrap_or(false),
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
            .last_listed
            .borrow()
            .get(id)
            .and_then(|source| source.summary.parent_conversation_id.clone())
        {
            return Some(parent);
        }
        std::iter::once(&self.session)
            .chain(&self.parked)
            .filter_map(|session| session.projected())
            .find(|view| {
                view.messages
                    .iter()
                    .flat_map(|message| &message.blocks)
                    .any(|block| {
                        matches!(block, MessageBlock::Tool { conversation_id: Some(linked), .. } if linked == id)
                    })
            })
            .map(|view| view.conversation.id.clone())
    }
}
