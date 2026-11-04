//! User-requested maintenance of the current native context, without a user turn.
use super::*;

impl CoreRuntime {
    /// Compact only this conversation's native context, using the selected
    /// model. Progress and completion arrive in snapshots.
    pub(crate) fn compact_conversation(
        &mut self,
        runtime_id: &str,
        selection: Option<crate::ModelSelection>,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        self.ensure_active(runtime_id)?;
        self.require_input_access()?;
        let snapshot = self
            .current_snapshot()
            .ok_or_else(|| RuntimeError::invalid("请选择会话"))?;
        if !snapshot.follow_up.can_compact {
            return Err(RuntimeError::invalid("当前会话不能压缩上下文"));
        }
        let runtime = self.runtime_instance(&snapshot.context_runtime_id)?.clone();
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
            if matches!(self.session.active_state, ActiveState::Pi(_))
                && matches!(
                    self.session.model_selection,
                    Some(Some(crate::ModelSelection::Managed { .. }))
                )
            {
                self.bind_gateway_model()?;
            }
            match self.session.active_state.live_mut() {
                Some(live) if live.can_compact() => live
                    .compact()
                    .map_err(|error| RuntimeError::context("上下文压缩", error))?,
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
