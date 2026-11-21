//! Disposable views of the Velune transcript. HAPI positions, watermarks and
//! optimistic-send correlation never change the application history.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use velune_application_api::conversation::{
    ApprovalOptionKind, ConversationSummary, InteractionKind, MessageRole, RunState,
};
use velune_application_api::{DeliveredBlock, DeliveredMessage, DeliveredSnapshot};

use super::super::session_control::one_shot_options;

const SESSION_CACHE_LIMIT: usize = 32;
const SEND_CACHE_LIMIT: usize = 1000;
const FIRST_LIVE_WINDOW: usize = 20;
const LIVE_WINDOW: usize = 200;

pub(super) struct History {
    inner: Mutex<HistoryInner>,
}

struct HistoryInner {
    sequence: u64,
    entries: HashMap<String, Entry>,
}

#[derive(Default)]
struct Entry {
    last_access: u64,
    epoch: u64,
    version: u64,
    positions: Vec<(String, Position)>,
    fields: Option<SessionFields>,
    sends: VecDeque<SendReceipt>,
}

struct SendReceipt {
    local_id: String,
    message_id: Option<String>,
}

#[derive(PartialEq, Eq)]
struct SessionFields {
    metadata: Value,
    created_at: i64,
    updated_at: i64,
    last_user_message_at: Option<i64>,
    active: bool,
    thinking: bool,
    requests: Map<String, Value>,
}

impl History {
    pub(super) fn new(epoch: u64) -> Self {
        Self {
            inner: Mutex::new(HistoryInner {
                sequence: epoch,
                entries: HashMap::new(),
            }),
        }
    }

    pub(super) fn note_send(&self, id: &str, local_id: String, message_id: Option<String>) {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.entry(id);
        if let Some(receipt) = entry
            .sends
            .iter_mut()
            .find(|receipt| receipt.local_id == local_id)
        {
            if message_id.is_some() {
                receipt.message_id = message_id;
            }
        } else {
            if entry.sends.len() == SEND_CACHE_LIMIT {
                entry.sends.pop_front();
            }
            entry.sends.push_back(SendReceipt {
                local_id,
                message_id,
            });
        }
    }

    pub(super) fn summary(&self, row: &ConversationSummary) -> Value {
        let inner = self.inner.lock().unwrap();
        let cached = inner
            .entries
            .get(&row.id)
            .and_then(|entry| entry.fields.as_ref().map(|fields| (fields, entry.version)));
        let (created_at, updated_at) = times(row);
        let (metadata, active, thinking, last_user, requests, version) = match cached {
            Some((fields, version)) => (
                fields.metadata.clone(),
                fields.active,
                fields.thinking,
                fields.last_user_message_at,
                Some(&fields.requests),
                version,
            ),
            None => (metadata(row), true, false, None, None, 0),
        };
        let pending = requests.into_iter().flatten().take(5).map(|(id, item)| {
            json!({"id": id, "kind": "permission", "tool": item["tool"], "since": item["createdAt"]})
        }).collect::<Vec<_>>();
        let count = requests.map_or(0, Map::len);
        json!({
            "id": row.id, "active": active, "thinking": thinking,
            "activeAt": updated_at, "updatedAt": updated_at, "createdAt": created_at,
            "lastUserMessageAt": last_user, "metadata": metadata, "hasConversationContent": true,
            "metadataVersion": version, "agentStateVersion": version, "todosUpdatedAt": 0,
            "pendingRequestsCount": count, "pendingRequests": pending,
            "pendingRequestKinds": if count == 0 { &[][..] } else { &["permission"][..] },
            "backgroundTaskCount": 0, "futureScheduledMessageCount": 0,
        })
    }

    pub(super) fn project(&self, id: &str, snapshot: DeliveredSnapshot) -> Projection {
        let (created_at, updated_at) = times(&snapshot.conversation);
        let requests = snapshot
            .pending_interactions
            .iter()
            .filter_map(|interaction| {
                let InteractionKind::Approval {
                    title,
                    detail,
                    options,
                } = &interaction.kind
                else {
                    return None;
                };
                let allows = one_shot_options(options)
                    .any(|option| option.approval_kind == Some(ApprovalOptionKind::AllowOnce));
                let denies = one_shot_options(options)
                    .any(|option| option.approval_kind == Some(ApprovalOptionKind::RejectOnce));
                if !allows || !denies {
                    return None;
                }
                Some((
                    interaction.id.clone(),
                    json!({
                        "tool": title, "createdAt": updated_at,
                        "arguments": {"description": detail}
                    }),
                ))
            })
            .collect::<Map<_, _>>();
        let thinking = matches!(snapshot.run_state, RunState::Running | RunState::Stopping);
        let active =
            snapshot.actions.can_send || snapshot.actions.can_cancel || !requests.is_empty();
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.entry(id);
        let version = entry.last_access;
        for confirmation in snapshot.message_identity_confirmations {
            for receipt in &mut entry.sends {
                if receipt.message_id.as_deref() == Some(confirmation.previous_id.as_str()) {
                    receipt.message_id = Some(confirmation.current_id.clone());
                }
            }
        }
        let local_ids = entry
            .sends
            .iter()
            .filter_map(|receipt| Some((receipt.message_id.as_deref()?, receipt.local_id.as_str())))
            .collect::<HashMap<_, _>>();
        let pending = entry
            .sends
            .iter()
            .map(|receipt| (receipt.local_id.clone(), receipt.message_id.is_some()))
            .collect::<HashMap<_, _>>();
        let mut at = created_at;
        let mut rows = Vec::new();
        for message in snapshot.messages {
            let Some(content) = content(&message) else {
                continue;
            };
            let seq = rows.len() + 1;
            at = at.max(message.timestamp_unix_ms.unwrap_or(at));
            let user = message.role == MessageRole::User;
            let local_id = local_ids
                .get(message.id.as_str())
                .filter(|_| user)
                .map(|id| id.to_string());
            rows.push(Message {
                id: message.id,
                seq,
                local_id,
                content,
                created_at: at,
                invoked_at: user.then_some(at),
                stamp: message.stamp,
            });
        }
        let positions = rows
            .iter()
            .map(|row| (row.id.clone(), row.position()))
            .collect::<Vec<_>>();
        if entry.epoch == 0 || !positions.starts_with(&entry.positions) {
            entry.epoch = version;
        }
        entry.positions = positions;
        let fields = SessionFields {
            metadata: metadata(&snapshot.conversation),
            created_at,
            updated_at,
            last_user_message_at: rows.iter().filter_map(|row| row.invoked_at).next_back(),
            active,
            thinking,
            requests,
        };
        if entry.fields.as_ref() != Some(&fields) {
            entry.version = version;
            entry.fields = Some(fields);
        }
        let fields = entry.fields.as_ref().unwrap();
        let session = json!({
            "id": id, "namespace": "velune", "seq": entry.version,
            "createdAt": fields.created_at, "updatedAt": fields.updated_at, "activeAt": fields.updated_at,
            "active": fields.active, "thinking": fields.thinking, "thinkingAt": fields.updated_at,
            "lastUserMessageAt": fields.last_user_message_at, "hasConversationContent": true,
            "metadata": fields.metadata, "metadataVersion": entry.version,
            "agentState": {"controlledByUser": false, "startingMode": "remote", "requests": fields.requests},
            "agentStateVersion": entry.version
        });
        Projection {
            session,
            epoch: entry.epoch,
            rows,
            pending,
        }
    }
}

impl HistoryInner {
    fn entry(&mut self, id: &str) -> &mut Entry {
        if self.entries.len() == SESSION_CACHE_LIMIT && !self.entries.contains_key(id) {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(id, _)| id.clone())
                .unwrap();
            self.entries.remove(&oldest);
        }
        self.sequence += 1;
        let entry = self.entries.entry(id.to_owned()).or_default();
        entry.last_access = self.sequence;
        entry
    }
}

fn times(row: &ConversationSummary) -> (i64, i64) {
    let created = row.created_at_unix_ms.unwrap_or(0).max(0);
    let updated = row.updated_at_unix_ms.unwrap_or(created).max(created);
    (created, updated)
}

fn metadata(row: &ConversationSummary) -> Value {
    // HAPI uses path as a project grouping key. Share the project label
    // without disclosing the host's absolute directory.
    let project = row
        .cwd
        .as_deref()
        .and_then(|cwd| {
            cwd.rsplit(['/', '\\'])
                .find(|component| !component.is_empty())
        })
        .unwrap_or("Velune");
    json!({
        "name": row.title.display_text(),
        "path": format!("/Velune/{project}"),
        "host": "Velune",
        "startingMode": "remote",
        "capabilities": {"terminal": false},
    })
}

fn content(message: &DeliveredMessage) -> Option<Value> {
    let mut lines = Vec::new();
    for block in &message.blocks {
        let text = match block {
            DeliveredBlock::Text { text }
            | DeliveredBlock::Notice { text }
            | DeliveredBlock::ContextCompaction { text } => text.as_str(),
            DeliveredBlock::RuntimeFailure { .. } => "Velune reported an execution error.",
            _ => continue,
        };
        if !text.trim().is_empty() {
            lines.push(text);
        }
    }
    if lines.is_empty() {
        return None;
    }
    let text = lines.join("\n");
    Some(if message.role == MessageRole::User {
        json!({"role": "user", "content": {"type": "text", "text": text}})
    } else {
        json!({
            "role": "agent",
            "content": {"type": "codex", "data": {"type": "message", "message": text}}
        })
    })
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
struct Position {
    at: i64,
    seq: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Message {
    id: String,
    seq: usize,
    local_id: Option<String>,
    content: Value,
    created_at: i64,
    invoked_at: Option<i64>,
    #[serde(skip)]
    stamp: u64,
}

impl Message {
    fn position(&self) -> Position {
        Position {
            at: self.invoked_at.unwrap_or(self.created_at),
            seq: self.seq,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessageQuery {
    limit: Option<usize>,
    before_seq: Option<usize>,
    before_at: Option<i64>,
    after_seq: Option<usize>,
    after_at: Option<i64>,
    until_seq: Option<usize>,
    until_at: Option<i64>,
    epoch: Option<u64>,
}

impl MessageQuery {
    pub(super) fn valid(&self) -> bool {
        (1..=200).contains(&self.limit.unwrap_or(50))
            && self.before_seq.is_some() == self.before_at.is_some()
            && self.after_seq.is_some() == self.after_at.is_some()
            && self.until_seq.is_some() == self.until_at.is_some()
            && !(self.before_seq.is_some() && self.after_seq.is_some())
            && (self.after_seq.is_some() || (self.until_seq.is_none() && self.epoch.is_none()))
    }
}

pub(super) struct Projection {
    pub(super) session: Value,
    epoch: u64,
    rows: Vec<Message>,
    pending: HashMap<String, bool>,
}

impl Projection {
    pub(super) fn page(&self, query: MessageQuery) -> Value {
        let limit = query.limit.unwrap_or(50);
        let head = self.rows.last().map(Message::position);
        let reset =
            query.after_seq.is_some() && query.epoch.is_some_and(|epoch| epoch != self.epoch);
        let (direction, rows, next_after, snapshot_head, has_more) =
            if let Some((seq, at)) = query.after_seq.zip(query.after_at).filter(|_| !reset) {
                let after = Position { at, seq };
                let until = query
                    .until_seq
                    .zip(query.until_at)
                    .map(|(seq, at)| Position { at, seq });
                let bound = head.map(|head| until.map_or(head, |until| until.min(head)));
                let end = bound.map_or(0, |bound| {
                    self.rows.partition_point(|row| row.position() <= bound)
                });
                let start = self.rows[..end].partition_point(|row| row.position() <= after);
                let page_end = end.min(start + limit);
                let rows = &self.rows[start..page_end];
                let next_after = rows.last().map(Message::position).or(Some(after));
                ("after", rows, next_after, bound, page_end < end)
            } else {
                let before = query
                    .before_seq
                    .zip(query.before_at)
                    .map(|(seq, at)| Position { at, seq });
                let end = before.map_or(self.rows.len(), |before| {
                    self.rows.partition_point(|row| row.position() < before)
                });
                let start = end.saturating_sub(limit);
                let rows = &self.rows[start..end];
                let direction = if before.is_some() { "before" } else { "latest" };
                (
                    direction,
                    rows,
                    rows.last().map(Message::position),
                    head,
                    start > 0,
                )
            };
        let next_before = rows.first().map(Message::position);
        json!({
            "messages": rows,
            "page": {
                "direction": direction, "limit": limit, "epoch": self.epoch, "reset": reset,
                "nextBeforeSeq": next_before.map(|position| position.seq),
                "nextBeforeAt": next_before.map(|position| position.at),
                "nextAfterSeq": next_after.map(|position| position.seq),
                "nextAfterAt": next_after.map(|position| position.at),
                "snapshotHeadSeq": snapshot_head.map(|position| position.seq),
                "snapshotHeadAt": snapshot_head.map(|position| position.at),
                "hasMore": has_more,
            }
        })
    }

    pub(super) fn queued_state(&self, local_ids: &[String]) -> Value {
        let requested = local_ids.iter().collect::<HashSet<_>>();
        let invoked = self
            .rows
            .iter()
            .filter_map(|row| {
                let id = row.local_id.as_ref()?;
                requested.contains(id).then_some((id, row.invoked_at?))
            })
            .collect::<HashMap<_, _>>();
        let queued = requested
            .iter()
            .filter(|id| !invoked.contains_key(*id) && self.pending.get(**id) == Some(&true))
            .collect::<Vec<_>>();
        let unknown = requested
            .iter()
            .filter(|id| !invoked.contains_key(*id) && self.pending.get(**id) != Some(&true))
            .collect::<Vec<_>>();
        let invoked = invoked
            .into_iter()
            .map(|(id, at)| json!({"localId": id, "invokedAt": at}))
            .collect::<Vec<_>>();
        json!({
            "queuedLocalIds": queued, "indeterminateLocalIds": unknown,
            "invokedLocalMessages": invoked,
        })
    }
}

#[derive(Default)]
pub(super) struct Changes {
    epoch: Option<u64>,
    session: Option<Value>,
    known: HashMap<String, (u64, Option<String>)>,
    unknown: HashSet<String>,
}

impl Changes {
    pub(super) fn observe(&mut self, id: &str, projection: Projection) -> Vec<Value> {
        let mut changes = Vec::new();
        if self.session.as_ref() != Some(&projection.session) {
            changes.push(
                json!({"type": "session-updated", "sessionId": id, "data": projection.session}),
            );
            self.session = Some(projection.session);
        }
        let first = self.epoch.is_none();
        let invalidated = self.epoch.is_some_and(|epoch| epoch != projection.epoch);
        self.epoch = Some(projection.epoch);
        let rows = &projection.rows[projection.rows.len().saturating_sub(LIVE_WINDOW)..];
        let send_start = if first {
            rows.len().saturating_sub(FIRST_LIVE_WINDOW)
        } else {
            0
        };
        if invalidated {
            changes.push(json!({"type": "messages-invalidated", "sessionId": id}));
        }
        for row in &rows[send_start..] {
            let fingerprint = (row.stamp, row.local_id.clone());
            if !invalidated && self.known.get(&row.id) != Some(&fingerprint) {
                changes.push(json!({"type": "message-received", "sessionId": id, "message": row}));
            }
        }
        self.known = rows
            .iter()
            .map(|row| (row.id.clone(), (row.stamp, row.local_id.clone())))
            .collect();
        let unknown = projection
            .pending
            .into_iter()
            .filter_map(|(id, confirmed)| (!confirmed).then_some(id))
            .collect::<HashSet<_>>();
        let newly_unknown = unknown.difference(&self.unknown).collect::<Vec<_>>();
        if !newly_unknown.is_empty() {
            changes.push(json!({"type": "messages-indeterminate", "sessionId": id, "localIds": newly_unknown}));
        }
        self.unknown = unknown;
        changes
    }
}

pub(super) fn list_changes(
    previous: Option<&[ConversationSummary]>,
    current: &[ConversationSummary],
) -> Vec<Value> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let current_by_id = current
        .iter()
        .map(|row| (&row.id, row))
        .collect::<HashMap<_, _>>();
    let mut changes = previous
        .iter()
        .filter(|row| !current_by_id.contains_key(&row.id))
        .map(|row| json!({"type": "session-removed", "sessionId": row.id}))
        .collect::<Vec<_>>();
    let previous_by_id = previous
        .iter()
        .map(|row| (&row.id, row))
        .collect::<HashMap<_, _>>();
    if let Some(row) = current
        .iter()
        .find(|row| previous_by_id.get(&row.id) != Some(row))
    {
        // Without data, the official clients re-fetch the list. One frame
        // suffices even when multiple rows changed in this poll.
        changes.push(json!({"type": "session-updated", "sessionId": row.id}));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use velune_application_api::conversation::{
        ConversationActions, ConversationTitle, FollowUp, InteractionOption, RuntimeInteraction,
    };
    use velune_application_api::{ConversationUsage, TranscriptCursor};

    fn snapshot(ids: &[&str]) -> DeliveredSnapshot {
        DeliveredSnapshot {
            cursor: TranscriptCursor {
                generation: 1,
                sequence: 1,
            },
            base: None,
            message_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
            messages: ids
                .iter()
                .map(|id| DeliveredMessage {
                    stamp: 1,
                    completed: true,
                    id: (*id).to_owned(),
                    role: MessageRole::User,
                    timestamp_unix_ms: Some(100),
                    blocks: vec![DeliveredBlock::Text {
                        text: format!("message {id}"),
                    }],
                })
                .collect(),
            revision: 1,
            conversation: ConversationSummary {
                id: "runtime:session".into(),
                title: ConversationTitle::Native("Shared conversation".into()),
                updated_at_unix_ms: Some(100),
                created_at_unix_ms: Some(99),
                runtime_id: "runtime".into(),
                cwd: Some("/private/host/project".into()),
                can_rename: false,
                can_delete: false,
                parent_conversation_id: None,
                native_read_state: None,
            },
            context_runtime_id: "runtime".into(),
            resource_id: None,
            model_record_key: None,
            run_state: RunState::Idle,
            transcript_turns: Vec::new(),
            usage: ConversationUsage::default(),
            transcript_items: Vec::new(),
            transcript_outline: Vec::new(),
            transcript_message_identities: Vec::new(),
            message_identity_confirmations: Vec::new(),
            pending_interactions: Vec::new(),
            actions: ConversationActions {
                input_access: Default::default(),
                can_send: true,
                can_cancel: false,
                can_switch: false,
            },
            follow_up: FollowUp::default(),
            sub_agents: Vec::new(),
            collaborators: Vec::new(),
        }
    }

    #[test]
    fn compound_cursor_survives_append_and_resets_on_rewrite() {
        let history = History::new(500);
        let initial = history.project("runtime:session", snapshot(&["a", "b"]));
        let latest = initial.page(MessageQuery {
            limit: Some(1),
            ..Default::default()
        });
        assert_eq!(latest["messages"][0]["id"], "b");
        assert_eq!(latest["page"]["nextBeforeSeq"], 2);
        assert_eq!(latest["page"]["nextBeforeAt"], 100);
        assert_eq!(latest["page"]["hasMore"], true);

        let query = || MessageQuery {
            after_seq: Some(2),
            after_at: Some(100),
            epoch: Some(initial.epoch),
            ..Default::default()
        };
        let appended = history.project("runtime:session", snapshot(&["a", "b", "c"]));
        let page = appended.page(query());
        assert_eq!(page["messages"][0]["id"], "c");
        assert_eq!(page["page"]["reset"], false);
        assert_eq!(page["page"]["epoch"], initial.epoch);

        let replaced = history.project("runtime:session", snapshot(&["a", "rewritten", "c"]));
        assert_eq!(replaced.page(query())["page"]["reset"], true);
    }

    #[test]
    fn live_feed_correlates_only_authoritative_ids_and_filters_unsupported_approvals() {
        let history = History::new(500);
        let id = "runtime:session";
        history.note_send(id, "acknowledged".into(), Some("native-user".into()));
        history.note_send(id, "unknown".into(), None);
        let mut snapshot = snapshot(&["native-user"]);
        for (id, choices) in [
            (
                "supported",
                [
                    ("vendorPermitOnce", ApprovalOptionKind::AllowOnce),
                    ("decline", ApprovalOptionKind::RejectOnce),
                ],
            ),
            (
                "unsupported",
                [
                    ("allow", ApprovalOptionKind::AllowAlways),
                    ("deny", ApprovalOptionKind::RejectOnce),
                ],
            ),
        ] {
            snapshot.pending_interactions.push(RuntimeInteraction {
                id: id.into(),
                kind: InteractionKind::Approval {
                    title: "Approve command".into(),
                    detail: "fixture command".into(),
                    options: choices
                        .into_iter()
                        .map(|(id, kind)| InteractionOption {
                            id: id.into(),
                            label: id.into(),
                            approval_kind: Some(kind),
                        })
                        .collect(),
                },
            });
        }
        let projection = history.project(id, snapshot.clone());
        let approvals = projection.session["agentState"]["requests"]
            .as_object()
            .unwrap();
        assert_eq!(approvals.keys().collect::<Vec<_>>(), ["supported"]);
        assert_eq!(projection.session["metadata"]["path"], "/Velune/project");
        assert_eq!(projection.rows[0].local_id.as_deref(), Some("acknowledged"));
        let queued = projection.queued_state(&["acknowledged".into(), "unknown".into()]);
        assert_eq!(queued["invokedLocalMessages"][0]["localId"], "acknowledged");
        assert_eq!(queued["indeterminateLocalIds"], json!(["unknown"]));

        let mut changes = Changes::default();
        let frames = changes.observe(id, projection);
        assert_eq!(frames[0]["type"], "session-updated");
        assert_eq!(frames[1]["type"], "message-received");
        assert_eq!(frames[2]["type"], "messages-indeterminate");
        assert!(
            changes
                .observe(id, history.project(id, snapshot))
                .is_empty()
        );
    }
}
