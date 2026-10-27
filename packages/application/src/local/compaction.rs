//! User-requested maintenance of the current native context, without a user turn.
use super::*;

impl CoreRuntime {
    pub(super) fn compact_conversation_action(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        let snapshot = self
            .current_snapshot()
            .ok_or_else(|| RuntimeError::invalid("请选择会话"))?;
        if !snapshot.follow_up.can_compact {
            return Err(RuntimeError::invalid("当前会话不能压缩上下文"));
        }
        let runtime = self.runtime_instance(&snapshot.context_runtime_id)?.clone();
        let selection = super::model_selection::requested_selection(request)?;
        self.validate_model_selection(&runtime, selection.as_ref())?;
        if matches!(self.session.active_state, ActiveState::History(_))
            || self.session.model_selection.as_ref() != Some(&selection)
        {
            let native = self.native_snapshot().expect("loaded context");
            self.prepare_with_selection(native, selection.as_ref(), false)?;
        }
        let result = (|| {
            if let Some((gateway_id, session)) = self.backend_session_identity() {
                self.execution_backends
                    .begin_user_turn(&gateway_id, &session)
                    .map_err(|error| RuntimeError::context("上下文压缩准备", error))?;
            }
            if matches!(self.session.active_state, ActiveState::Pi)
                && matches!(
                    self.session.model_selection,
                    Some(Some(crate::ModelSelection::Managed { .. }))
                )
            {
                self.bind_gateway_model()?;
            }
            match &mut self.session.active_state {
                ActiveState::Native(session) => session
                    .compact()
                    .map_err(|error| RuntimeError::context("上下文压缩", error))?,
                ActiveState::Pi => {
                    let id = self
                        .session
                        .pi
                        .client
                        .as_mut()
                        .expect("prepared Pi")
                        .compact()
                        .map_err(|error| RuntimeError::context("上下文压缩", error))?;
                    self.session
                        .pi
                        .projection
                        .as_mut()
                        .expect("prepared Pi")
                        .begin_compaction(id);
                    self.session.pi.busy = true;
                }
                _ => return Err(RuntimeError::invalid("此运行时不支持主动压缩上下文")),
            }
            Ok::<_, RuntimeError>(())
        })();
        if result.is_err() && self.session.gateway_runner.is_some() && !self.session.executing() {
            self.finish_managed_turn()?;
        }
        result?;
        self.session.compacting = true;
        self.session.cancel_requested = false;
        self.drain_runtime()?;
        self.snapshot_result()
    }
}
