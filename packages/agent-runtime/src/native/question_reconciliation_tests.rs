//! Exercise actual native request lifetimes without a model, account or user data.
use super::*;
use velune_test_support::TempRoot;

pub(super) struct Fixture {
    /// Owns the session's directories until the fixture drops.
    _root: TempRoot,
    pub(super) session: NativeSession,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = TempRoot::new("question-lifecycle");
        let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/question_lifecycle_peer.py");
        let mut session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.to_path_buf()),
            kind: NativeKind::Codex166,
            binary,
            node_binary: None,
            agent_dir: root.to_path_buf(),
            resources_directory: root.to_path_buf(),
            projection_directory: root.to_path_buf(),
            gateway: None,
            auto_compaction: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        session.replace_snapshot("parent", Some(root.path()), "runtime", vec![]);
        session.turn_id = Some("turn".into());
        session.running();
        Self {
            _root: root,
            session,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
    }
}

pub(super) fn pending(session: &NativeSession) -> Vec<RuntimeInteraction> {
    session
        .snapshot
        .as_ref()
        .unwrap()
        .pending_interactions
        .clone()
}
