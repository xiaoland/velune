use super::*;

fn frame(request: &str, messages: Value, identities: Value) -> Value {
    json!({"type":"extension_ui_request","method":"setStatus","statusKey":"velune.session-metadata",
        "statusText":json!({"requestId":request,"snapshotVersion":1,"messages":messages,"messageIds":identities,
            "identityConfirmations":[],"sessionFile":"/synthetic/new.jsonl","name":"new","cwd":"/synthetic"}).to_string()})
}

#[test]
fn snapshot_frame_rejects_count_equal_branch_mismatch_stale_request_and_duplicate_ids() {
    let messages = json!([{"id":"native:a:0","role":"user","content":"first"},{"id":"native:b:0","role":"assistant","content":"second"}]);
    let valid = frame(
        "query",
        messages.clone(),
        json!(["native:a:0", "native:b:0"]),
    );
    let snapshot = Client::projection_snapshot(&valid, "query").unwrap();
    assert_eq!(snapshot["data"]["messages"], messages);
    assert_eq!(
        snapshot["nativeSession"]["sessionFile"],
        "/synthetic/new.jsonl"
    );
    assert!(Client::projection_snapshot(&valid, "another-query").is_err());
    for (body, ids) in [
        (messages.clone(), json!(["native:a:0"])),
        (messages.clone(), json!(["native:b:0", "native:a:0"])),
        (
            json!([{"id":"native:a:0"},{"id":"native:a:0"}]),
            json!(["native:a:0", "native:a:0"]),
        ),
    ] {
        assert!(Client::projection_snapshot(&frame("query", body, ids), "query").is_err());
    }
    let legacy = json!({"statusText":json!({"requestId":"query","messageIds":[],"identityConfirmations":[]}).to_string()});
    assert!(Client::projection_snapshot(&legacy, "query").is_err());
}

#[test]
fn blocking_snapshot_never_reads_another_message_view_and_preserves_after_frame_events() {
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root =
        std::env::temp_dir().join(format!("velune-snapshot-{:x}", u64::from_le_bytes(random)));
    std::fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"
import json,sys,os
def emit(value): print(json.dumps(value),flush=True)
for line in sys.stdin:
 r=json.loads(line)
 with open(os.path.join(os.path.dirname(__file__),'requests.jsonl'),'a') as log: log.write(json.dumps(r)+'\n')
 if r['type']=='get_state':
  emit({'type':'extension_ui_request','method':'notify','message':'old-get-state-event'})
  emit({'type':'response','id':r['id'],'command':'get_state','success':True,'data':{'sessionFile':'/synthetic/old.jsonl','sessionName':'old'}})
 elif r['type']=='prompt':
  atomic=r['message'].startswith('/velune-projection-sync ')
  if atomic: assert r['message']=='/velune-projection-sync '+r['id']
  emit({'type':'extension_ui_request','method':'notify','message':'covered'})
  body=[{'id':'native:u:0','role':'user','content':'x'*(9*1024*1024)},{'id':'native:a:0','role':'assistant','content':'answer'}]
  projection={'messageIds':['native:u:0','native:a:0'],'identityConfirmations':[],'sessionFile':'/synthetic/new.jsonl','name':'new'}
  if atomic: projection.update({'requestId':r['id'],'snapshotVersion':1,'messages':body})
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps(projection)})
  # Appends and a divergent same-length agent view can occur before the
  # command receipt. Neither may be silently covered by the earlier frame.
  emit({'type':'message_start','message':{'id':'late','role':'custom','customType':'background-task-notification','content':'late'}})
  emit({'type':'message_end','message':{'id':'late','role':'custom','customType':'background-task-notification','content':'late'}})
  emit({'type':'response','id':r['id'],'command':'prompt','success':True,'data':{'disposition':'handled'}})
 elif r['type']=='get_messages':
  # The historical protocol sees the newer independently sampled agent view.
  emit({'type':'response','id':r['id'],'command':'get_messages','success':True,'data':{'messages':[{'role':'user','content':'first'},{'role':'assistant','content':'answer'},{'role':'custom','content':'new append'}]}})
 else:
  raise RuntimeError('unexpected operation')
"#).unwrap();
    let config: Config = serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
    let mut client = Client::spawn(config).unwrap();
    let (state, snapshot) = client.state().unwrap();
    assert_eq!(state["data"]["sessionFile"], "/synthetic/new.jsonl");
    assert_eq!(state["data"]["sessionName"], "new");
    assert_eq!(snapshot["data"]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        snapshot["data"]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .len(),
        9 * 1024 * 1024
    );
    let covered = client.take_buffered_events();
    assert!(
        serde_json::to_vec(&covered).unwrap().len() < 2048,
        "snapshot body must not be retained as a deferred event"
    );
    assert_eq!(
        covered[0]["message"], "old-get-state-event",
        "records deferred by get_state remain before the later snapshot barrier"
    );
    assert_eq!(covered[1]["message"], "covered");
    assert!(covered.iter().any(|v| v["message"] == "covered"));
    assert!(!covered.iter().any(|v| v["type"] == "message_start"));
    let after = client.poll();
    assert!(!after.iter().any(|v| v["message"] == "old-get-state-event"));
    assert_eq!(
        after
            .iter()
            .filter(|v| v["type"] == "message_start" || v["type"] == "message_end")
            .count(),
        2
    );
    let requests = std::fs::read_to_string(root.join("requests.jsonl")).unwrap();
    assert!(!requests.contains("get_messages"));
    client.child.stop();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_or_timed_out_snapshot_withdraws_only_its_header_and_preserves_real_events() {
    for timeout in [false, true] {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-snapshot-failure-{:x}",
            u64::from_le_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("timeout"), if timeout { "yes" } else { "no" }).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(&helper, r#"
import json,sys,os,time
def emit(value): print(json.dumps(value),flush=True)
timeout=open(os.path.join(os.path.dirname(__file__),'timeout')).read()=='yes'
for line in sys.stdin:
 r=json.loads(line)
 if r['type']=='get_state':
  emit({'type':'response','id':r['id'],'command':'get_state','success':True,'data':{}}); continue
 emit({'type':'extension_error','error':'real extension diagnostic'})
 emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps({'sessionFile':'/synthetic/previous.jsonl'})})
 emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps({'requestId':r['id'],'snapshotVersion':1,'messages':[{'id':'native:u:0','role':'user','content':'new body'}],'messageIds':['native:u:0'],'identityConfirmations':[],'sessionFile':'/synthetic/new.jsonl','name':'new'})})
 emit({'type':'extension_ui_request','method':'notify','message':'real late event'})
 if timeout: time.sleep(3)
 else: emit({'type':'response','id':r['id'],'command':'prompt','success':False,'error':'synthetic rejection'})
"#).unwrap();
        let config: Config = serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
        let mut client = Client::spawn(config).unwrap();
        client
            .request_with_timeout(json!({"type":"get_state"}), Duration::from_secs(10))
            .unwrap();
        let result = client.request_response(json!({"id":"failed-query","type":"prompt","message":"/velune-projection-sync failed-query"}), "prompt", Duration::from_millis(500), false);
        assert!(
            matches!(&result, Err(Error::Timeout(_))) == timeout,
            "timeout={timeout}, result={result:?}"
        );
        assert!(result.is_err());
        assert!(client.prepared_snapshot.is_none());
        assert!(client.snapshot_event_barrier.is_none());
        let events = client.take_buffered_events();
        assert!(
            events
                .iter()
                .any(|event| event["error"] == "real extension diagnostic")
        );
        assert!(
            events
                .iter()
                .any(|event| event["message"] == "real late event")
        );
        let headers = events
            .iter()
            .filter_map(|event| {
                event["statusText"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
            })
            .collect::<Vec<_>>();
        assert!(
            headers
                .iter()
                .any(|header| header["sessionFile"] == "/synthetic/previous.jsonl")
        );
        assert!(
            !headers
                .iter()
                .any(|header| header["sessionFile"] == "/synthetic/new.jsonl"
                    || header["snapshotHeaderOnly"] == true)
        );
        client.child.stop();
        std::fs::remove_dir_all(root).unwrap();
    }
}
