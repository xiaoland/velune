//! Exercise actual native request lifetimes without a model, account or user data.
use super::*;
use std::os::unix::fs::PermissionsExt;

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) session: NativeSession,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-question-lifecycle-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
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
            os_home: Some(root.clone()),
            kind: NativeKind::Codex166,
            binary,
            node_binary: None,
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        session.replace_snapshot("parent", Some(&root), "runtime", vec![]);
        session.turn_id = Some("turn".into());
        session.running();
        Self { root, session }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
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
