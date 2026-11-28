//! Real synthetic Pi protocol gates for the edit dependency chain.
use super::*;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use velune_conversation::{EditPlacement, MessageRole, RunState, RuntimeInteractionReply};
use velune_test_support::TempRoot;

struct Fixture(TempRoot);

fn native_header(root: &std::path::Path, name: &str) -> String {
    let id = match name {
        "edit" => "00000000-0000-4000-8000-000000000001",
        "other" => "00000000-0000-4000-8000-000000000002",
        _ => panic!("unexpected synthetic native source"),
    };
    format!(
        "{}\n",
        serde_json::json!({
            "type":"session", "version":3, "id":id,
            "timestamp":"2026-11-16T00:00:00.000Z", "cwd":root,
        })
    )
}

impl Fixture {
    fn new(gate: &str, reject: bool) -> Self {
        let root = TempRoot::new("pi-edit-async");
        fs::write(root.join("gate"), gate).unwrap();
        fs::write(root.join("reject"), if reject { "yes" } else { "no" }).unwrap();
        fs::write(root.join("pi"), "#!/bin/sh\nprintf '1.4.0\\n'\n").unwrap();
        fs::set_permissions(root.join("pi"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("node"), r#"#!/usr/bin/python3
import json,os,pathlib,sys
if '--input-type=module' in sys.argv: print(json.dumps({'binary':str(pathlib.Path(__file__).with_name('pi'))}))
elif '--version' in sys.argv: print('1.4.0')
else: os.execv('/usr/bin/python3',['/usr/bin/python3']+sys.argv[1:])
"#).unwrap();
        fs::set_permissions(root.join("node"), fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["edit", "other"] {
            fs::write(
                root.join(format!("{name}.jsonl")),
                native_header(&root, name),
            )
            .unwrap();
            fs::write(root.join(format!("{name}-history.json")), serde_json::json!({"cwd":root.path(),"messages":[
                {"id":"pi:synthetic:prefix-user:0","role":"user","content":"retained prompt"},
                {"id":"pi:synthetic:prefix-reply:0","role":"assistant","content":"retained reply","stopReason":"stop"},
                {"id":"pi:synthetic:edited-user:0","role":"user","content":"old selected body"},
                {"id":"pi:synthetic:future-reply:0","role":"assistant","content":"discarded future","stopReason":"stop"}
            ]}).to_string()).unwrap();
        }
        fs::write(root.join("pi_sessions.mjs"), r#"
import json,pathlib,sys,hashlib,uuid
# sessionDir is an explicit native history override. Managed HOME chooses a
# different agentDir for authentication/configuration, not a different index.
root=(pathlib.Path(sys.argv[sys.argv.index('--session-dir')+1]) if '--session-dir' in sys.argv else pathlib.Path(sys.argv[sys.argv.index('--agent-dir')+1])/'sessions').resolve(strict=True)
def native_source(flag):
 path=pathlib.Path(sys.argv[sys.argv.index(flag)+1])
 assert not path.is_symlink()
 path=path.resolve(strict=True)
 assert path.parent==root
 header=json.loads(path.read_text().splitlines()[0])
 assert header['type']=='session' and header['version']==3
 uuid.UUID(header['id'])
 assert pathlib.Path(header['cwd']).is_absolute()
 return path,header
if '--inspect-session' in sys.argv:
 source,header=native_source('--inspect-session')
 history=json.loads(root.joinpath(source.stem+'-history.json').read_text())
 if '--with-summary' in sys.argv: history['summary']={'path':str(source),'cwd':header['cwd'],'name':'Synthetic'}
 print(json.dumps(history))
elif '--fork-session' in sys.argv:
 request=json.load(sys.stdin); assert request['preservePrefixEdits'] is True
 mode=(root/'gate').read_text()
 if mode=='fallback-stale-before-read':
  (root/'entered-fork-before-read').touch()
  while not (root/'release-fork').exists(): __import__('time').sleep(.005)
 if hashlib.sha256((root/'edit.jsonl').read_bytes()).hexdigest()!=request['expectedDigest']:
  print(json.dumps({'error':'source differs from authorized byte digest','code':'fork_rejected'})); sys.exit(0)
 source=json.loads((root/'edit-history.json').read_text()); source['messages']=source['messages'][:2]
 for message in source['messages']: message['id']=message['id'].replace('pi:synthetic:','pi:fork:')
 _,header=native_source('--fork-session')
 header.update(id='00000000-0000-4000-8000-000000000003',parentSession=str(root/'edit.jsonl'))
 (root/'fork.jsonl').write_text(json.dumps(header)+'\n')
 (root/'fork-history.json').write_text(json.dumps(source))
 with (root/'fork-count').open('a') as count: count.write('fork\n')
 if mode=='fallback-association-fail': (root/'app'/'conversation-links.json.tmp').mkdir()
 if mode=='fallback-source':
  (root/'entered-fork').touch()
  while not (root/'release-fork').exists(): __import__('time').sleep(.005)
 if mode=='fallback-sync-fail': print(json.dumps({'error':'原生副本已创建；synthetic target fsync failed','code':'fork_created'}))
 elif mode=='fallback-unknown': print(json.dumps({'error':'synthetic fork outcome unknown; recovery path '+str(root/'fork.jsonl'),'code':'fork_outcome_unknown'}))
 else: print(json.dumps({'path':str(root/'fork.jsonl')}))
elif '--delete-message' in sys.argv:
 request=json.load(sys.stdin); source,_=native_source('--delete-message')
 path=root/(source.stem+'-history.json'); history=json.loads(path.read_text())
 boundary=next(i for i,m in enumerate(history['messages']) if m['id']==request['messageId'])
 history['messages']=history['messages'][:boundary] if request['truncate'] else [m for i,m in enumerate(history['messages']) if i!=boundary]
 history['retainedReplyIds']=[m['id'] for m in history['messages'] if m['role']=='assistant']
 path.write_text(json.dumps(history)); (root/'deleted').touch()
 with (root/'delete-count').open('a') as log: log.write('delete\n')
 print(json.dumps({'ok':True}))
else:
 if (root/'hold-index').exists():
  (root/'entered-index').touch()
  while (root/'hold-index').exists(): __import__('time').sleep(.005)
 request=json.load(sys.stdin); wanted=set(request['want']); files=[]
 for name in ['edit','other','fork']:
  path=root/(name+'.jsonl')
  if not path.is_file() or path.is_symlink(): continue
  header=json.loads(path.read_text().splitlines()[0])
  assert header['type']=='session' and header['version']==3
  uuid.UUID(header['id'])
  assert pathlib.Path(header['cwd']).is_absolute()
  stat=path.stat()
  files.append({'path':str(path),'sessionId':header['id'],'size':str(stat.st_size),'mtimeNs':str(stat.st_mtime_ns),'cwd':header['cwd']})
 print(json.dumps({'contractVersion':3,'files':files,'sessions':[{'path':f['path'],'cwd':f['cwd'],'name':'Synthetic'} for f in files if f['path'] in wanted]}))
"#).unwrap();
        fs::write(root.join("pi_rpc.mjs"), r#"
import json,pathlib,sys,time,os
if '--validate' in sys.argv: sys.exit(0)
# Follow the same explicit sessionDir override as the history reader; HOME and
# PI_CODING_AGENT_DIR remain the production-selected managed native config root.
root=pathlib.Path(sys.argv[sys.argv.index('--session-dir')+1]).resolve(strict=True)
source=pathlib.Path(sys.argv[sys.argv.index('--session')+1])
assert not source.is_symlink()
source=source.resolve(strict=True); assert source.parent==root
name=source.stem
history=json.loads(root.joinpath(name+'-history.json').read_text()); branched=False
(root/(name+'-pid')).write_text(str(os.getpid()))
(root/(name+'-home')).write_text(os.environ.get('HOME',''))
mode=(root/'gate').read_text(); reject=(root/'reject').read_text()=='yes'
def emit(value): print(json.dumps(value),flush=True)
def status(key,value): emit({'type':'extension_ui_request','method':'setStatus','statusKey':key,'statusText':json.dumps(value)})
def gate(phase):
 if name!='edit' or phase!=mode: return False
 (root/('entered-'+phase)).touch()
 while not (root/'release').exists(): time.sleep(.005)
 return reject
primary_prompt=None; primary_clear=None; primary_abort=None
compact_request=None
def requests():
 global primary_prompt,primary_clear,primary_abort,compact_request
 if mode not in ('primary-controls', 'compact-recovery'):
  yield from sys.stdin; return
 import select
 buffered=b''
 while True:
  if compact_request is not None and (root/'release-compact').exists():
   emit({'type':'compaction_end','result':{}})
   emit({'type':'response','id':compact_request['id'],'command':'compact','success':True})
   compact_request=None
  if primary_clear is not None and (root/'release-primary-clear').exists():
   emit({'type':'response','id':primary_clear['id'],'command':'clear_queue','success':True,'data':{}}); primary_clear=None
  if primary_abort is not None and (root/'release-primary-abort').exists():
   emit({'type':'response','id':primary_abort['id'],'command':'abort','success':True,'data':{}})
   emit({'type':'agent_settled','aborted':True}); primary_abort=None
  if primary_prompt is not None and (root/'release-primary-fact').exists():
   emit({'type':'response','id':primary_prompt['id'],'command':'prompt','success':True,'data':{'disposition':'queued'}}); primary_prompt=None
  if b'\n' in buffered:
   line,buffered=buffered.split(b'\n',1); yield line.decode('utf-8'); continue
  if not select.select([sys.stdin.fileno()],[],[],.005)[0]: continue
  chunk=os.read(sys.stdin.fileno(),65536)
  if not chunk: return
  buffered+=chunk
for line in requests():
 r=json.loads(line); command=r['type']; data={}
 with root.joinpath(name+'-wire').open('a') as log: log.write(json.dumps(r)+'\n')
 if command=='prompt':
  text=r.get('message','')
  if text.startswith('/velune-projection-sync '):
   if not branched and gate('source'):
    emit({'type':'response','id':r['id'],'command':command,'success':False,'error':'synthetic source rejection'}); continue
   if branched and gate('snapshot'):
    emit({'type':'response','id':r['id'],'command':command,'success':False,'error':'synthetic snapshot rejection'}); continue
   messages=history['messages']
   name_file=root/'name-written'
   title=name_file.read_text() if name_file.exists() else 'Synthetic'
   status('velune.session-metadata',{'requestId':r['id'],'snapshotVersion':1,'messages':messages,'messageIds':[m['id'] for m in messages],'identityConfirmations':[],'sessionFile':str(source),'name':title,'cwd':str(root)})
   data={'disposition':'handled'}
  elif text.startswith('/velune-background-work '):
   status('velune.background-work-v1',{'version':1,'kind':'unsupported','supported':False,'tasks':[]}); data={'disposition':'handled'}
  elif text.startswith('/velune-rewind '):
   if mode.startswith('fallback'):
    status('velune.rewind',{'entryId':text.split(' ',1)[1],'fallback':'synthetic safe precheck','unmodified':True,'reason':'prefix_context_edit'})
    emit({'type':'response','id':r['id'],'command':command,'success':True,'data':{'disposition':'handled'}}); continue
   if gate('branch'):
    emit({'type':'response','id':r['id'],'command':command,'success':False,'error':'synthetic branch rejection'}); continue
   history['messages']=history['messages'][:2]; branched=True
   root.joinpath(name+'-history.json').write_text(json.dumps(history))
   status('velune.rewind',{'entryId':text.split(' ',1)[1]}); data={'disposition':'handled'}
  else:
   if text=='synthetic fail connection':
    print('{',flush=True); (root/'failed-emitted').touch(); continue
   if mode=='writer' and name=='edit':
    (root/'writer-prompt-read').touch(); continue
   if mode=='primary-controls' and name=='edit':
    primary_prompt=r
    emit({'type':'agent_start'})
    message={'id':'pi:synthetic:replacement:0','role':'user','content':text}
    history['messages'].append(message); root.joinpath(name+'-history.json').write_text(json.dumps(history))
    emit({'type':'message_start','message':{'role':'user','content':text}})
    emit({'type':'extension_ui_request','method':'confirm','id':'primary-confirm','title':'Synthetic same-session question'})
    (root/'primary-prompt-read').touch(); continue
   if mode in ('composed-send','composed-reject') and name=='edit':
    emit({'type':'agent_start'})
    (root/'entered-composed-send').touch()
    while not (root/'release-composed-send').exists(): time.sleep(.005)
    if mode=='composed-reject':
     emit({'type':'response','id':r['id'],'command':command,'success':False,'error':'synthetic prompt rejection'}); continue
    emit({'type':'response','id':r['id'],'command':command,'success':True,'data':{'disposition':'queued'}}); continue
   if mode in ('prompt-budget-model','prompt-budget-form') and name=='edit':
    emit({'type':'agent_start'})
    message={'id':'pi:synthetic:replacement:0','role':'user','content':text}
    history['messages'].append(message); root.joinpath(name+'-history.json').write_text(json.dumps(history))
    native_event={'role':'user','content':text}
    emit({'type':'message_start','message':native_event}); emit({'type':'message_end','message':native_event})
    if mode=='prompt-budget-form': emit({'type':'extension_ui_request','method':'confirm','id':'edit-confirm','title':'Synthetic pending human confirmation'})
    (root/'entered-long-prompt').touch()
    while not (root/'release-preflight').exists(): time.sleep(.005)
    emit({'type':'response','id':r['id'],'command':command,'success':True,'data':{'disposition':'queued'}})
    (root/'preflight-ack').touch()
    while not (root/'release-native-end').exists(): time.sleep(.005)
    emit({'type':'agent_settled'}); (root/'native-finished').touch(); continue
   elif mode in ('prompt-success-eof','prompt-unmatched-eof','prompt-eof') and name=='edit':
    if mode!='prompt-eof': emit({'type':'response','id':r['id'] if mode=='prompt-success-eof' else 'unrelated-id','command':command,'success':True,'data':{}})
    sys.exit(0)
   else:
    emit({'type':'agent_start'})
    message={'id':'pi:synthetic:replacement:0','role':'user','content':text}
    history['messages'].append(message); root.joinpath(name+'-history.json').write_text(json.dumps(history))
    emit({'type':'message_start','message':message}); emit({'type':'message_end','message':message})
    # This session has accepted the turn and then asks a runtime question.
    # awaiting_user is only an intermediate preflight receipt, not acceptance.
    if name=='other': emit({'type':'extension_ui_request','method':'confirm','id':'other-confirm','title':'Synthetic question'}); data={'disposition':'queued'}
 elif command=='set_model':
  if name=='fork':
   (root/'entered-fallback-model').touch()
   while not (root/'release-model').exists(): time.sleep(.005)
  if gate('model'):
   emit({'type':'response','id':r['id'],'command':command,'success':False,'error':'synthetic model rejection'}); continue
  if mode=='writer' and name=='edit':
   emit({'type':'response','id':r['id'],'command':command,'success':True,'data':{}})
   (root/'entered-writer').touch()
   while not (root/'release').exists(): time.sleep(.005)
   continue
 elif command=='compact':
  emit({'type':'compaction_start'})
  if mode=='compact-disconnect':
   (root/'entered-compact').touch()
   while not (root/'release-compact').exists(): time.sleep(.005)
   sys.exit(0)
  compact_request=r; (root/'entered-compact').touch(); continue
 elif command=='set_session_name':
  (root/'name-written').write_text(r['name'])
 elif command=='get_state': data={'sessionFile':str(source),'sessionName':'Synthetic'}
 elif command=='get_messages': data={'messages':history['messages']}
 elif command=='extension_ui_response':
  if mode=='primary-controls' and name=='edit':
   assert r['id']=='primary-confirm' and r['confirmed'] is True
   (root/'primary-answered').touch()
  (root/(name+'-replied')).touch(); continue
 elif command=='clear_queue':
  if mode=='primary-controls' and name=='edit':
   primary_clear=r; (root/'primary-clear-read').touch(); continue
 elif command=='abort':
  if mode=='writer' and name=='edit':
   (root/'writer-abort-read').touch(); sys.exit(0)
  if mode=='primary-controls' and name=='edit':
   primary_abort=r; (root/'primary-abort-read').touch(); continue
  (root/(name+'-cancelled')).touch(); emit({'type':'agent_settled','aborted':True})
 emit({'type':'response','id':r['id'],'command':command,'success':True,'data':data})
"#).unwrap();
        Self(root)
    }
    fn open(&self) -> ApplicationService {
        let service = ApplicationService::open(Options {
            home_directory: self.0.join("app"),
            resources_directory: self.0.to_path_buf(),
        })
        .unwrap();
        service
            .execute(
                ClientViewId("edit".into()),
                Command::UpsertRuntime {
                    runtime: crate::config::RuntimeInstance {
                        enabled: true,
                        id: "pi".into(),
                        name: "Synthetic Pi".into(),
                        type_id: crate::conversation::RuntimeKind::Pi,
                        gateway_id: "default".into(),
                        settings: BTreeMap::from([
                            (
                                "binary".into(),
                                self.0.join("pi").to_string_lossy().into_owned(),
                            ),
                            (
                                "nodeBinary".into(),
                                self.0.join("node").to_string_lossy().into_owned(),
                            ),
                            ("agentDir".into(), self.0.to_string_lossy().into_owned()),
                            ("sessionDir".into(), self.0.to_string_lossy().into_owned()),
                        ]),
                    },
                },
            )
            .unwrap();
        service
    }
    fn id(&self, name: &str) -> String {
        format!("pi:{}", self.0.join(format!("{name}.jsonl")).display())
    }
    fn wait(&self, name: &str) {
        let end = Instant::now() + Duration::from_secs(6);
        while !self.0.join(name).exists() {
            assert!(Instant::now() < end, "helper did not enter {name}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn requests(&self, name: &str) -> Vec<serde_json::Value> {
        fs::read_to_string(self.0.join(format!("{name}-wire")))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for marker in [
            "release",
            "release-fork",
            "release-model",
            "release-preflight",
            "release-native-end",
            "release-primary-clear",
            "release-primary-abort",
            "release-primary-fact",
            "release-composed-send",
            "release-compact",
        ] {
            let _ = fs::write(self.0.join(marker), []);
        }
        let _ = fs::remove_file(self.0.join("hold-index"));
    }
}
fn close_after_cleanup(service: &ApplicationService) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match service.close(true) {
            Ok(()) => return,
            Err(error) if error.code == "busy" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("service cleanup did not finish: {error}"),
        }
    }
}

fn later(
    service: &ApplicationService,
    view: &str,
    command: Command,
) -> mpsc::Receiver<Result<Response, Failure>> {
    let (reply, result) = mpsc::channel();
    let service = service.clone();
    let view = ClientViewId(view.into());
    std::thread::spawn(move || {
        let _ = reply.send(service.execute(view, command));
    });
    result
}
fn snapshot(service: &ApplicationService, id: String) -> crate::DeliveredSnapshot {
    let Response::ReadSnapshot(result) = service
        .execute(
            ClientViewId("observer".into()),
            Command::ReadSnapshot {
                runtime_id: "pi".into(),
                conversation_id: id,
            },
        )
        .unwrap()
    else {
        panic!("snapshot response")
    };
    result.snapshot.unwrap()
}

#[test]
fn pi_compaction_live_rename_does_not_read_the_native_index() {
    let fixture = Fixture::new("compact-recovery", false);
    let service = fixture.open();
    let id = fixture.id("edit");
    let initial = snapshot(&service, id.clone());
    assert!(initial.follow_up.can_compact);
    service
        .execute(
            ClientViewId("edit".into()),
            Command::CompactConversation {
                runtime_id: "pi".into(),
                conversation_id: id.clone(),
                selection: None,
            },
        )
        .unwrap();
    fixture.wait("entered-compact");
    fs::write(fixture.0.join("hold-index"), []).unwrap();
    let rename = later(
        &service,
        "edit",
        Command::RenameConversation {
            runtime_id: "pi".into(),
            conversation_id: id.clone(),
            title: "Updated during compaction".into(),
        },
    )
    .recv_timeout(Duration::from_secs(2));
    fs::remove_file(fixture.0.join("hold-index")).unwrap();
    let Response::RenameConversation(configuration) = rename
        .expect("live rename must not wait for the session index")
        .unwrap()
    else {
        panic!("rename response")
    };
    assert!(!fixture.0.join("entered-index").exists());
    assert!(configuration.conversations.iter().any(|row| {
        row.conversation.id == id
            && row.conversation.title.display_text() == "Updated during compaction"
    }));
    fixture.wait("name-written");
    fs::write(fixture.0.join("release-compact"), []).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let current = snapshot(&service, id.clone());
        if current.actions.can_send {
            assert_eq!(
                current.conversation.title.display_text(),
                "Updated during compaction"
            );
            assert_eq!(current.run_state, RunState::Idle);
            break;
        }
        assert!(Instant::now() < deadline, "Pi compaction did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    close_after_cleanup(&service);
}

#[test]
fn pi_compaction_disconnect_preserves_failure_and_allows_a_new_turn() {
    let fixture = Fixture::new("compact-disconnect", false);
    let service = fixture.open();
    let id = fixture.id("edit");
    let initial = snapshot(&service, id.clone());
    assert!(initial.follow_up.can_compact);
    service
        .execute(
            ClientViewId("edit".into()),
            Command::CompactConversation {
                runtime_id: "pi".into(),
                conversation_id: id.clone(),
                selection: None,
            },
        )
        .unwrap();
    fixture.wait("entered-compact");
    fs::write(fixture.0.join("release-compact"), []).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let current = snapshot(&service, id.clone());
        if current.run_state == RunState::Failed && current.actions.can_send {
            assert_eq!(
                current.actions.input_access,
                velune_conversation::InputAccess::Editable
            );
            assert!(
                current
                    .messages
                    .iter()
                    .any(|message| message.blocks.iter().any(|block| {
                        matches!(block, crate::DeliveredBlock::ContextCompaction { text }
                    if text.contains("上下文压缩已中断"))
                    }))
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "session remained unwritable after compact transport failure"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    service
        .execute(
            ClientViewId("edit".into()),
            Command::SendTurn {
                submission_id: None,
                source_runtime_id: "pi".into(),
                conversation_id: id,
                target_runtime_id: "pi".into(),
                selection: None,
                turn: crate::ComposedTurn {
                    text: "Explicit follow-up after failed compact".into(),
                    attachments: vec![],
                    effort: None,
                    permission_mode: None,
                },
            },
        )
        .unwrap();
    assert_eq!(
        fixture
            .requests("edit")
            .iter()
            .filter(|request| request["message"] == "Explicit follow-up after failed compact")
            .count(),
        1
    );
    close_after_cleanup(&service);
}

fn start_edit(
    service: &ApplicationService,
    fixture: &Fixture,
    text: String,
) -> mpsc::Receiver<Result<Response, Failure>> {
    let current = snapshot(service, fixture.id("edit"));
    let editable = current
        .follow_up
        .editable
        .iter()
        .find(|item| item.message_id.ends_with("edited-user:0"))
        .unwrap();
    later(
        service,
        "edit",
        Command::EditMessage {
            runtime_id: "pi".into(),
            conversation_id: fixture.id("edit"),
            message_id: editable.message_id.clone(),
            placement: EditPlacement::InPlace,
            target_runtime_id: None,
            expected_revision: editable.revision.clone(),
            selection: Some(crate::ModelSelection::Native {
                provider_id: Some("synthetic".into()),
                model_id: "alpha".into(),
            }),
            turn: crate::ComposedTurn {
                text,
                attachments: vec![],
                effort: None,
                permission_mode: None,
            },
        },
    )
}
fn assert_no_successors(fixture: &Fixture, phase: &str) {
    let wire = fixture.requests("edit");
    assert!(
        !wire
            .iter()
            .any(|r| r["type"] == "prompt"
                && r["message"].as_str().is_some_and(|s| !s.starts_with('/'))),
        "replacement dispatched before {phase} confirmation"
    );
    if matches!(phase, "branch" | "snapshot") {
        assert!(!wire.iter().any(|r| r["type"] == "set_model"));
    }
}
fn other_session_progress(service: &ApplicationService, fixture: &Fixture) {
    let other = snapshot(service, fixture.id("other"));
    let interaction = other
        .pending_interactions
        .first()
        .expect("other session question");
    let velune_conversation::InteractionKind::UserInput { questions } = &interaction.kind else {
        panic!("Pi confirm must project a question, not an approval decision");
    };
    assert_eq!(questions.len(), 1);
    let question = &questions[0];
    let option = question
        .options
        .iter()
        .find(|option| option.id == "yes")
        .expect("native confirm affirmative option");
    let replies = || {
        fixture
            .requests("other")
            .into_iter()
            .filter(|request| request["type"] == "extension_ui_response")
            .collect::<Vec<_>>()
    };
    assert!(replies().is_empty());
    // A malformed answer is rejected locally before any ticket or wire. The
    // same question must remain answerable with its actual public question ID.
    let rejected = later(
        service,
        "other",
        Command::ReplyRuntimeInteraction {
            runtime_id: "pi".into(),
            conversation_id: fixture.id("other"),
            interaction_id: interaction.id.clone(),
            reply: RuntimeInteractionReply::Decision {
                option_id: option.id.clone(),
            },
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .err()
    .expect("decision is not a Pi question answer");
    assert_eq!(rejected.kind, FailureKind::Invalid);
    assert_eq!(rejected.code, "invalid");
    assert!(
        replies().is_empty(),
        "local validation rejection dispatched native reply wire"
    );
    assert!(
        snapshot(service, fixture.id("other"))
            .pending_interactions
            .iter()
            .any(|pending| pending.id == interaction.id),
        "local rejection consumed the pending question"
    );
    later(
        service,
        "other",
        Command::ReplyRuntimeInteraction {
            runtime_id: "pi".into(),
            conversation_id: fixture.id("other"),
            interaction_id: interaction.id.clone(),
            reply: RuntimeInteractionReply::Answers {
                answers: vec![velune_conversation::InteractionAnswer {
                    question_id: question.id.clone(),
                    values: vec![option.id.clone()],
                }],
            },
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .unwrap();
    fixture.wait("other-replied");
    let native = replies();
    assert_eq!(
        native.len(),
        1,
        "correcting local validation must send exactly one native answer"
    );
    assert_eq!(native[0]["id"], interaction.id);
    assert_eq!(native[0]["confirmed"], true);
    later(
        service,
        "other",
        Command::Cancel {
            runtime_id: "pi".into(),
            conversation_id: fixture.id("other"),
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .unwrap();
    fixture.wait("other-cancelled");
}
fn prepare_other(service: &ApplicationService, fixture: &Fixture) {
    snapshot(service, fixture.id("other"));
    later(
        service,
        "other",
        Command::SendTurn {
            submission_id: None,
            source_runtime_id: "pi".into(),
            conversation_id: fixture.id("other"),
            target_runtime_id: "pi".into(),
            selection: None,
            turn: crate::ComposedTurn {
                text: "synthetic other input".into(),
                attachments: vec![],
                effort: None,
                permission_mode: None,
            },
        },
    )
    .recv_timeout(Duration::from_secs(4))
    .expect("synthetic other session must acknowledge the accepted prompt")
    .unwrap();
    let end = Instant::now() + Duration::from_secs(4);
    while snapshot(service, fixture.id("other"))
        .pending_interactions
        .is_empty()
    {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn pi_edit_dependency_gates_keep_other_session_read_reply_cancel_progressing() {
    for phase in ["branch", "snapshot", "model"] {
        let fixture = Fixture::new(phase, false);
        let service = fixture.open();
        prepare_other(&service, &fixture);
        let reply = start_edit(&service, &fixture, "replacement".into());
        fixture.wait(&format!("entered-{phase}"));
        assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert_no_successors(&fixture, phase);
        other_session_progress(&service, &fixture);
        fs::write(fixture.0.join("release"), []).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let view = snapshot(&service, fixture.id("edit"));
            if view.run_state == RunState::Running {
                assert!(
                    view.messages
                        .iter()
                        .all(|message| !message.id.contains("edited-user")
                            && !message.id.contains("future-reply")),
                    "intermediate Running snapshot contains abandoned native messages"
                );
            }
            if view.messages.iter().any(|message| {
                message.role == MessageRole::User && message.id.contains("replacement")
            }) {
                break;
            }
            assert!(Instant::now() < end, "native replacement echo not observed");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(
            reply.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(),
            Response::EditMessage(_)
        ));
        let replacement = fixture
            .requests("edit")
            .into_iter()
            .filter(|r| r["type"] == "prompt" && r["message"] == "replacement")
            .count();
        assert_eq!(replacement, 1);
        close_after_cleanup(&service);
    }
}

#[test]
fn pi_edit_snapshot_and_model_rejection_report_partial_without_default_prompt() {
    for phase in ["snapshot", "model"] {
        let fixture = Fixture::new(phase, true);
        let service = fixture.open();
        prepare_other(&service, &fixture);
        let reply = start_edit(&service, &fixture, "replacement".into());
        fixture.wait(&format!("entered-{phase}"));
        assert_no_successors(&fixture, phase);
        other_session_progress(&service, &fixture);
        fs::write(fixture.0.join("release"), []).unwrap();
        let error = reply
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .err()
            .expect("partial failure");
        assert!(
            error.message.contains("尚未发送替换内容"),
            "{}",
            error.message
        );
        assert_no_successors(&fixture, phase);
        close_after_cleanup(&service);
    }
}

#[test]
fn pi_edit_blocked_writer_is_not_native_acceptance_and_cancel_does_not_replay() {
    let fixture = Fixture::new("writer", false);
    let service = fixture.open();
    let reply = start_edit(&service, &fixture, "x".repeat(1024 * 1024));
    fixture.wait("entered-writer");
    assert!(matches!(
        reply.recv_timeout(Duration::from_millis(150)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    later(
        &service,
        "observer",
        Command::ReadRuntimeHomeDirectory {
            runtime_id: "pi".into(),
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .unwrap();
    let cancel = later(
        &service,
        "edit",
        Command::Cancel {
            runtime_id: "pi".into(),
            conversation_id: fixture.id("edit"),
        },
    );
    assert!(
        matches!(
            cancel.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "reserved admission is not a native stop ACK through a blocked pipe"
    );
    assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
    // Release only after the no-false-acceptance assertions. The peer reads
    // the whole original prompt, ACKs ClearQueue, then reads Abort and exits
    // without its ACK: both remaining outcomes are genuinely indeterminate.
    fs::write(fixture.0.join("release"), []).unwrap();
    fixture.wait("writer-abort-read");
    assert_eq!(
        cancel
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .err()
            .expect("missing native Abort ACK")
            .code,
        "runtime_operation_indeterminate"
    );
    assert_eq!(
        reply
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .err()
            .expect("missing native preflight ACK")
            .code,
        "runtime_operation_indeterminate"
    );
    assert!(fixture.0.join("writer-prompt-read").exists());
    assert_eq!(
        fixture
            .requests("edit")
            .iter()
            .filter(|request| request["type"] == "abort")
            .count(),
        1
    );
    close_after_cleanup(&service);
}

#[test]
fn pi_edit_source_switch_fences_late_branch_and_never_dispatches_old_replacement() {
    let fixture = Fixture::new("branch", false);
    let service = fixture.open();
    let reply = start_edit(&service, &fixture, "replacement".into());
    fixture.wait("entered-branch");
    assert_no_successors(&fixture, "branch");
    let changed = fixture.0.join("changed-source");
    fs::create_dir(&changed).unwrap();
    later(
        &service,
        "configuration",
        Command::UpsertRuntime {
            runtime: crate::config::RuntimeInstance {
                enabled: true,
                id: "pi".into(),
                name: "Synthetic Pi".into(),
                type_id: crate::conversation::RuntimeKind::Pi,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    (
                        "binary".into(),
                        fixture.0.join("pi").to_string_lossy().into_owned(),
                    ),
                    (
                        "nodeBinary".into(),
                        fixture.0.join("node").to_string_lossy().into_owned(),
                    ),
                    ("agentDir".into(), changed.to_string_lossy().into_owned()),
                    ("sessionDir".into(), changed.to_string_lossy().into_owned()),
                ]),
            },
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .unwrap();
    let error = reply
        .recv_timeout(Duration::from_secs(4))
        .unwrap()
        .err()
        .expect("old-source edit must end");
    assert!(
        error.message.contains("来源") || error.message.contains("配置"),
        "{}",
        error.message
    );
    fs::write(fixture.0.join("release"), []).unwrap();
    assert_no_successors(&fixture, "branch");
    close_after_cleanup(&service);
}

#[test]
fn pi_delete_waits_for_source_confirmation_without_prompt_or_owner_wait() {
    for truncate in [false, true] {
        let fixture = Fixture::new("source", false);
        let service = fixture.open();
        prepare_other(&service, &fixture);
        let view = snapshot(&service, fixture.id("edit"));
        let choice = view
            .follow_up
            .editable
            .iter()
            .find(|item| item.message_id.contains("edited-user"))
            .unwrap();
        let reply = later(
            &service,
            "delete",
            Command::DeleteUserMessage {
                runtime_id: "pi".into(),
                conversation_id: fixture.id("edit"),
                message_id: choice.message_id.clone(),
                truncate,
                expected_revision: choice.revision.clone().unwrap(),
            },
        );
        fixture.wait("entered-source");
        assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert!(!fixture.0.join("deleted").exists());
        other_session_progress(&service, &fixture);
        fs::write(fixture.0.join("release"), []).unwrap();
        let Response::DeleteUserMessage(result) =
            reply.recv_timeout(Duration::from_secs(5)).unwrap().unwrap()
        else {
            panic!("delete reply")
        };
        let fresh = result.snapshot.unwrap();
        assert!(
            fresh
                .messages
                .iter()
                .all(|message| !message.id.contains("edited-user"))
        );
        assert_eq!(
            fresh
                .messages
                .iter()
                .filter(|message| message.role == MessageRole::Assistant)
                .count(),
            if truncate { 1 } else { 2 }
        );
        assert!(fixture.requests("edit").iter().all(|request| {
            request["type"] != "set_model"
                && request["message"]
                    .as_str()
                    .is_none_or(|text| text.starts_with('/'))
        }));
        close_after_cleanup(&service);
    }
}

#[test]
fn pi_failed_cancel_keeps_real_child_and_home_until_bounded_cleanup_capacity_returns() {
    use crate::local::execution_workers::cleanup_test_gate;
    let fixtures = [
        Fixture::new("healthy", false),
        Fixture::new("healthy", false),
        Fixture::new("healthy", false),
    ];
    let service = fixtures[0].open();
    let mut ids = Vec::new();
    let mut gates = Vec::new();
    for (index, fixture) in fixtures.iter().enumerate() {
        let runtime_id = format!("cleanup-pi-{index}");
        service
            .execute(
                ClientViewId("configuration".into()),
                Command::UpsertRuntimeWithExecution {
                    runtime: crate::config::RuntimeInstance {
                        enabled: true,
                        id: runtime_id.clone(),
                        name: "Synthetic cleanup Pi".into(),
                        type_id: crate::conversation::RuntimeKind::Pi,
                        gateway_id: "default".into(),
                        settings: BTreeMap::from([
                            (
                                "binary".into(),
                                fixture.0.join("pi").to_string_lossy().into_owned(),
                            ),
                            (
                                "nodeBinary".into(),
                                fixture.0.join("node").to_string_lossy().into_owned(),
                            ),
                            ("agentDir".into(), fixture.0.to_string_lossy().into_owned()),
                            (
                                "sessionDir".into(),
                                fixture.0.to_string_lossy().into_owned(),
                            ),
                        ]),
                    },
                    execution: crate::RuntimeExecution {
                        home: crate::RuntimeHome::Managed { remote_root: None },
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        let id = format!("{runtime_id}:{}", fixture.0.join("edit.jsonl").display());
        service
            .execute(
                ClientViewId("cleanup".into()),
                Command::ReadSnapshot {
                    runtime_id: runtime_id.clone(),
                    conversation_id: id.clone(),
                },
            )
            .unwrap();
        let gate = cleanup_test_gate::register(&fixture.0.join("pi"));
        let _ = service.execute(
            ClientViewId("cleanup".into()),
            Command::SendTurn {
                submission_id: None,
                source_runtime_id: runtime_id.clone(),
                conversation_id: id.clone(),
                target_runtime_id: runtime_id.clone(),
                selection: None,
                turn: crate::ComposedTurn {
                    text: "synthetic fail connection".into(),
                    attachments: vec![],
                    effort: None,
                    permission_mode: None,
                },
            },
        );
        fixture.wait("failed-emitted");
        if index < 2 {
            gate.0.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        ids.push((runtime_id, id));
        gates.push(gate);
    }
    assert!(
        matches!(gates[2].0.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "cleanup exceeded per-location bound"
    );
    let third = &fixtures[2];
    let pid = fs::read_to_string(third.0.join("edit-pid")).unwrap();
    let alive = || {
        velune_agent_runtime::process::command("/bin/kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success()
    };
    assert!(alive(), "polling a failure must not reap the actual child");
    let home = PathBuf::from(fs::read_to_string(third.0.join("edit-home")).unwrap());
    assert!(home.is_dir());
    let before = third.requests("edit").len();
    let cancel = later(
        &service,
        "cleanup",
        Command::Cancel {
            runtime_id: ids[2].0.clone(),
            conversation_id: ids[2].1.clone(),
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap();
    assert!(
        cancel.is_err(),
        "failed cancel must not invent native acceptance"
    );
    assert_eq!(
        third.requests("edit").len(),
        before,
        "failed cancel dispatched new wire"
    );
    assert!(
        service
            .execute(
                ClientViewId("configuration".into()),
                Command::DeleteRuntime {
                    id: ids[2].0.clone()
                }
            )
            .is_err(),
        "real child/HOME lease released while cleanup capacity was full"
    );
    prepare_other(&service, &fixtures[0]);
    other_session_progress(&service, &fixtures[0]);
    gates[0].1.send(()).unwrap();
    gates[2]
        .0
        .recv_timeout(Duration::from_secs(3))
        .expect("cleanup did not retry after capacity returned");
    assert!(alive(), "cleanup gate still owns actual child");
    gates[2].1.send(()).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while alive() {
        assert!(
            Instant::now() < end,
            "cleanup did not kill/reap its exact child"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if service
            .execute(
                ClientViewId("configuration".into()),
                Command::DeleteRuntime {
                    id: ids[2].0.clone(),
                },
            )
            .is_ok()
        {
            break;
        }
        assert!(
            Instant::now() < end,
            "completed Pi cleanup did not release runtime lease"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    gates[1].1.send(()).unwrap();
    close_after_cleanup(&service);
}

#[test]
fn pi_edit_generic_branch_rejection_never_creates_a_fallback_copy() {
    let fixture = Fixture::new("branch", true);
    let service = fixture.open();
    let reply = start_edit(&service, &fixture, "replacement".into());
    fixture.wait("entered-branch");
    assert_no_successors(&fixture, "branch");
    fs::write(fixture.0.join("release"), []).unwrap();
    assert!(reply.recv_timeout(Duration::from_secs(4)).unwrap().is_err());
    assert!(
        !fixture.0.join("fork-count").exists(),
        "generic rejection did not prove an unmodified source"
    );
    assert_no_successors(&fixture, "branch");
    close_after_cleanup(&service);
}

#[test]
fn pi_edit_safe_fallback_commits_same_logical_tail_before_model_ack_and_one_prompt() {
    let fixture = Fixture::new("fallback", false);
    let service = fixture.open();
    let original = fs::read(fixture.0.join("edit.jsonl")).unwrap();
    let original_history = fs::read(fixture.0.join("edit-history.json")).unwrap();
    let reply = start_edit(&service, &fixture, "replacement".into());
    fixture.wait("entered-fallback-model");
    assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
    let links: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.0.join("app/conversation-links.json")).unwrap())
            .unwrap();
    assert_eq!(links["links"][0]["id"], fixture.id("edit"));
    assert_eq!(links["links"][0]["segments"].as_array().unwrap().len(), 2);
    assert!(
        fixture
            .requests("fork")
            .iter()
            .all(|request| request["message"] != "replacement")
    );
    fs::write(fixture.0.join("release-model"), []).unwrap();
    let Response::EditMessage(result) =
        reply.recv_timeout(Duration::from_secs(5)).unwrap().unwrap()
    else {
        panic!("fallback edit reply")
    };
    let final_view = result.snapshot.unwrap();
    assert_eq!(final_view.conversation.id, fixture.id("edit"));
    assert_eq!(
        final_view
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .count(),
        2,
        "fork prefix displayed twice"
    );
    assert!(
        final_view
            .messages
            .iter()
            .all(|message| !message.id.contains("edited-user")
                && !message.id.contains("future-reply"))
    );
    assert_eq!(
        fixture
            .requests("fork")
            .iter()
            .filter(|request| request["message"] == "replacement")
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("fork-count")).unwrap(),
        "fork\n"
    );
    assert_eq!(fs::read(fixture.0.join("edit.jsonl")).unwrap(), original);
    assert_eq!(
        fs::read(fixture.0.join("edit-history.json")).unwrap(),
        original_history
    );
    close_after_cleanup(&service);
}

#[test]
fn pi_edit_fork_failure_preserves_original_association_and_recovery_without_replay() {
    for mode in [
        "fallback-association-fail",
        "fallback-unknown",
        "fallback-sync-fail",
    ] {
        let fixture = Fixture::new(mode, false);
        let service = fixture.open();
        let original = fs::read(fixture.0.join("edit.jsonl")).unwrap();
        let reply = start_edit(&service, &fixture, "replacement".into());
        let error = reply
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .err()
            .expect("fork failure");
        assert!(
            error.message.contains("副本") || error.message.contains("unknown"),
            "{}",
            error.message
        );
        assert!(!fixture.0.join("app/conversation-links.json").exists());
        assert!(
            fixture.0.join("fork.jsonl").exists(),
            "recovery copy was deleted"
        );
        assert_eq!(
            fs::read_to_string(fixture.0.join("fork-count")).unwrap(),
            "fork\n"
        );
        assert_eq!(fs::read(fixture.0.join("edit.jsonl")).unwrap(), original);
        assert!(
            fixture
                .requests("fork")
                .iter()
                .all(|request| request["message"] != "replacement")
        );
        close_after_cleanup(&service);
    }
}

#[test]
fn pi_edit_fallback_source_switch_never_installs_or_replays_a_created_copy() {
    let fixture = Fixture::new("fallback-source", false);
    let service = fixture.open();
    let original = fs::read(fixture.0.join("edit.jsonl")).unwrap();
    let reply = start_edit(&service, &fixture, "replacement".into());
    fixture.wait("entered-fork");
    let changed = fixture.0.join("changed-source");
    fs::create_dir(&changed).unwrap();
    later(
        &service,
        "configuration",
        Command::UpsertRuntime {
            runtime: crate::config::RuntimeInstance {
                enabled: true,
                id: "pi".into(),
                name: "Synthetic Pi".into(),
                type_id: crate::conversation::RuntimeKind::Pi,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    (
                        "binary".into(),
                        fixture.0.join("pi").to_string_lossy().into_owned(),
                    ),
                    (
                        "nodeBinary".into(),
                        fixture.0.join("node").to_string_lossy().into_owned(),
                    ),
                    ("agentDir".into(), changed.to_string_lossy().into_owned()),
                    ("sessionDir".into(), changed.to_string_lossy().into_owned()),
                ]),
            },
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .unwrap();
    assert!(reply.recv_timeout(Duration::from_secs(4)).unwrap().is_err());
    fs::write(fixture.0.join("release-fork"), []).unwrap();
    assert!(fixture.0.join("fork.jsonl").exists());
    assert!(!fixture.0.join("app/conversation-links.json").exists());
    assert_eq!(
        fs::read_to_string(fixture.0.join("fork-count")).unwrap(),
        "fork\n"
    );
    assert_eq!(fs::read(fixture.0.join("edit.jsonl")).unwrap(), original);
    assert!(fixture.requests("fork").is_empty());
    close_after_cleanup(&service);
}

#[test]
fn pi_prompt_terminal_response_without_echo_keeps_acceptance_and_cleanup_separate() {
    use crate::local::execution_workers::cleanup_test_gate;
    for mode in ["prompt-success-eof", "prompt-unmatched-eof", "prompt-eof"] {
        let fixture = Fixture::new(mode, false);
        let service = fixture.open();
        let (entered, release) = cleanup_test_gate::register(&fixture.0.join("pi"));
        let reply = start_edit(&service, &fixture, "replacement".into());
        let outcome = reply.recv_timeout(Duration::from_secs(5)).unwrap();
        if mode == "prompt-success-eof" {
            match outcome {
                Ok(Response::EditMessage(result)) => assert_eq!(
                    result.submission_receipt,
                    Some(crate::SubmissionReceipt::Sending)
                ),
                Err(error) => {
                    assert_eq!(error.code, "message_delivery_accepted", "{}", error.message)
                }
                _ => panic!("wrong edit terminal response"),
            }
        } else {
            let error = outcome.err().expect("EOF must not invent input acceptance");
            assert_ne!(error.code, "message_delivery_accepted");
        }
        entered.recv_timeout(Duration::from_secs(3)).unwrap();
        later(
            &service,
            "reader",
            Command::ReadRuntimeHomeDirectory {
                runtime_id: "pi".into(),
            },
        )
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
        assert!(
            service
                .execute(
                    ClientViewId("configuration".into()),
                    Command::DeleteRuntime { id: "pi".into() }
                )
                .is_err(),
            "cleanup released actual runtime lease before worker completion"
        );
        assert_eq!(
            fixture
                .requests("edit")
                .iter()
                .filter(|r| r["message"] == "replacement")
                .count(),
            1
        );
        release.send(()).unwrap();
        let end = Instant::now() + Duration::from_secs(4);
        loop {
            if service
                .execute(
                    ClientViewId("configuration".into()),
                    Command::DeleteRuntime { id: "pi".into() },
                )
                .is_ok()
            {
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        close_after_cleanup(&service);
    }
}

#[test]
fn pi_composed_send_waits_for_exact_prompt_receipt_and_rejects_before_delivery() {
    use std::sync::mpsc::TryRecvError;

    for (mode, rejected) in [("composed-send", false), ("composed-reject", true)] {
        let fixture = Fixture::new(mode, false);
        let service = fixture.open();
        let id = fixture.id("edit");
        let _ = snapshot(&service, id.clone());
        let reply = later(
            &service,
            "edit",
            Command::SendTurn {
                submission_id: Some("pi-composed-acceptance-test".into()),
                source_runtime_id: "pi".into(),
                conversation_id: id.clone(),
                target_runtime_id: "pi".into(),
                selection: None,
                turn: crate::ComposedTurn {
                    text: "ordinary composed prompt".into(),
                    effort: None,
                    permission_mode: None,
                    attachments: Vec::new(),
                },
            },
        );
        fixture.wait("entered-composed-send");
        assert!(
            matches!(reply.try_recv(), Err(TryRecvError::Empty)),
            "SendTurn replied after local enqueue, before Pi's exact prompt response"
        );

        fs::write(fixture.0.join("release-composed-send"), []).unwrap();
        let outcome = reply.recv_timeout(Duration::from_secs(5)).unwrap();
        if rejected {
            let Err(error) = outcome else {
                panic!("explicit Pi rejection must fail the submission");
            };
            assert_ne!(error.code, "message_delivery_accepted");
            let failed = snapshot(&service, id);
            assert_eq!(failed.run_state, RunState::Failed);
            assert!(
                failed.actions.can_send,
                "rejected input must release Pi's send gate"
            );
        } else {
            let Response::SendTurn(result) = outcome.unwrap() else {
                panic!("expected composed SendTurn response");
            };
            assert_eq!(
                result.submission_receipt,
                Some(crate::SubmissionReceipt::Sending)
            );
            assert!(
                result.submitted_message_id.is_none(),
                "no native echo was fabricated"
            );
        }
        close_after_cleanup(&service);
    }
}

#[test]
fn pi_delete_completed_and_mutating_io_retain_facts_across_cancel_and_source_switch() {
    use crate::local::execution_workers::pi_delete_test_gate;
    for stage in ["mutation", "install"] {
        for action in ["cancel", "source"] {
            let fixture = Fixture::new("healthy", false);
            let service = fixture.open();
            let view = snapshot(&service, fixture.id("edit"));
            let choice = view
                .follow_up
                .editable
                .iter()
                .find(|item| item.message_id.contains("edited-user"))
                .unwrap();
            let mut mutation_release = None;
            let mut install_release = None;
            let entered = if stage == "mutation" {
                let (entered, release) =
                    pi_delete_test_gate::register_mutation(&fixture.0.join("pi"));
                mutation_release = Some(release);
                entered
            } else {
                let (entered, release) =
                    pi_delete_test_gate::register_install(&fixture.0.join("pi"));
                install_release = Some(release);
                entered
            };
            let reply = later(
                &service,
                "delete",
                Command::DeleteUserMessage {
                    runtime_id: "pi".into(),
                    conversation_id: fixture.id("edit"),
                    message_id: choice.message_id.clone(),
                    truncate: false,
                    expected_revision: choice.revision.clone().unwrap(),
                },
            );
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(
                fs::read_to_string(fixture.0.join("delete-count")).unwrap(),
                "delete\n"
            );
            let mut new_id = None;
            if action == "cancel" {
                later(
                    &service,
                    "cancel",
                    Command::Cancel {
                        runtime_id: "pi".into(),
                        conversation_id: fixture.id("edit"),
                    },
                )
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap();
            } else {
                let changed = fixture.0.join("new-source");
                fs::create_dir(&changed).unwrap();
                fs::write(
                    changed.join("other.jsonl"),
                    native_header(&changed, "other"),
                )
                .unwrap();
                fs::write(changed.join("other-history.json"),serde_json::json!({"cwd":changed,"messages":[{"id":"pi:new:sentinel:0","role":"user","content":"new source sentinel"}]}).to_string()).unwrap();
                later(
                    &service,
                    "configuration",
                    Command::UpsertRuntime {
                        runtime: crate::config::RuntimeInstance {
                            enabled: true,
                            id: "pi".into(),
                            name: "Synthetic Pi".into(),
                            type_id: crate::conversation::RuntimeKind::Pi,
                            gateway_id: "default".into(),
                            settings: BTreeMap::from([
                                (
                                    "binary".into(),
                                    fixture.0.join("pi").to_string_lossy().into_owned(),
                                ),
                                (
                                    "nodeBinary".into(),
                                    fixture.0.join("node").to_string_lossy().into_owned(),
                                ),
                                ("agentDir".into(), changed.to_string_lossy().into_owned()),
                                ("sessionDir".into(), changed.to_string_lossy().into_owned()),
                            ]),
                        },
                    },
                )
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap();
                let id = format!("pi:{}", changed.join("other.jsonl").display());
                assert!(
                    snapshot(&service, id.clone())
                        .message_ids
                        .iter()
                        .any(|id| id == "pi:new:sentinel:0")
                );
                new_id = Some(id);
            }
            assert!(
                matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "in-flight mutation lost its final outcome channel"
            );
            if let Some(release) = mutation_release {
                release.send(()).unwrap();
            }
            if let Some(release) = install_release {
                release.release();
            }
            let error = reply
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .err()
                .expect("completed-with-refresh-error required");
            assert_eq!(error.code, "runtime_operation_completed");
            assert!(
                error.message.contains("消息删除已执行"),
                "{}",
                error.message
            );
            assert_eq!(
                fs::read_to_string(fixture.0.join("delete-count")).unwrap(),
                "delete\n"
            );
            let history: serde_json::Value =
                serde_json::from_slice(&fs::read(fixture.0.join("edit-history.json")).unwrap())
                    .unwrap();
            assert!(
                history["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|m| m["id"] != choice.message_id)
            );
            assert!(fixture.requests("edit").iter().all(|r| {
                r["type"] != "set_model"
                    && r["message"].as_str().is_none_or(|text| {
                        text.starts_with("/velune-projection-sync ")
                            || text.starts_with("/velune-background-work ")
                    })
            }));
            if let Some(id) = new_id {
                assert!(
                    snapshot(&service, id)
                        .message_ids
                        .iter()
                        .any(|id| id == "pi:new:sentinel:0"),
                    "old delete history overwrote the new source"
                );
            }
            close_after_cleanup(&service);
        }
    }
}

#[test]
fn pi_edit_authorized_digest_rejects_source_change_before_fork_first_read() {
    let fixture = Fixture::new("fallback-stale-before-read", false);
    let service = fixture.open();
    let reply = start_edit(&service, &fixture, "replacement".into());
    fixture.wait("entered-fork-before-read");
    fs::write(
        fixture.0.join("edit.jsonl"),
        "externally changed after Verify",
    )
    .unwrap();
    fs::write(fixture.0.join("release-fork"), []).unwrap();
    let error = reply
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .err()
        .expect("stale authorization must reject before creation");
    assert_eq!(error.code, "fork_rejected");
    assert!(!fixture.0.join("fork.jsonl").exists());
    assert!(!fixture.0.join("fork-count").exists());
    assert!(!fixture.0.join("app/conversation-links.json").exists());
    assert_no_successors(&fixture, "branch");
    close_after_cleanup(&service);
}

#[test]
fn pi_edit_preparation_deadline_does_not_terminate_written_model_or_human_wait() {
    use crate::local::execution_workers::{cleanup_test_gate, pi_edit_deadline_test_gate};
    for mode in ["prompt-budget-model", "prompt-budget-form"] {
        let fixture = Fixture::new(mode, false);
        let service = fixture.open();
        service
            .execute(
                ClientViewId("configuration".into()),
                Command::UpsertRuntimeWithExecution {
                    runtime: crate::config::RuntimeInstance {
                        enabled: true,
                        id: "pi".into(),
                        name: "Synthetic Pi".into(),
                        type_id: crate::conversation::RuntimeKind::Pi,
                        gateway_id: "default".into(),
                        settings: BTreeMap::from([
                            (
                                "binary".into(),
                                fixture.0.join("pi").to_string_lossy().into_owned(),
                            ),
                            (
                                "nodeBinary".into(),
                                fixture.0.join("node").to_string_lossy().into_owned(),
                            ),
                            ("agentDir".into(), fixture.0.to_string_lossy().into_owned()),
                            (
                                "sessionDir".into(),
                                fixture.0.to_string_lossy().into_owned(),
                            ),
                        ]),
                    },
                    execution: crate::RuntimeExecution {
                        home: crate::RuntimeHome::Managed { remote_root: None },
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        prepare_other(&service, &fixture);
        let reply = start_edit(&service, &fixture, "replacement".into());
        fixture.wait("entered-long-prompt");
        let observed = Instant::now() + Duration::from_secs(2);
        loop {
            let before = snapshot(&service, fixture.id("edit"));
            if before.run_state == RunState::Running
                && (mode != "prompt-budget-form" || !before.pending_interactions.is_empty())
            {
                break;
            }
            assert!(
                Instant::now() < observed,
                "helper model/human wait was not projected"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let (cleanup_entered, _cleanup_release) =
            cleanup_test_gate::register(&fixture.0.join("pi"));
        // Arm only after the real peer has read this one prompt and started
        // model/human work. No 30-second sleep or progress-based renewal.
        let expired = pi_edit_deadline_test_gate::expire_after_prompt(&fixture.0.join("pi"));
        expired.recv_timeout(Duration::from_secs(2)).unwrap();
        other_session_progress(&service, &fixture);
        assert!(
            matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "preparation budget became a native run deadline"
        );
        assert!(
            matches!(cleanup_entered.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "expired preparation started terminal cleanup"
        );
        let pid = fs::read_to_string(fixture.0.join("edit-pid")).unwrap();
        let alive = || {
            velune_agent_runtime::process::command("/bin/kill")
                .args(["-0", pid.trim()])
                .status()
                .unwrap()
                .success()
        };
        assert!(alive(), "preparation timeout killed the Written execution");
        let home = PathBuf::from(fs::read_to_string(fixture.0.join("edit-home")).unwrap());
        assert!(
            home.starts_with(fixture.0.join("app/runtime-homes")) && home.is_dir(),
            "actual synthetic HOME not retained"
        );
        assert!(
            service
                .execute(
                    ClientViewId("configuration".into()),
                    Command::DeleteRuntime { id: "pi".into() }
                )
                .is_err(),
            "runtime/HOME lease released during native wait"
        );
        let wire = fixture.requests("edit");
        assert_eq!(
            wire.iter()
                .filter(|r| r["message"] == "replacement")
                .count(),
            1
        );
        assert!(
            wire.iter()
                .all(|r| r["type"] != "abort" && r["type"] != "clear_queue"),
            "preparation expiry dispatched cancellation wire"
        );
        assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
        fs::write(fixture.0.join("release-preflight"), []).unwrap();
        let Response::EditMessage(result) =
            reply.recv_timeout(Duration::from_secs(5)).unwrap().unwrap()
        else {
            panic!("expected exact preflight acknowledgement")
        };
        assert_eq!(
            result.submission_receipt,
            Some(crate::SubmissionReceipt::Sending)
        );
        assert_eq!(
            result.snapshot.as_ref().unwrap().run_state,
            RunState::Running,
            "queued preflight acknowledgement is not a native turn-end fact"
        );
        assert!(
            alive(),
            "preflight ACK ended a still-running native execution"
        );

        assert_eq!(
            fixture
                .requests("edit")
                .iter()
                .filter(|r| r["message"] == "replacement")
                .count(),
            1
        );
        assert!(
            matches!(cleanup_entered.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "preparation expiry cleaned up after normal completion"
        );
        fs::write(fixture.0.join("release-native-end"), []).unwrap();
        fixture.wait("native-finished");
        let end = Instant::now() + Duration::from_secs(3);
        while snapshot(&service, fixture.id("edit")).run_state == RunState::Running {
            assert!(
                Instant::now() < end,
                "native lifecycle end did not settle the turn"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // Preparation expiry was not allowed to start cleanup. The explicit
        // close below now authorizes it, so release the test-owned fence first.
        drop(_cleanup_release);
        close_after_cleanup(&service);
        assert!(
            !alive(),
            "authorized close left the exact synthetic child alive"
        );
    }
}

#[test]
fn pi_same_session_reply_and_cancel_keep_edit_fact_channel_and_late_acceptance() {
    let fixture = Fixture::new("primary-controls", false);
    let service = fixture.open();
    service
        .execute(
            ClientViewId("configuration".into()),
            Command::UpsertRuntimeWithExecution {
                runtime: crate::config::RuntimeInstance {
                    enabled: true,
                    id: "pi".into(),
                    name: "Synthetic Pi".into(),
                    type_id: crate::conversation::RuntimeKind::Pi,
                    gateway_id: "default".into(),
                    settings: BTreeMap::from([
                        (
                            "binary".into(),
                            fixture.0.join("pi").to_string_lossy().into_owned(),
                        ),
                        (
                            "nodeBinary".into(),
                            fixture.0.join("node").to_string_lossy().into_owned(),
                        ),
                        ("agentDir".into(), fixture.0.to_string_lossy().into_owned()),
                        (
                            "sessionDir".into(),
                            fixture.0.to_string_lossy().into_owned(),
                        ),
                    ]),
                },
                execution: crate::RuntimeExecution {
                    home: crate::RuntimeHome::Managed { remote_root: None },
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let edit = start_edit(&service, &fixture, "replacement".into());
    fixture.wait("primary-prompt-read");
    let end = Instant::now() + Duration::from_secs(3);
    let question = loop {
        let view = snapshot(&service, fixture.id("edit"));
        if let Some(interaction) = view.pending_interactions.first() {
            break interaction.clone();
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    };
    let velune_conversation::InteractionKind::UserInput { questions } = &question.kind else {
        panic!("native form question")
    };
    let answer = questions.first().unwrap();
    let yes = answer
        .options
        .iter()
        .find(|option| option.id == "yes")
        .unwrap();
    later(
        &service,
        "edit",
        Command::ReplyRuntimeInteraction {
            runtime_id: "pi".into(),
            conversation_id: fixture.id("edit"),
            interaction_id: question.id.clone(),
            reply: RuntimeInteractionReply::Answers {
                answers: vec![velune_conversation::InteractionAnswer {
                    question_id: answer.id.clone(),
                    values: vec![yes.id.clone()],
                }],
            },
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .unwrap();
    fixture.wait("primary-answered");
    assert!(
        matches!(edit.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "answer consumed the original edit wait"
    );
    let replies = fixture.requests("edit");
    assert_eq!(
        replies
            .iter()
            .filter(|r| r["type"] == "extension_ui_response")
            .count(),
        1
    );
    let cancel = later(
        &service,
        "edit",
        Command::Cancel {
            runtime_id: "pi".into(),
            conversation_id: fixture.id("edit"),
        },
    );
    fixture.wait("primary-clear-read");
    // The helper continues reading while withholding ClearQueue's ACK, so an
    // early Abort would actually be visible here, not hidden by a blocked stdin.
    snapshot(&service, fixture.id("edit"));
    assert!(
        fixture
            .requests("edit")
            .iter()
            .all(|r| r["type"] != "abort")
    );
    assert!(matches!(cancel.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(matches!(edit.try_recv(), Err(mpsc::TryRecvError::Empty)));
    fs::write(fixture.0.join("release-primary-clear"), []).unwrap();
    fixture.wait("primary-abort-read");
    assert!(
        matches!(cancel.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "ClearQueue ACK fabricated Abort acceptance"
    );
    assert!(matches!(edit.try_recv(), Err(mpsc::TryRecvError::Empty)));
    fs::write(fixture.0.join("release-primary-abort"), []).unwrap();
    assert!(matches!(
        cancel
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        Response::Cancel(_)
    ));
    assert!(
        matches!(edit.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "Cancel discarded the pending prompt fact channel"
    );
    assert_ne!(
        snapshot(&service, fixture.id("edit")).run_state,
        RunState::Running
    );
    fs::write(fixture.0.join("release-primary-fact"), []).unwrap();
    let Response::EditMessage(result) = edit.recv_timeout(Duration::from_secs(3)).unwrap().unwrap()
    else {
        panic!("late exact prompt fact")
    };
    assert_eq!(
        result.submission_receipt,
        Some(crate::SubmissionReceipt::Sending)
    );
    assert_ne!(
        result.snapshot.unwrap().run_state,
        RunState::Running,
        "late preflight ACK restarted a cancelled native turn"
    );
    let wire = fixture.requests("edit");
    assert_eq!(
        wire.iter()
            .filter(|r| r["message"] == "replacement")
            .count(),
        1
    );
    assert_eq!(
        wire.iter().filter(|r| r["type"] == "clear_queue").count(),
        1
    );
    assert_eq!(wire.iter().filter(|r| r["type"] == "abort").count(), 1);
    assert_eq!(
        wire.iter()
            .filter(|r| r["type"] == "extension_ui_response")
            .count(),
        1
    );
    close_after_cleanup(&service);
}
