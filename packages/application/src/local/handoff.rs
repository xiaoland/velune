//! A user's decision to give up execution outlives the current view and handle.
use super::*;
use crate::ModelSelection;
use crate::conversation::{ControlHandoff, ConversationSnapshot, InputAccess};
use serde::{Deserialize, Serialize};
use velune_agent_runtime::execution::ExecutionTarget;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Identity {
    location: Location,
    kind: RuntimeKind,
    source: PathBuf,
    native_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Location {
    Local,
    Ssh {
        destination: String,
        port: Option<u16>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Hold {
    identity: Identity,
    phase: Phase,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Requested,
    HandleClosed,
    Released,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Disk {
    schema_version: u32,
    holds: Vec<Hold>,
}

pub(super) struct Handoffs {
    path: PathBuf,
    // Keep a damaged or unknown file intact and fail closed for dispatch.
    holds: Result<Vec<Hold>, String>,
}

impl Handoffs {
    pub(super) fn open(home: &Path) -> Self {
        let path = home.join("conversation-handoffs.json");
        let holds = Self::load(&path).map_err(|error| {
            tracing::warn!(target: "velune_application", event = "conversation_handoffs_unavailable", detail = %error);
            error.to_string()
        });
        Self { path, holds }
    }

    fn load(path: &Path) -> Result<Vec<Hold>, RuntimeError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let disk: Disk = serde_json::from_slice(&bytes)?;
        if disk.schema_version != 1 {
            return Err(RuntimeError::invalid(
                "会话交接记录格式不受支持，原文件已保留",
            ));
        }
        for (index, hold) in disk.holds.iter().enumerate() {
            if hold.identity.native_id.is_empty()
                || hold.identity.source.as_os_str().is_empty()
                || disk.holds[..index]
                    .iter()
                    .any(|prior| prior.identity == hold.identity)
            {
                return Err(RuntimeError::invalid(
                    "会话交接记录的身份无效，原文件已保留",
                ));
            }
        }
        Ok(disk.holds)
    }

    fn get(&self, identity: &Identity) -> Option<Phase> {
        self.holds
            .as_ref()
            .ok()?
            .iter()
            .find(|hold| hold.identity == *identity)
            .map(|hold| hold.phase)
    }

    fn set(&mut self, identity: &Identity, phase: Option<Phase>) -> Result<(), RuntimeError> {
        let old = self
            .holds
            .as_ref()
            .map_err(|detail| RuntimeError::Invalid(format!("无法读取会话交接记录：{detail}")))?;
        if self.get(identity) == phase {
            return Ok(());
        }
        let mut holds = old.clone();
        holds.retain(|hold| hold.identity != *identity);
        if let Some(phase) = phase {
            holds.push(Hold {
                identity: identity.clone(),
                phase,
            });
        }
        let disk = Disk {
            schema_version: 1,
            holds,
        };
        crate::repository::commit(&self.path, &serde_json::to_vec_pretty(&disk)?)?;
        self.holds = Ok(disk.holds);
        Ok(())
    }
}

impl CoreRuntime {
    fn handoff_identity(
        &self,
        runtime: &RuntimeInstance,
        conversation_id: &str,
    ) -> Result<Identity, RuntimeError> {
        let (execution, os_home) = self.runtime_location(runtime)?;
        let local = execution.is_local();
        let location = match execution {
            ExecutionTarget::Local => Location::Local,
            ExecutionTarget::Ssh(ssh) => Location::Ssh {
                destination: ssh.destination,
                port: ssh.port,
            },
        };
        let mut native_id = conversation_id
            .strip_prefix(&format!("{}:", runtime.id))
            .unwrap_or(conversation_id)
            .to_owned();
        let source = if runtime.type_id == RuntimeKind::Pi && Path::new(&native_id).is_absolute() {
            // Pi names the actual session file even when instances share a sessions directory.
            if local && let Ok(path) = std::fs::canonicalize(&native_id) {
                native_id = path.to_string_lossy().into_owned();
            }
            PathBuf::from(&native_id)
        } else if let Some(path) = self.native_agent_directory(runtime, os_home.as_deref())? {
            if local {
                std::fs::canonicalize(&path).unwrap_or(path)
            } else {
                path
            }
        } else {
            // ACP does not expose a filesystem identity to compare across instances.
            PathBuf::from(&runtime.id)
        };
        Ok(Identity {
            location,
            kind: runtime.type_id,
            source,
            native_id,
        })
    }

    fn handoff_identity_for_snapshot(
        &self,
        snapshot: &ConversationSnapshot,
        session: &session::Session,
    ) -> Result<Identity, RuntimeError> {
        let (runtime_id, native_id) = self
            .linked_handoff_source(&snapshot.conversation.id)
            .unwrap_or((&snapshot.context_runtime_id, &snapshot.conversation.id));
        let runtime = session
            .execution()
            .filter(|configuration| configuration.runtime.id == runtime_id)
            .map(|configuration| &configuration.runtime)
            .map_or_else(|| self.runtime_instance(runtime_id), Ok)?;
        self.handoff_identity(runtime, native_id)
    }

    fn linked_handoff_source(&self, id: &str) -> Option<(&str, &str)> {
        let segment = self
            .conversation_links
            .iter()
            .find(|link| link.id == id)?
            .segments
            .last()?;
        Some((
            &segment.runtime_instance_id,
            &segment.native_conversation_id,
        ))
    }

    fn can_release_conversation(&self, identity: &Identity) -> bool {
        if self.handoffs.holds.is_err()
            || !self.session.can_reopen_execution()
            || self.session.gateway().is_some()
            || matches!(
                self.session.model_selection,
                Some(Some(ModelSelection::Managed { .. }))
            )
        {
            return false;
        }
        let supported = match &self.session.active_state {
            ActiveState::Native(native) => self
                .control_target()
                .is_ok_and(|target| native.can_handoff_control(target.as_ref())),
            ActiveState::Pi(_) | ActiveState::Acp(_) => true,
            ActiveState::Empty | ActiveState::History(_) | ActiveState::Pending(_) => false,
        };
        supported && !self.other_handle_owns(identity)
    }

    fn other_handle_owns(&self, identity: &Identity) -> bool {
        self.parked.iter().any(|session| {
            session.active_state.live().is_some()
                && session.projected().is_some_and(|snapshot| {
                    self.handoff_identity_for_snapshot(snapshot, session)
                        .is_ok_and(|other| other == *identity)
                })
        })
    }

    pub(super) fn conversation_handoff(
        &self,
        snapshot: &ConversationSnapshot,
    ) -> Option<ControlHandoff> {
        if snapshot.conversation.parent_conversation_id.is_some()
            || self.session.side.is_some()
            || self.session.unsubmitted_draft.is_some()
        {
            return None;
        }
        let holds = self.handoffs.holds.as_ref().ok()?;
        let live = self.session.active_state.live().is_some();
        if holds.is_empty() && !live {
            return None;
        }
        let identity = self
            .handoff_identity_for_snapshot(snapshot, &self.session)
            .ok()?;
        match self.handoffs.get(&identity) {
            Some(Phase::Requested | Phase::HandleClosed) => Some(ControlHandoff::Releasing),
            Some(Phase::Released) => Some(ControlHandoff::Released),
            None if live && self.can_release_conversation(&identity) => {
                Some(ControlHandoff::CanRelease)
            }
            None => None,
        }
    }

    pub(super) fn check_handoff_input(
        &self,
        snapshot: &ConversationSnapshot,
    ) -> Result<(), RuntimeError> {
        if self.handoffs.holds.is_err() {
            return Err(RuntimeError::invalid("会话交接记录不可用，暂不能提交输入"));
        }
        if matches!(
            snapshot.follow_up.control_handoff,
            Some(ControlHandoff::Releasing | ControlHandoff::Released)
        ) {
            return Err(RuntimeError::invalid(
                "会话已申请交还，请先恢复 Velune 控制",
            ));
        }
        Ok(())
    }

    pub(super) fn current_handoff_blocks_dispatch(&self) -> bool {
        self.handoffs.holds.is_err() || self.current_handoff_phase().is_some()
    }

    pub(super) fn current_handoff_is_released(&self) -> bool {
        self.current_handoff_phase() == Some(Phase::Released)
    }

    fn current_handoff_phase(&self) -> Option<Phase> {
        let holds = self.handoffs.holds.as_ref().ok()?;
        if holds.is_empty() {
            return None;
        }
        let snapshot = self.session.projected()?;
        let identity = self
            .handoff_identity_for_snapshot(snapshot, &self.session)
            .ok()?;
        self.handoffs.get(&identity)
    }

    pub(super) fn scheduled_handoff_blocks_dispatch(&self, id: &str) -> bool {
        let Ok(holds) = &self.handoffs.holds else {
            return true;
        };
        if holds.is_empty() {
            return false;
        }
        let source = self.linked_handoff_source(id).or_else(|| {
            self.runtime_instances
                .iter()
                .filter(|runtime| {
                    id.strip_prefix(&runtime.id)
                        .is_some_and(|rest| rest.starts_with(':'))
                })
                .max_by_key(|runtime| runtime.id.len())
                .map(|runtime| (runtime.id.as_str(), id))
        });
        source.is_some_and(|(runtime_id, native_id)| {
            self.runtime_instance(runtime_id)
                .and_then(|runtime| self.handoff_identity(runtime, native_id))
                .is_ok_and(|identity| self.handoffs.get(&identity).is_some())
        })
    }

    pub(crate) fn set_conversation_handoff(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
        release: bool,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        // Accept and persist the handoff before the owner drains runnable queues.
        self.focus_conversation(conversation_id);
        let snapshot = self
            .queried_snapshot(runtime_id, conversation_id)?
            .ok_or_else(|| RuntimeError::invalid("要交接的会话已变化，请刷新后重试"))?;
        if snapshot.conversation.parent_conversation_id.is_some()
            || self.session.side.is_some()
            || self.session.unsubmitted_draft.is_some()
        {
            return Err(RuntimeError::invalid("此会话不能交还控制权"));
        }
        let identity = self.handoff_identity_for_snapshot(&snapshot, &self.session)?;
        if release {
            if self.handoffs.get(&identity).is_none() {
                if !self.can_release_conversation(&identity) {
                    return Err(RuntimeError::invalid("此运行时当前没有可交还的原生会话"));
                }
                self.handoffs.set(&identity, Some(Phase::Requested))?;
            }
            self.advance_conversation_handoff()?;
        } else {
            self.handoffs.set(&identity, None)?;
            if let Ok(Some(target)) = self.control_target() {
                self.control_observations.invalidate(&target);
            }
        }
        self.snapshot_result()
    }

    pub(super) fn advance_conversation_handoff(&mut self) -> Result<(), RuntimeError> {
        let Ok(holds) = &self.handoffs.holds else {
            return Ok(());
        };
        if !holds.iter().any(|hold| hold.phase != Phase::Released) || self.session.executing() {
            return Ok(());
        }
        let Some(snapshot) = self.session.projected() else {
            return Ok(());
        };
        if snapshot
            .pending_interactions
            .iter()
            .any(|interaction| interaction.blocks_turn())
            || self.session.compacting
        {
            return Ok(());
        }
        let identity = self.handoff_identity_for_snapshot(snapshot, &self.session)?;
        let Some(phase @ (Phase::Requested | Phase::HandleClosed)) = self.handoffs.get(&identity)
        else {
            return Ok(());
        };
        if self.other_handle_owns(&identity) {
            return Ok(());
        }
        let target = self.control_target()?;
        let had_handle = self.session.active_state.live().is_some();
        if had_handle {
            self.invalidate_execution()?;
            if let Ok(Some(target)) = self.control_target() {
                self.control_observations.invalidate(&target);
            }
            self.handoffs.set(&identity, Some(Phase::HandleClosed))?;
        }
        // With a native lock protocol, success requires a fresh observation.
        // Otherwise successful close confirms that Velune's handle is gone.
        let released = match target {
            Some(target) => match self.observe_control_target(Ok(Some(target))) {
                InputAccess::Editable => true,
                InputAccess::ExternalController => had_handle || phase == Phase::HandleClosed,
                _ => false,
            },
            None => had_handle || phase == Phase::HandleClosed,
        };
        if released {
            self.handoffs.set(&identity, Some(Phase::Released))?;
            if let Ok(Some(target)) = self.control_target() {
                self.control_observations.invalidate(&target);
            }
        }
        Ok(())
    }
}
