//! The conversations a conversation's agent contacted through Velune, or
//! whose agents contacted it, as its transcript shows.
use super::*;
use crate::conversation::{
    Collaborator, ConversationSnapshot, MessageBlock, MessageRole, agent_message,
};

impl CoreRuntime {
    /// In order of first mention: a collaboration tool's result names the
    /// conversation it reported on, a received message the one it came from.
    /// A listed one takes its listed title and its runtime's name.
    pub(super) fn collaborators(&self, snapshot: &ConversationSnapshot) -> Vec<Collaborator> {
        let mut found = Vec::<Collaborator>::new();
        for message in &snapshot.messages {
            for block in &message.blocks {
                let (id, sender) = match block {
                    MessageBlock::Tool {
                        conversation_id: None,
                        output: Some(output),
                        ..
                    } => match delivery::collaboration_conversation(output) {
                        Some(id) => (id, None),
                        None => continue,
                    },
                    MessageBlock::Text { text } if message.role == MessageRole::User => {
                        match agent_message::split(text) {
                            Some((_, sender)) => (sender.conversation_id.clone(), Some(sender)),
                            None => continue,
                        }
                    }
                    _ => continue,
                };
                if id == snapshot.conversation.id {
                    continue;
                }
                let index = found
                    .iter()
                    .position(|known| known.conversation_id == id)
                    .unwrap_or_else(|| {
                        found.push(Collaborator {
                            conversation_id: id,
                            title: String::new(),
                            runtime_name: None,
                            contacted: false,
                            contacted_by: false,
                        });
                        found.len() - 1
                    });
                let entry = &mut found[index];
                match sender {
                    Some(sender) => {
                        entry.contacted_by = true;
                        entry.title = sender.title;
                        entry.runtime_name = Some(sender.runtime_name);
                    }
                    None => entry.contacted = true,
                }
            }
        }
        let listed = self.last_listed.borrow();
        for entry in &mut found {
            if let Some(source) = listed.get(&entry.conversation_id) {
                entry.title = source.summary.title.display_text().to_owned();
                if let Ok(runtime) = self.runtime_instance(&source.summary.runtime_id) {
                    entry.runtime_name = Some(runtime.name.clone());
                }
            }
        }
        found
    }
}
