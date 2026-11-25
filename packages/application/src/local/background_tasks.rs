//! Background metadata reads only query an already-owned runtime connection.
use super::*;
use velune_conversation::{BackgroundTaskList, BackgroundTaskOutput};

impl CoreRuntime {
    pub(crate) fn background_tasks(
        &mut self,
        runtime: &str,
        conversation: &str,
    ) -> Result<BackgroundTaskList, RuntimeError> {
        let session = std::iter::once(&mut self.session)
            .chain(&mut self.parked)
            .find(|session| session.conversation_id() == Some(conversation));
        if let Some(session) = session {
            if !Self::background_runtime_matches(session, runtime) {
                return Err(RuntimeError::invalid("后台任务来源运行时不匹配"));
            }
            if let Some(live) = session.active_state.live_mut() {
                live.refresh_background_tasks()
                    .map_err(|error| RuntimeError::context("刷新后台任务", error))?;
                return Self::read_background_observation(live);
            }
        }
        Ok(BackgroundTaskList::default())
    }
    fn read_background_observation(
        live: &dyn LiveConversation,
    ) -> Result<BackgroundTaskList, RuntimeError> {
        // Keep the receive charge on the observation until it crosses this boundary.
        live.background_tasks()
            .map(|result| result.into_parts().0)
            .map_err(|error| RuntimeError::context("读取后台任务", error))
    }
    fn background_runtime_matches(session: &session::Session, runtime: &str) -> bool {
        session.projected().is_some_and(|snapshot| {
            snapshot.conversation.runtime_id == runtime || snapshot.context_runtime_id == runtime
        })
    }
    fn background_session(
        &mut self,
        runtime: &str,
        conversation: &str,
    ) -> Result<&mut session::Session, RuntimeError> {
        let session = std::iter::once(&mut self.session)
            .chain(&mut self.parked)
            .find(|session| session.conversation_id() == Some(conversation))
            .ok_or_else(|| RuntimeError::invalid("会话没有可用的后台任务连接"))?;
        if !Self::background_runtime_matches(session, runtime) {
            return Err(RuntimeError::invalid("后台任务来源运行时不匹配"));
        }
        Ok(session)
    }
    pub(crate) fn read_background_task_output(
        &mut self,
        runtime: &str,
        conversation: &str,
        task: &str,
        request: Option<&str>,
    ) -> Result<BackgroundTaskOutput, RuntimeError> {
        self.background_session(runtime, conversation)?
            .active_state
            .live_mut()
            .ok_or_else(|| RuntimeError::invalid("此历史任务没有可用的输出读取连接"))?
            .read_background_task_output(task, request)
            .map(|result| result.into_parts().0)
            .map_err(|error| RuntimeError::context("读取后台任务输出", error))
    }
    pub(crate) fn stop_background_task(
        &mut self,
        runtime: &str,
        conversation: &str,
        task: &str,
    ) -> Result<BackgroundTaskList, RuntimeError> {
        let live = self.background_session(runtime, conversation)?
            .active_state
            .live_mut()
            .ok_or_else(|| RuntimeError::invalid("此历史任务不能独立停止"))?;
        live.stop_background_task(task)
            .map_err(|error| RuntimeError::context("停止后台任务", error))?;
        Self::read_background_observation(live)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_tasks_observations_do_not_focus_prepare_or_scan_cold_history() {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-background-owner-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut app = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        fn history(id: &str) -> session::Session {
            let snapshot: crate::conversation::ConversationSnapshot = serde_json::from_value(serde_json::json!({
                "revision":1,"conversation":{"id":id,"title":{"source":"untitled"},"runtimeId":"synthetic"},
                "contextRuntimeId":"synthetic","resourceId":null,"modelRecordKey":null,"runState":"idle",
                "messages":[{"id":"launch","role":"tool","completed":true,"blocks":[{"kind":"tool","toolID":"receipt","title":"bg_delegate","state":"completed","output":"x".repeat(2*1024*1024)}]}],
                "transcriptTurns":[],"transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],
                "actions":{"canSend":true,"canCancel":false,"canSwitch":true}
            })).unwrap();
            session::Session {
                active_state: ActiveState::History(Box::new(snapshot.into())),
                ..Default::default()
            }
        }
        app.runtime.session = history("focused");
        app.runtime.parked.push(history("parked"));
        let key = app.runtime.session.key;
        let view = crate::ClientViewId("background-test".into());
        for _ in 0..8 {
            let response = app
                .execute_command(
                    crate::Command::ReadBackgroundTasks {
                        runtime_id: "synthetic".into(),
                        conversation_id: "parked".into(),
                    },
                    false,
                    &view,
                )
                .unwrap();
            let crate::Response::ReadBackgroundTasks(list) = response else {
                panic!("expected task list")
            };
            assert!(
                !list.live && list.tasks.is_empty() && list.issue.is_none(),
                "cold launch receipts must not be parsed on owner or treated as live tasks"
            );
            assert!(app.runtime.session.key == key);
        }
        assert!(
            app.execute_command(
                crate::Command::StopBackgroundTask {
                    runtime_id: "synthetic".into(),
                    conversation_id: "parked".into(),
                    task_id: "receipt".into()
                },
                false,
                &view
            )
            .is_err()
        );
        assert!(
            app.runtime.session.key == key,
            "task control must not change focus"
        );
        assert!(
            app.runtime.parked[0].active_state.live().is_none(),
            "observing history never prepares a runtime"
        );
        app.close().unwrap();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
