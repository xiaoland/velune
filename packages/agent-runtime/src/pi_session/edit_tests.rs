//! Typed edit dependencies distinguish enqueue/Written from validated outcomes.
use super::*;

fn fixture() -> (PiSession, std::path::PathBuf) {
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-pi-edit-tickets-{:x}",
        u64::from_le_bytes(nonce)
    ));
    std::fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"
import json,pathlib,sys,time
root=pathlib.Path(__file__).parent
branched=False; slot_prompt=None
def emit(value): print(json.dumps(value),flush=True)
for line in sys.stdin:
 r=json.loads(line); kind=r['type']; data={}
 with (root/'wire').open('a') as log: log.write(json.dumps(r)+'\n')
 if kind=='prompt' and r['message'].startswith('/velune-projection-sync '):
  messages=[] if branched else [{'id':'pi:synthetic:user:0','role':'user','content':'old body'}]
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps({'requestId':r['id'],'snapshotVersion':1,'messages':messages,'messageIds':[m['id'] for m in messages],'identityConfirmations':[],'sessionFile':'/synthetic/session.jsonl'})})
  data={'disposition':'handled'}
 elif kind=='prompt' and r['message'].startswith('/velune-rewind '):
  (root/'entered-branch').touch()
  while not (root/'release-branch').exists(): time.sleep(.005)
  branched=True
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.rewind','statusText':json.dumps({'entryId':r['message'].split(' ',1)[1]})})
  data={'disposition':'handled'}
 elif kind=='set_model':
  (root/'entered-model').touch()
  while not (root/'release-model').exists(): time.sleep(.005)
  emit({'type':'response','id':r['id'],'command':kind,'success':False,'error':'synthetic unsupported model'}); continue
 elif kind=='prompt' and r['message'].startswith('/velune-background-work '):
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps({'version':1,'kind':'unsupported','tasks':[]})})
  data={'disposition':'handled'}
 elif kind=='prompt':
  text=r['message']
  if text=='control-slots':
   slot_prompt=r; emit({'type':'agent_start'})
   emit({'type':'extension_ui_request','method':'confirm','id':'dialog','title':'same connection'})
   (root/'slot-question').touch(); continue
  if text.startswith('echo-'):
   emit({'type':'agent_start'})
   for index,message in enumerate([
    {'role':'user','content':text},
    {'id':'pi:synthetic:user:0','role':'user','content':text},
    {'id':'pi:synthetic:unrelated:0','role':'user','content':'unrelated native input'},
    {'id':'pi:synthetic:new:0','role':'user','content':text}
   ]):
    emit({'type':'message_start','message':message}); (root/('echo-'+str(index))).touch()
    while not (root/('release-echo-'+str(index))).exists(): time.sleep(.005)
   if text=='echo-reject':
    emit({'type':'response','id':r['id'],'command':kind,'success':False,'error':'native replacement rejected'}); continue
  elif text in ('success-eof','unmatched-eof','pure-eof'):
   (root/'prompt-admitted').touch()
   while not (root/'release-native').exists(): time.sleep(.005)
   if text!='pure-eof': emit({'type':'response','id':r['id'] if text=='success-eof' else 'unrelated-request','command':kind,'success':True,'data':{}})
   (root/'native-exited').touch(); sys.exit(0)
 elif kind=='extension_ui_response':
  assert r['id']=='dialog' and r['confirmed'] is True
  (root/'slot-answer').touch(); continue
 elif kind=='abort':
  emit({'type':'agent_settled','aborted':True})
 elif kind=='get_state' and slot_prompt is not None and (root/'release-slot-fact').exists():
  emit({'type':'response','id':slot_prompt['id'],'command':'prompt','success':True,'data':{'disposition':'queued'}}); slot_prompt=None
 elif kind=='get_commands': data={'commands':[]}
 emit({'type':'response','id':r['id'],'command':kind,'success':True,'data':data})
"#).unwrap();
    let config: Config = serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
    let mut client = Client::spawn(config.clone()).unwrap();
    client.state().unwrap();
    let projection = PiProjection::new(ConversationSummary {
        native_read_state: None,
        id: "pi:/synthetic/session.jsonl".into(),
        runtime_id: "pi".into(),
        title: crate::conversation::conversation_title(None, None),
        cwd: None,
        created_at_unix_ms: None,
        updated_at_unix_ms: None,
        can_rename: false,
        can_delete: false,
        parent_conversation_id: None,
    });
    let mut session = PiSession::new(config);
    session.start(client, projection);
    (session, root)
}
fn settle(session: &mut PiSession, ticket: PiTicket) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        session.poll();
        if let Some(result) = session.take_confirmation(ticket)? {
            return result;
        }
        assert!(Instant::now() < end, "typed edit operation did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn wait(session: &mut PiSession, root: &std::path::Path, name: &str) {
    let end = Instant::now() + Duration::from_secs(3);
    while !root.join(name).exists() {
        session.poll();
        assert!(Instant::now() < end, "helper did not enter {name}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn edit_tickets_do_not_settle_at_written_and_model_rejection_is_not_success() {
    let (mut session, root) = fixture();
    let history = session.begin_edit_history("pi", None).unwrap();
    settle(&mut session, history).unwrap();
    let start = Instant::now();
    let branch = session.begin_edit_branch("user").unwrap();
    assert!(start.elapsed() < Duration::from_millis(100));
    wait(&mut session, &root, "entered-branch");
    session.poll();
    assert!(
        session.take_confirmation(branch).unwrap().is_none(),
        "physical Written cannot confirm rewind"
    );
    assert!(
        session
            .begin_edit_model(Some("synthetic"), "alpha")
            .is_err()
    );
    std::fs::write(root.join("release-branch"), []).unwrap();
    settle(&mut session, branch).unwrap();
    let history = session.begin_edit_history("pi", None).unwrap();
    settle(&mut session, history).unwrap();
    assert!(session.snapshot().unwrap().messages.is_empty());
    let model = session
        .begin_edit_model(Some("synthetic"), "alpha")
        .unwrap();
    assert_ne!(
        branch.session_generation, model.session_generation,
        "rewind must bind a new native generation"
    );
    wait(&mut session, &root, "entered-model");
    session.poll();
    assert!(session.take_confirmation(model).unwrap().is_none());
    assert!(
        session
            .begin_submission_prompt(&TurnInput {
                text: "replacement".into(),
                effort: None,
                permission_mode: None,
                images: vec![]
            })
            .is_err()
    );
    std::fs::write(root.join("release-model"), []).unwrap();
    assert!(matches!(
        settle(&mut session, model),
        Err(Error::CommandFailed(_))
    ));
    let wire = std::fs::read_to_string(root.join("wire")).unwrap();
    assert!(!wire.contains("replacement"));
    session.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn edit_generation_change_cannot_satisfy_a_pending_dependency() {
    let (mut session, root) = fixture();
    let ticket = session
        .begin_edit_model(Some("synthetic"), "alpha")
        .unwrap();
    wait(&mut session, &root, "entered-model");
    session
        .running
        .as_mut()
        .unwrap()
        .native_scope_changed()
        .unwrap();
    assert!(matches!(
        session.take_confirmation(ticket).unwrap(),
        Some(Err(Error::Transport(_)))
    ));
    std::fs::write(root.join("release-model"), []).unwrap();
    session.poll();
    assert!(
        !std::fs::read_to_string(root.join("wire"))
            .unwrap()
            .contains("replacement")
    );
    session.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unbound_native_user_echoes_never_confirm_the_current_input() {
    for text in ["echo-ack", "echo-reject"] {
        let (mut session, root) = fixture();
        let history = session.begin_edit_history("pi", None).unwrap();
        settle(&mut session, history).unwrap();
        let ticket = session
            .begin_edit_prompt(&TurnInput {
                text: text.into(),
                effort: None,
                permission_mode: None,
                images: vec![],
            })
            .unwrap();
        for index in 0..4 {
            wait(&mut session, &root, &format!("echo-{index}"));
            session.poll();
            assert!(
                session.take_confirmation(ticket).unwrap().is_none(),
                "ID-less/old/unrelated/unbound matching input event settled a prompt"
            );
            std::fs::write(root.join(format!("release-echo-{index}")), []).unwrap();
        }
        let result = settle(&mut session, ticket);
        if text == "echo-ack" {
            result.unwrap();
        } else {
            assert!(
                matches!(result, Err(Error::CommandFailed(_))),
                "unbound echo poisoned the real rejection"
            );
        }
        let count = std::fs::read_to_string(root.join("wire"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|request| request["message"] == text)
            .count();
        assert_eq!(count, 1);
        session.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn exact_prompt_outcome_and_eof_in_one_pump_preserve_acceptance_without_echo() {
    for text in ["success-eof", "unmatched-eof", "pure-eof"] {
        let (mut session, root) = fixture();
        let ticket = session
            .begin_edit_prompt(&TurnInput {
                text: text.into(),
                effort: None,
                permission_mode: None,
                images: vec![],
            })
            .unwrap();
        // Pump admission once; then withhold all session processing until the
        // helper has flushed its terminal response and closed stdout.
        let end = Instant::now() + Duration::from_secs(3);
        while !root.join("prompt-admitted").exists() {
            let _ = session.running.as_mut().unwrap().client.poll_pi();
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        std::fs::write(root.join("release-native"), []).unwrap();
        let mut items = Vec::new();
        while !session.requires_failure_cleanup() {
            items.extend(session.running.as_mut().unwrap().client.poll_pi());
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(items.iter().any(|item| matches!(item, PiPortItem::Receipt(receipt) if receipt.ticket==ticket && matches!(&receipt.phase, PiReceiptPhase::Outcome(_)))), text=="success-eof");
        // Apply one owner batch with the connection already failure-closed.
        session.apply_port_items(items);
        let outcome = session
            .take_confirmation(ticket)
            .unwrap()
            .expect("terminal fact missing");
        if text == "success-eof" {
            outcome.unwrap();
        } else {
            assert!(
                outcome.is_err(),
                "EOF/unmatched response invented acceptance"
            );
        }
        assert!(session.requires_failure_cleanup());
        assert!(
            session
                .begin_edit_prompt(&TurnInput {
                    text: "never replay".into(),
                    effort: None,
                    permission_mode: None,
                    images: vec![]
                })
                .is_err()
        );
        let wire = std::fs::read_to_string(root.join("wire")).unwrap();
        assert_eq!(
            wire.lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .filter(|r| r["message"] == text)
                .count(),
            1
        );
        assert!(!wire.contains("never replay"));
        session.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
