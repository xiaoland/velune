//! In-memory evidence connecting native executions to visible messages.
use crate::{Message, MessageIdentityConfirmation, TurnOutcome};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionKind {
    UserTurn,
    ContextMaintenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionRecord {
    pub id: String,
    pub kind: ExecutionKind,
    pub message_ids: Vec<String>,
    pub outcome: Option<TurnOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Activity {
    pub id: String,
    pub execution_id: String,
    /// An ended activity may have no known outcome. Idle alone proves no success.
    pub ended: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Executions {
    pub records: Vec<ExecutionRecord>,
    pub current: Option<Activity>,
}

impl Executions {
    pub fn begin(&mut self, kind: ExecutionKind, resume: bool) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = format!("activity-{time:x}-{sequence:x}");
        let execution_id = if resume {
            self.records
                .iter_mut()
                .rev()
                .find(|record| record.kind == kind)
                .map(|record| {
                    record.outcome = None;
                    record.id.clone()
                })
        } else {
            None
        }
        .unwrap_or_else(|| {
            self.records.push(ExecutionRecord {
                id: id.clone(),
                kind,
                message_ids: Vec::new(),
                outcome: None,
            });
            id.clone()
        });
        self.current = Some(Activity {
            id,
            execution_id,
            ended: false,
        });
    }

    pub fn attach(&mut self, message_id: &str) {
        if let Some(activity) = &self.current
            && !activity.ended
            && let Some(record) = self
                .records
                .iter_mut()
                .find(|r| r.id == activity.execution_id)
            && !record.message_ids.iter().any(|id| id == message_id)
        {
            record.message_ids.push(message_id.into());
        }
    }

    /// Once per explicit activity; shutdown of an idle connection cannot rewrite it.
    pub fn finish(&mut self, outcome: Option<TurnOutcome>) {
        if let Some(activity) = &mut self.current
            && !activity.ended
        {
            if let Some(record) = self
                .records
                .iter_mut()
                .find(|r| r.id == activity.execution_id)
            {
                record.outcome = outcome;
            }
            activity.ended = true;
        }
    }

    pub fn record(&self, message_id: &str) -> Option<&ExecutionRecord> {
        self.records
            .iter()
            .find(|r| r.message_ids.iter().any(|id| id == message_id))
    }

    pub fn confirm(&mut self, confirmations: &[MessageIdentityConfirmation]) {
        let ids: BTreeMap<_, _> = confirmations
            .iter()
            .map(|c| (c.previous_id.as_str(), c.current_id.as_str()))
            .collect();
        for record in &mut self.records {
            for id in &mut record.message_ids {
                if let Some(current) = ids.get(id.as_str()) {
                    *id = (*current).into();
                }
            }
            let mut seen = BTreeSet::new();
            record.message_ids.retain(|id| seen.insert(id.clone()));
        }
    }

    pub fn retain(&mut self, messages: &[Message]) {
        let ids: BTreeSet<_> = messages.iter().map(|m| m.id.as_str()).collect();
        for record in &mut self.records {
            record.message_ids.retain(|id| ids.contains(id.as_str()));
        }
        self.records.retain(|r| {
            !r.message_ids.is_empty()
                || self
                    .current
                    .as_ref()
                    .is_some_and(|a| !a.ended && a.execution_id == r.id)
        });
        if self
            .current
            .as_ref()
            .is_some_and(|a| !self.records.iter().any(|r| r.id == a.execution_id))
        {
            self.current = None;
        }
    }

    /// Preserve observed evidence when the same native source is prepared again.
    /// No old activity becomes active in the replacement connection.
    pub fn restore(&mut self, previous: &Self, messages: &[Message]) {
        let mut observed = previous.clone();
        observed.current = None;
        observed.retain(messages);
        let claimed: BTreeSet<_> = observed
            .records
            .iter()
            .flat_map(|r| r.message_ids.iter().cloned())
            .collect();
        for record in &mut self.records {
            record.message_ids.retain(|id| !claimed.contains(id));
        }
        self.records.retain(|r| !r.message_ids.is_empty());
        self.records.extend(observed.records);
        let positions: BTreeMap<_, _> = messages
            .iter()
            .enumerate()
            .map(|(index, message)| (message.id.as_str(), index))
            .collect();
        self.records.sort_by_key(|record| {
            record
                .message_ids
                .iter()
                .filter_map(|id| positions.get(id.as_str()).copied())
                .min()
                .unwrap_or(usize::MAX)
        });
        self.current = None;
    }

    /// Native history has a genuine boundary, but does not activate an operation.
    pub fn history(
        &mut self,
        id: String,
        kind: ExecutionKind,
        message_ids: Vec<String>,
        outcome: Option<TurnOutcome>,
    ) {
        self.records.push(ExecutionRecord {
            id,
            kind,
            message_ids,
            outcome,
        });
    }

    pub fn map_ids(&mut self, map: &impl Fn(&str) -> String) {
        for record in &mut self.records {
            record.id = map(&record.id);
            for id in &mut record.message_ids {
                *id = map(id);
            }
        }
        if let Some(activity) = &mut self.current {
            activity.execution_id = map(&activity.execution_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_continue_has_new_activity_but_steer_and_identity_share_scope() {
        let mut executions = Executions::default();
        executions.begin(ExecutionKind::UserTurn, false);
        let first = executions.current.clone().unwrap();
        executions.attach("u1");
        executions.attach("work");
        executions.attach("u2");
        executions.finish(Some(TurnOutcome::Cancelled));
        executions.finish(Some(TurnOutcome::Completed));
        assert_eq!(
            executions.record("u1").unwrap().outcome,
            Some(TurnOutcome::Cancelled)
        );
        executions.confirm(&[MessageIdentityConfirmation {
            previous_id: "u1".into(),
            current_id: "native-u1".into(),
        }]);
        executions.map_ids(&|id| format!("segment-1:{id}"));
        executions.begin(ExecutionKind::UserTurn, true);
        assert_ne!(executions.current.as_ref().unwrap().id, first.id);
        assert_eq!(
            executions.current.as_ref().unwrap().execution_id,
            format!("segment-1:{}", first.execution_id)
        );
        assert_eq!(executions.record("segment-1:u2").unwrap().outcome, None);
        executions.finish(Some(TurnOutcome::Completed));
        assert_eq!(
            executions.record("segment-1:native-u1").unwrap().outcome,
            Some(TurnOutcome::Completed)
        );
        assert_eq!(
            executions.record("segment-1:u2").unwrap().outcome,
            Some(TurnOutcome::Completed)
        );
    }
}
