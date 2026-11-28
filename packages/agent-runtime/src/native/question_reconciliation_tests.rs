//! Exercise actual native request lifetimes without a model, account or user data.
use super::*;
use std::os::unix::fs::PermissionsExt;
use velune_test_support::TempRoot;

pub(super) struct Fixture {
    /// Owns the session's directories until the fixture drops.
    _root: TempRoot,
    pub(super) session: NativeSession,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = TempRoot::new("question-lifecycle");
        let binary = root.join("codex");
        std::fs::write(
            &binary,
            r#"#!/usr/bin/python3
import json,sys
if '--version' in sys.argv:
 print('codex-cli 0.166.0');sys.exit(0)
for line in sys.stdin:
 request=json.loads(line)
 if 'id' in request and 'method' in request:
  print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{}}),flush=True)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
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
