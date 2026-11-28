//! Data-only receive ownership. Keys name actual native payload owners, not events.
use super::*;
use std::collections::{BTreeMap, HashSet, VecDeque};

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum DataOwner {
    History,
    Message(String),
    Interaction(String),
    ResolvedQuestions,
    Metadata,
    Identity,
    Selection,
    Provider,
    Instructions,
    Catalog,
    Turn,
    TurnError,
    Thread(String),
    Task(String),
    #[allow(
        dead_code,
        reason = "Claude task issues have no registered receive owner yet"
    )]
    TaskIssue,
    Stream,
    SideHistory,
    Title,
    TitleHidden,
    Handoff,
    HandoffHidden,
    SubAgents,
}

#[derive(Default)]
pub(super) struct NativeData {
    current: Option<DataRetention>,
    owners: BTreeMap<DataOwner, DataRetention>,
}
impl NativeData {
    // Split only pieces of one bounded projection. Deep copies escaping that
    // projection must call try_clone before allocating their payload instead.
    pub(super) fn branch(&mut self) -> DataRetention {
        let Some(current) = self.current.take() else {
            // Native file imports and local commands do not originate in RPC.
            return DataRetention::default();
        };
        let (current, piece) = current.split();
        self.current = Some(current);
        piece
    }
    pub(super) fn retain(&mut self, owner: DataOwner) {
        let piece = self.branch();
        let old = self.owners.remove(&owner).unwrap_or_default();
        self.owners.insert(owner, old.merge(piece));
    }
    pub(super) fn replace(&mut self, owner: DataOwner) {
        let piece = self.branch();
        self.owners.insert(owner, piece);
    }
    pub(super) fn merge_owner(&mut self, owner: DataOwner, piece: DataRetention) {
        let old = self.owners.remove(&owner).unwrap_or_default();
        self.owners.insert(owner, old.merge(piece));
    }
    pub(super) fn move_owner(&mut self, from: DataOwner, to: DataOwner) {
        if let Some(piece) = self.owners.remove(&from) {
            let old = self.owners.remove(&to).unwrap_or_default();
            self.owners.insert(to, old.merge(piece));
        }
    }
    pub(super) fn insert(&mut self, owner: DataOwner, piece: DataRetention) {
        self.owners.insert(owner, piece);
    }
    pub(super) fn try_clone_current(&self) -> Result<DataRetention> {
        self.current.as_ref().map_or_else(
            || Ok(DataRetention::default()),
            |retention| retention.try_clone().map_err(retention_error),
        )
    }
    pub(super) fn try_clone(&self, include: impl Fn(&DataOwner) -> bool) -> Result<DataRetention> {
        let mut result = DataRetention::default();
        for (owner, retention) in &self.owners {
            if include(owner) {
                result = result.merge(retention.try_clone().map_err(retention_error)?);
            }
        }
        Ok(result)
    }
}
pub(super) fn command_data_owner(owner: &DataOwner) -> bool {
    matches!(
        owner,
        DataOwner::Identity | DataOwner::Turn | DataOwner::Thread(_) | DataOwner::Interaction(_)
    )
}
fn snapshot_owner(owner: &DataOwner) -> bool {
    matches!(
        owner,
        DataOwner::History
            | DataOwner::Message(_)
            | DataOwner::Interaction(_)
            | DataOwner::Metadata
            | DataOwner::Identity
            | DataOwner::SubAgents
    )
}
pub(super) fn retention_error(error: std::io::Error) -> Error {
    Error::with_code(error.to_string(), "resource_exhausted")
}
impl NativeSession {
    pub(super) fn receive_native_record(
        &mut self,
        record: Retained<Value>,
        handle: impl FnOnce(&mut Self, Value) -> Result<()>,
    ) -> Result<()> {
        let (value, retention) = record.into_parts();
        let previous = self.data.current.replace(retention);
        let result = handle(self, value);
        self.data.current = previous;
        self.prune_native_data();
        result
    }

    pub(super) fn prune_native_data(&mut self) {
        let snapshot = self.snapshot.as_ref();
        let interactions = &self.interactions;
        let codex = &self.codex;
        let claude = &self.claude;
        // Each record may leave thousands of message owners live. Index the
        // borrowed IDs once per prune, and skip building either index when
        // there is no corresponding owner (e.g. cold paginated history).
        let mut message_ids = None;
        let mut pending_ids = None;
        self.data.owners.retain(|owner, _| match owner {
            DataOwner::Message(id) => snapshot.is_some_and(|snapshot| {
                message_ids
                    .get_or_insert_with(|| {
                        snapshot
                            .messages
                            .iter()
                            .map(|message| message.id.as_str())
                            .collect::<HashSet<_>>()
                    })
                    .contains(id.as_str())
            }),
            DataOwner::Interaction(id) => {
                interactions.contains_key(id)
                    || snapshot.is_some_and(|snapshot| {
                        pending_ids
                            .get_or_insert_with(|| {
                                snapshot
                                    .pending_interactions
                                    .iter()
                                    .map(|interaction| interaction.id.as_str())
                                    .collect::<HashSet<_>>()
                            })
                            .contains(id.as_str())
                    })
            }
            DataOwner::History
            | DataOwner::Metadata
            | DataOwner::Identity
            | DataOwner::SubAgents => snapshot.is_some(),
            DataOwner::ResolvedQuestions => !codex.resolved_async_questions.is_empty(),
            DataOwner::Turn => self.turn_id.is_some(),
            DataOwner::TurnError => codex.turn_error.is_some(),
            DataOwner::Thread(id) => codex.threads.contains_key(id),
            DataOwner::Task(id) => {
                claude.task_infos.contains_key(id) || codex.terminals.contains(id)
            }
            DataOwner::TaskIssue => claude.task_issue.is_some(),
            DataOwner::Stream => claude.stream_id.is_some() || claude.context.is_some(),
            DataOwner::Catalog => !self.available_models.is_empty(),
            DataOwner::Selection => self.native_selection.is_some() || codex.service_tier.is_some(),
            DataOwner::SideHistory => !claude.side_history.is_empty(),
            DataOwner::Title => codex.title.pending(),
            DataOwner::TitleHidden => codex.title.retains_data(),
            DataOwner::Handoff => codex.handoff_summary.is_some(),
            DataOwner::HandoffHidden => !codex.handoff_summary_hidden.is_empty(),
            _ => true,
        });
    }

    pub(super) fn clear_snapshot_data(&mut self) {
        self.data.owners.retain(|owner, _| !snapshot_owner(owner));
    }

    /// Guard-aware deep snapshot copy for Rust live consumers. Reserve before
    /// cloning; borrowed snapshot() is for inspection, not a cache boundary.
    pub fn owned_snapshot(&self) -> Result<Retained<ConversationSnapshot>> {
        self.retained_snapshot()?
            .ok_or_else(|| Error::new("没有活动会话"))
    }
    pub fn retained_snapshot(&self) -> Result<Option<Retained<ConversationSnapshot>>> {
        let Some(snapshot) = &self.snapshot else {
            return Ok(None);
        };
        let retention = self.data.try_clone(snapshot_owner)?;
        Ok(Some(Retained::from_parts(snapshot.clone(), retention)))
    }

    pub(super) fn install_history_data(&mut self, retention: DataRetention) {
        if self.snapshot.is_some() {
            // A cold projection derives messages, interactions and metadata from
            // the same pages. Keep their reservation once for this history base.
            // Per-message owners account only for data acquired after installation.
            self.data.insert(DataOwner::History, retention);
        }
    }
}

// Preserve DeferredRecords' existing count/byte gates; the parallel deque has
// only guards, never another copy of the JSON bodies.
#[derive(Default)]
pub(super) struct RetainedRecords {
    values: crate::transport::DeferredRecords,
    retention: VecDeque<DataRetention>,
}
impl RetainedRecords {
    pub(super) fn push(&mut self, record: Retained<Value>) -> std::io::Result<()> {
        let (value, retention) = record.into_parts();
        self.values.push(value)?;
        self.retention.push_back(retention);
        Ok(())
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &Value> {
        self.values.iter()
    }
    pub(super) fn drain_batch(&mut self, maximum: usize) -> Vec<Retained<Value>> {
        self.values
            .drain_batch(maximum)
            .into_iter()
            .map(|value| {
                Retained::from_parts(
                    value,
                    self.retention.pop_front().expect("receive ownership"),
                )
            })
            .collect()
    }
}
