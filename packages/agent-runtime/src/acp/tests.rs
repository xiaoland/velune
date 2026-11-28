use super::*;

#[test]
fn permission_projection_uses_effect_declared_by_agent() {
    let projected = |id: &'static str, kind| {
        let option = project_permission_option(wire::PermissionOption::new(id, id, kind));
        assert_eq!(option.id, id);
        option.approval_kind
    };
    assert_eq!(
        projected("allow", wire::PermissionOptionKind::AllowAlways),
        Some(ApprovalOptionKind::AllowAlways)
    );
    assert_eq!(
        projected("not-an-allow-id", wire::PermissionOptionKind::AllowOnce),
        Some(ApprovalOptionKind::AllowOnce)
    );
}

#[test]
fn rpc_error_keeps_code_message_and_native_data() {
    let error = rpc_error(&json!({"code":-32000,"message":"synthetic unavailable",
            "data":{"upstreamCode":"server_overloaded","requestId":"synthetic"}}))
    .unwrap();
    let text = error.to_string();
    for value in [
        "-32000",
        "synthetic unavailable",
        "server_overloaded",
        "requestId",
    ] {
        assert!(text.contains(value));
    }
    assert!(rpc_error(&json!({"code":"invalid","message":"bad"})).is_err());
}

pub(super) fn synthetic_client(mode: &str) -> (Client, PathBuf) {
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root =
        std::env::temp_dir().join(format!("velune-acp-form-{:x}", u64::from_le_bytes(random)));
    std::fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let helper = root.join("rpc.py");
    std::fs::write(&helper,r#"
import json,sys,os,time
mode=sys.argv[1]
root=os.path.dirname(__file__)
def marker(name): return os.path.exists(os.path.join(root,name))
def wait(name):
    while not marker(name): time.sleep(0.005)
pending = None
count = 0
seen = set()
live_model = 'model/a'
auto_values = {'model/a':False,'model/b':'default'}
def emit(value): print(json.dumps(dict(jsonrpc='2.0',**value)),flush=True)
def chunk(text,session='synthetic'):
    emit({'method':'session/update','params':{'sessionId':session,'update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}}}})
def form():
    emit({'id':'q','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','required':['value'],'properties':{'value':{'type':'string'}}}}})
def interactions():
    emit({'id':'p','method':'session/request_permission','params':{'sessionId':'synthetic','toolCall':{'toolCallId':'t','title':'Synthetic tool'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}})
    emit({'id':'q','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','required':['value'],'properties':{'value':{'type':'string'}}}}})
def options(value='model/a'):
    result=[{'id':'opaque/model-id','name':'Model','category':'model','type':'select','currentValue':value,'options':[{'value':'model/a','name':'A'},{'value':'model/b','name':'B'}]}]
    if mode=='model-auto-compaction':
        if value=='model/a':
            result.append({'id':'auto_compact_enabled','name':'Automatically compact','type':'boolean','currentValue':auto_values[value]})
        else:
            result.append({'id':'vendor.retention','name':'Retention','type':'select','currentValue':auto_values[value], 'options':[{'value':'default','name':'Default'},{'value':'less','name':'Less'}]})
        result.append({'id':'opaque/safety','name':'Permissions','category':'_permissions','type':'boolean','currentValue':False})
    return result

for line in sys.stdin:
    request=json.loads(line)
    method=request.get('method')
    params=request.get('params',{})
    if method=='initialize':
        assert params['clientCapabilities']['elicitation']['form']=={}
        result={'protocolVersion':1,'agentCapabilities':{'loadSession':True,'sessionCapabilities':{'list':{},'resume':{},'close':{},'delete':{}}},'authMethods':[]}
    elif method=='session/new':
        if mode=='history':
            chunk('first')
            chunk('ignored','unknown')
            chunk(' second')
        result={'sessionId':'synthetic','configOptions':options()}
        if mode=='idle-form':
            emit({'id':request['id'],'result':result})
            form()
            wait('release')
            continue
        if mode=='blocked':
            emit({'id':request['id'],'result':result})
            wait('updates')
            chunk('progress while stdin is blocked')
            interactions()
            wait('terminal')
            with open(os.path.join(root,'prompt-id')) as f: prompt_id=json.load(f)
            emit({'id':prompt_id,'result':{'stopReason':'end_turn'}})
            wait('release')
            continue
    elif method in ('session/load','session/resume') and mode=='reject-open':
        chunk('unconfirmed history',params['sessionId'])
        emit({'id':request['id'],'error':{'code':-32000,'message':'not opened','data':{'target':params['sessionId']}}})
        continue
    elif method=='session/load':
        chunk('authoritative replay',params['sessionId'])
        live_model='model/b'
        result={'configOptions':options(live_model)}
    elif method=='session/resume': result={'configOptions':options()}
    elif method=='session/list':
        result={'sessions':[]}
        if mode=='live':
            emit({'id':request['id'],'result':result})
            chunk('|')
            continue
    elif method=='session/set_config_option' and mode=='model-auto-compaction':
        assert params['sessionId']=='synthetic'
        config,value=params['configId'],params['value']
        if config=='opaque/model-id':
            assert auto_values[live_model] == (False if live_model=='model/a' else 'default')
            live_model=value
        elif live_model=='model/a':
            assert config=='auto_compact_enabled' and params['type']=='boolean' and isinstance(value,bool)
            auto_values[live_model]=value
        else:
            assert config=='vendor.retention' and value in ('default','less')
            auto_values[live_model]=value
        with open(os.path.join(root,'setting-writes'),'a') as f: f.write(config+'\n')
        result={'configOptions':options(live_model)}
    elif method=='session/set_config_option':
        assert params['configId']=='opaque/model-id'
        if mode=='control-budget':
            form()
            wait('release')
        if mode=='late-control':
            wait('release')
            emit({'id':request['id'],'result':{'configOptions':options('model/b')}})
            emit({'id':'after-deadline','method':'synthetic/unsupported','params':{}})
            with open(os.path.join(root,'terminal-emitted'),'w') as f: f.write('control')
            continue
        if mode=='reject':
            emit({'id':request['id'],'error':{'code':-32000,'message':'synthetic rejected','data':{'native':'kept'}}})
            continue
        result={'configOptions':options('model/b')}
        if mode=='accepted-disconnect':
            emit({'id':request['id'],'result':result})
            emit({'id':'unsupported','method':'synthetic/unsupported','params':{}})
            sys.exit(0)

    elif method=='session/prompt':
        if mode=='model-auto-compaction':
            assert auto_values[live_model] == json.loads(params['prompt'][0]['text'])
            emit({'id':request['id'],'result':{'stopReason':'end_turn'}})
            continue
        if mode=='blocked': continue
        if mode=='prompt-error':
            chunk('output before error')
            emit({'id':request['id'],'error':{'code':-32000,'message':'after execution','data':{'toolRan':True}}})
            continue
        if mode=='disconnect':
            chunk('output before disconnect')
            sys.exit(0)
        if mode=='auto-short-write':
            form()
            wait('automatic')
            emit({'id':'wrong-session','method':'session/request_permission','params':{'sessionId':'other-session','toolCall':{'toolCallId':'t'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}})
            chunk('automatic cancellation required')
            wait('release')
            continue
        if mode in ('late-complete','late-error'):
            wait('release')
            if mode=='late-error': emit({'id':request['id'],'error':{'code':-32000,'message':'late after deadline','data':{'executed':True}}})
            else: emit({'id':request['id'],'result':{'stopReason':'end_turn'}})
            # Also request an automatic reply in this batch. The latched timeout
            # must forbid it without losing the actual terminal result above.
            emit({'id':'after-deadline','method':'synthetic/unsupported','params':{}})
            with open(os.path.join(root,'terminal-emitted'),'w') as f: f.write(mode)
            continue
        if mode=='hold':
            wait('release')
            emit({'id':request['id'],'result':{'stopReason':'end_turn'}})
            continue
        pending=request['id']
        if mode.startswith('budget-'):
            wait('after-write')
            chunk('late output')
            interactions()
            continue
        if mode in ('duplicate-permission-first','duplicate-elicitation-first'):
            permission={'id':'same','method':'session/request_permission','params':{'sessionId':'synthetic','toolCall':{'toolCallId':'t'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}}
            form={'id':'same','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','properties':{'value':{'type':'string'}}}}}
            for value in ([permission,form] if mode=='duplicate-permission-first' else [form,permission]): emit(value)
            continue
        if mode=='live':
            interactions()
            continue
        count+=1
        form={'mode':'form','sessionId':'synthetic','message':'How many?',
              'requestedSchema':{'type':'object','required':['count'],'properties':{'count':{'type':'integer','minimum':1}}}}
        emit({'id':'q','method':'elicitation/create','params':form})
        if count==2: emit({'method':'$/cancel_request','params':{'requestId':'q'}})
        continue
    elif method=='session/cancel':
        if mode=='live':
            seen.add('cancel')
            chunk('cancel delivered')
            assert seen=={'p','q','cancel'}
            emit({'id':pending,'result':{'stopReason':'cancelled'}})
        continue
    elif method is None and mode.startswith('late-'):
        with open(os.path.join(root,'unexpected-reply'),'w') as f: f.write('new dispatch after expiry')
        continue
    elif method is None and mode=='idle-form':
        assert request['id']=='q' and request['result']['content']['value']
        count+=1
        with open(os.path.join(root,'reply-count'),'w') as f: f.write(str(count))
        chunk('idle answer delivered')
        continue
    elif method is None and mode.startswith('budget-'):
        assert request['id'] not in seen
        seen.add(request['id'])
        chunk(request['id']+' delivered')
        if seen=={'p','q'}:
            wait('release')
            if mode=='budget-error': emit({'id':pending,'error':{'code':-32000,'message':'late turn error','data':{'executed':True}}})
            else: emit({'id':pending,'result':{'stopReason':'end_turn'}})
        continue
    elif method is None and mode.startswith('duplicate-'):
        with open(os.path.join(root,'duplicate-reply'),'w') as f: f.write('unexpected response')
        continue
    elif method is None and mode=='live':
        seen.add(request['id'])
        if request['id']=='p': assert request['result']['outcome']['optionId']=='yes'
        else: assert request['result']['content']['value']=='answer'
        chunk(request['id']+' delivered')
        continue
    elif method is None and mode=='blocked': continue
    elif method is None and request['id']=='q':
        if count==1: assert request['result']['content']['count']==2
        else: assert request['result']['action']=='cancel'
        emit({'id':pending,'result':{'stopReason':'end_turn'}})
        continue
    else: result={}
    emit({'id':request['id'],'result':result})
"#).unwrap();
    let client = Client::connect(Config {
        binary: "/usr/bin/python3".into(),
        args: vec![helper.to_string_lossy().into_owned(), mode.into()],
        environment: Default::default(),
        execution: ExecutionTarget::Local,
        os_home: Some(root.clone()),
        mcp_servers: vec![],
    })
    .unwrap();
    (client, root)
}

#[test]
fn elicitation_form_negotiates_and_routes_typed_reply_and_cancellation() {
    let (mut client, root) = synthetic_client("form");
    client.create(&root, "synthetic-acp").unwrap();
    for turn in 0..3 {
        client.send("question", &[]).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while client.snapshot().unwrap().pending_interactions.is_empty()
            && client.turn_running()
            && Instant::now() < deadline
        {
            client.poll().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if turn != 1 {
            let interaction = client.snapshot().unwrap().pending_interactions[0].clone();
            assert!(matches!(
                interaction.kind,
                InteractionKind::UserInput { .. }
            ));
            if turn == 0 {
                client
                    .reply(
                        &interaction.id,
                        RuntimeInteractionReply::Answers {
                            answers: vec![InteractionAnswer {
                                question_id: "count".into(),
                                values: vec!["2".into()],
                            }],
                        },
                    )
                    .unwrap();
            } else {
                client.cancel().unwrap();
            }
        }
        while client.turn_running() && Instant::now() < deadline {
            client.poll().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!client.turn_running());
        assert!(client.snapshot().unwrap().pending_interactions.is_empty());
    }
    // A request whose RPC connection disappears cannot survive as an
    // answerable form or be converted into an ordinary conversation input.
    client.send("question", &[]).unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while client.snapshot().unwrap().pending_interactions.is_empty() && Instant::now() < deadline {
        client.poll().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let expired = client.snapshot().unwrap().pending_interactions[0]
        .id
        .clone();
    client.fail();
    assert!(client.snapshot().unwrap().pending_interactions.is_empty());
    assert!(
        client
            .reply(&expired, RuntimeInteractionReply::Cancel)
            .is_err()
    );
    client.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
fn command_deadline() -> Instant {
    Instant::now() + std::time::Duration::from_secs(4)
}

fn drive_port(
    client: &mut Client,
    done: impl Fn(&Client, &[crate::creation::Retained<AcpReceipt>]) -> bool,
) -> Vec<crate::creation::Retained<AcpReceipt>> {
    let deadline = command_deadline();
    let mut receipts = Vec::new();
    loop {
        client.poll().unwrap();
        receipts.extend(client.take_receipts());
        if done(client, &receipts) {
            return receipts;
        }
        assert!(
            Instant::now() < deadline,
            "ACP synthetic port did not progress: {receipts:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn acp_port_blocked_stdin_keeps_poll_and_replies_live_until_written() {
    let (mut client, root) = synthetic_client("blocked");
    client.create(&root, "synthetic-acp").unwrap();
    client.enable_command_port().unwrap();
    let before = Instant::now();
    let prompt = client
        .enqueue(
            AcpCommand::Prompt {
                text: "x".repeat(200_000),
                images: vec![],
                run_deadline: None,
            },
            command_deadline(),
        )
        .unwrap();
    assert!(before.elapsed() < std::time::Duration::from_millis(500));
    std::fs::write(
        root.join("prompt-id"),
        serde_json::to_vec(&prompt.wire_id).unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("updates"), []).unwrap();
    let receipts = drive_port(&mut client, |client, _| {
        client.snapshot().unwrap().pending_interactions.len() == 2
    });
    assert!(
        !receipts
            .iter()
            .any(|receipt| receipt.ticket == prompt && receipt.stage == AcpReceiptStage::Written)
    );
    assert!(client.snapshot().unwrap().messages.iter().any(|message| message.blocks.iter().any(|block|
            matches!(block, MessageBlock::Text { text } if text == "progress while stdin is blocked"))));
    let interactions = client.snapshot().unwrap().pending_interactions.clone();
    let permission = interactions
        .iter()
        .find(|i| matches!(i.kind, InteractionKind::Approval { .. }))
        .unwrap()
        .id
        .clone();
    let elicitation = interactions
        .iter()
        .find(|i| matches!(i.kind, InteractionKind::UserInput { .. }))
        .unwrap()
        .id
        .clone();
    let answer = client
        .enqueue(
            AcpCommand::ReplyElicitation {
                interaction_id: elicitation.clone(),
                reply: RuntimeInteractionReply::Answers {
                    answers: vec![InteractionAnswer {
                        question_id: "value".into(),
                        values: vec!["answer".into()],
                    }],
                },
            },
            command_deadline(),
        )
        .unwrap();
    // The large prompt still blocks the actual pipe, so neither reply can
    // become Written while these duplicate-admission checks run.
    let duplicate = client.enqueue(
        AcpCommand::ReplyElicitation {
            interaction_id: elicitation,
            reply: RuntimeInteractionReply::Cancel,
        },
        command_deadline(),
    );
    assert!(
        matches!(&duplicate, Err(AcpAdmissionError::NotDispatched { code, .. })
        if code == "already_pending"),
        "{duplicate:?}"
    );
    let permission_reply = client
        .enqueue(
            AcpCommand::ReplyPermission {
                interaction_id: permission.clone(),
                reply: RuntimeInteractionReply::Decision {
                    option_id: "yes".into(),
                },
            },
            command_deadline(),
        )
        .unwrap();
    let duplicate = client.enqueue(
        AcpCommand::ReplyPermission {
            interaction_id: permission.clone(),
            reply: RuntimeInteractionReply::Cancel,
        },
        command_deadline(),
    );
    assert!(
        matches!(&duplicate, Err(AcpAdmissionError::NotDispatched { code, .. })
        if code == "already_pending"),
        "{duplicate:?}"
    );
    let cancel = client
        .enqueue(AcpCommand::Cancel, command_deadline())
        .unwrap();
    client.poll().unwrap();
    assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 2);
    std::fs::write(root.join("terminal"), []).unwrap();
    let terminal = drive_port(&mut client, |client, receipts| {
        !client.turn_running()
            && receipts.iter().any(|receipt| {
                receipt.ticket == prompt
                    && matches!(receipt.stage, AcpReceiptStage::TurnCompleted { .. })
            })
    });
    assert!(
        !terminal
            .iter()
            .any(|receipt| receipt.stage == AcpReceiptStage::Written)
    );
    // Terminal parent is not permission delivery. Both interactions and their
    // write leases stay tracked until the blocked stream is actually drained.
    assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 2);
    assert!(matches!(
        client.enqueue(AcpCommand::Close, command_deadline()),
        Err(AcpAdmissionError::NeedsPreparation(_))
    ));
    assert!(
        client
            .enqueue(
                AcpCommand::ReplyPermission {
                    interaction_id: permission,
                    reply: RuntimeInteractionReply::Cancel
                },
                command_deadline()
            )
            .is_err()
    );
    std::fs::write(root.join("release"), []).unwrap();
    let settled = drive_port(&mut client, |client, receipts| {
        client.snapshot().unwrap().pending_interactions.is_empty()
            && receipts
                .iter()
                .any(|r| r.ticket == cancel && r.stage == AcpReceiptStage::Written)
            && receipts
                .iter()
                .any(|r| r.ticket == answer && r.stage == AcpReceiptStage::Written)
            && receipts
                .iter()
                .any(|r| r.ticket == permission_reply && r.stage == AcpReceiptStage::Written)
    });
    assert!(
        settled
            .iter()
            .any(|r| r.ticket == prompt && r.stage == AcpReceiptStage::Written)
    );
    assert!(!client.needs_cleanup());
    client.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acp_port_native_rejection_is_not_prompt_turn_error_or_disconnect() {
    for mode in ["reject", "prompt-error", "disconnect"] {
        let (mut client, root) = synthetic_client(mode);
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        assert!(client.send("sync must not mutate", &[]).is_err());
        assert!(client.cancel().is_err());
        let command = if mode == "reject" {
            AcpCommand::SetConfig {
                config_id: "opaque/model-id".into(),
                value: ConfigValue::Select("model/b".into()),
            }
        } else {
            AcpCommand::Prompt {
                text: "synthetic".into(),
                images: vec![],
                run_deadline: None,
            }
        };
        let ticket = client.enqueue(command, command_deadline()).unwrap();
        let deadline = command_deadline();
        let mut receipts = Vec::new();
        let mut failure = None;
        while Instant::now() < deadline {
            let outcome = client.poll();
            receipts.extend(client.take_receipts());
            if let Err(error) = outcome {
                failure = Some(error);
                break;
            }
            if receipts
                .iter()
                .any(|r| r.ticket == ticket && !matches!(r.stage, AcpReceiptStage::Written))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            receipts
                .iter()
                .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written),
            "{mode}: {receipts:?}"
        );
        match mode {
            "reject" => {
                assert!(receipts.iter().any(|r| matches!(&r.stage, AcpReceiptStage::ProtocolRejected { error }
                        if error.code == -32000 && error.message == "synthetic rejected" && error.data == Some(json!({"native":"kept"})))));
                assert_eq!(
                    client.config_options()[0].value,
                    ConfigValue::Select("model/a".into())
                );
                assert!(!client.needs_cleanup());
            }
            "prompt-error" => {
                assert!(
                    receipts
                        .iter()
                        .any(|r| matches!(&r.stage, AcpReceiptStage::TurnError { error }
                        if error.data == Some(json!({"toolRan":true}))))
                );
                assert!(
                    !receipts
                        .iter()
                        .any(|r| matches!(r.stage, AcpReceiptStage::ProtocolRejected { .. }))
                );
            }
            _ => {
                assert!(failure.is_some());
                assert!(
                    receipts
                        .iter()
                        .any(|r| matches!(r.stage, AcpReceiptStage::Indeterminate { .. }))
                );
                assert!(client.needs_cleanup());
                client.poll().unwrap();
                assert!(client.needs_cleanup());
                assert!(matches!(
                    client.enqueue(AcpCommand::Cancel, command_deadline()),
                    Err(AcpAdmissionError::NeedsPreparation(_))
                ));
            }
        }
        client.shutdown().unwrap();
        assert!(!client.needs_cleanup());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn acp_port_new_load_replay_and_opaque_config_are_authoritative() {
    let (mut client, root) = synthetic_client("history");
    client.enable_command_port().unwrap();
    let ticket = client
        .enqueue(
            AcpCommand::New {
                cwd: root.clone(),
                runtime_id: "synthetic-acp".into(),
            },
            command_deadline(),
        )
        .unwrap();
    drive_port(&mut client, |_, receipts| {
        receipts.iter().any(|r| {
            r.ticket == ticket && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
        })
    });
    let messages = &client.snapshot().unwrap().messages;
    assert_eq!(messages.len(), 1);
    assert!(
        matches!(&messages[0].blocks[0], MessageBlock::Text { text } if text == "first second")
    );
    assert_eq!(client.models().unwrap().len(), 2);
    let config = client
        .enqueue(
            AcpCommand::SetConfig {
                config_id: "opaque/model-id".into(),
                value: ConfigValue::Select("model/a".into()),
            },
            command_deadline(),
        )
        .unwrap();
    drive_port(&mut client, |_, receipts| {
        receipts.iter().any(|r| {
            r.ticket == config && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
        })
    });
    // Returned options, not the submitted value, are authoritative; no restart/session probe.
    assert_eq!(
        client.config_options()[0].value,
        ConfigValue::Select("model/b".into())
    );
    assert_eq!(
        client.snapshot().unwrap().conversation.id,
        "synthetic-acp:synthetic"
    );
    let revision = client.snapshot().unwrap().revision;
    assert!(matches!(
        client.enqueue(
            AcpCommand::Load {
                native_id: "loaded".into(),
                cwd: root.clone(),
                runtime_id: "synthetic-acp".into()
            },
            command_deadline()
        ),
        Err(AcpAdmissionError::NeedsPreparation(_))
    ));
    assert_eq!(client.snapshot().unwrap().revision, revision);
    for resume in [false, true] {
        let close = client
            .enqueue(AcpCommand::Close, command_deadline())
            .unwrap();
        drive_port(&mut client, |_, receipts| {
            receipts.iter().any(|r| {
                r.ticket == close && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
            })
        });
        let command = if resume {
            AcpCommand::Resume {
                native_id: "loaded".into(),
                cwd: root.clone(),
                runtime_id: "synthetic-acp".into(),
            }
        } else {
            AcpCommand::Load {
                native_id: "loaded".into(),
                cwd: root.clone(),
                runtime_id: "synthetic-acp".into(),
            }
        };
        let load = client.enqueue(command, command_deadline()).unwrap();
        drive_port(&mut client, |_, receipts| {
            receipts.iter().any(|r| {
                r.ticket == load && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
            })
        });
        let messages = &client.snapshot().unwrap().messages;
        if resume {
            assert!(!messages.iter().any(|message| {
                message
                    .blocks
                    .iter()
                    .any(|block| matches!(block, MessageBlock::Text { .. }))
            }));
        } else {
            assert_eq!(messages.len(), 1);
            assert!(messages[0].completed);
            assert!(
                matches!(&messages[0].blocks[0], MessageBlock::Text { text } if text == "authoritative replay")
            );
        }
    }
    client.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acp_port_reused_wire_ids_stay_on_their_connection_generation() {
    let (mut old, old_root) = synthetic_client("hold");
    let (mut new, new_root) = synthetic_client("hold");
    old.create(&old_root, "synthetic-acp").unwrap();
    new.create(&new_root, "synthetic-acp").unwrap();
    old.enable_command_port().unwrap();
    new.enable_command_port().unwrap();
    let old_ticket = old
        .enqueue(
            AcpCommand::Prompt {
                text: "old".into(),
                images: vec![],
                run_deadline: None,
            },
            command_deadline(),
        )
        .unwrap();
    let new_ticket = new
        .enqueue(
            AcpCommand::Prompt {
                text: "new".into(),
                images: vec![],
                run_deadline: None,
            },
            command_deadline(),
        )
        .unwrap();
    assert_eq!(old_ticket.wire_id, new_ticket.wire_id);
    assert_ne!(old_ticket.generation, new_ticket.generation);
    assert_ne!(old_ticket.session, new_ticket.session);
    // Consuming Written cannot remove either still-live prompt operation.
    drive_port(&mut old, |_, receipts| {
        receipts.iter().any(|r| r.stage == AcpReceiptStage::Written)
    });
    drive_port(&mut new, |_, receipts| {
        receipts.iter().any(|r| r.stage == AcpReceiptStage::Written)
    });
    std::fs::write(old_root.join("release"), []).unwrap();
    drive_port(&mut old, |_, receipts| {
        receipts.iter().any(|r| {
            r.ticket == old_ticket && matches!(r.stage, AcpReceiptStage::TurnCompleted { .. })
        })
    });
    new.poll().unwrap();
    assert!(new.turn_running());
    assert!(new.take_receipts().is_empty());
    std::fs::write(new_root.join("release"), []).unwrap();
    drive_port(&mut new, |_, receipts| {
        receipts.iter().any(|r| {
            r.ticket == new_ticket && matches!(r.stage, AcpReceiptStage::TurnCompleted { .. })
        })
    });
    old.shutdown().unwrap();
    new.shutdown().unwrap();
    std::fs::remove_dir_all(old_root).unwrap();
    std::fs::remove_dir_all(new_root).unwrap();
}
#[test]
fn acp_port_permission_elicitation_and_cancel_are_delivered_before_prompt_result() {
    let (mut client, root) = synthetic_client("live");
    client.create(&root, "synthetic-acp").unwrap();
    client.enable_command_port().unwrap();
    // Leave 23 completed control results unconsumed. Their receipt budget
    // must remain reserved, with separate space for replies to a live turn.
    for pages in 1..=23 {
        client
            .enqueue(
                AcpCommand::List {
                    cwd: None,
                    cursor: None,
                },
                command_deadline(),
            )
            .unwrap();
        let deadline = command_deadline();
        loop {
            client.poll().unwrap();
            if client.snapshot().unwrap().messages.iter().any(|m| {
                m.blocks
                    .iter()
                    .any(|b| matches!(b, MessageBlock::Text { text } if text.len() == pages))
            }) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    let prompt = client
        .enqueue(
            AcpCommand::Prompt {
                text: "question".into(),
                images: vec![],
                run_deadline: None,
            },
            command_deadline(),
        )
        .unwrap();
    let deadline = command_deadline();
    while client.snapshot().unwrap().pending_interactions.len() != 2 {
        client.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        matches!(client.enqueue(AcpCommand::List { cwd: None, cursor: None }, command_deadline()),
            Err(AcpAdmissionError::NotDispatched { code, .. }) if code == "queue_full")
    );
    let interactions = client.snapshot().unwrap().pending_interactions.clone();
    let mut replies = Vec::new();
    for interaction in interactions {
        let command = match interaction.kind {
            InteractionKind::Approval { .. } => AcpCommand::ReplyPermission {
                interaction_id: interaction.id,
                reply: RuntimeInteractionReply::Decision {
                    option_id: "yes".into(),
                },
            },
            _ => AcpCommand::ReplyElicitation {
                interaction_id: interaction.id,
                reply: RuntimeInteractionReply::Answers {
                    answers: vec![InteractionAnswer {
                        question_id: "value".into(),
                        values: vec!["answer".into()],
                    }],
                },
            },
        };
        replies.push(client.enqueue(command, command_deadline()).unwrap());
    }
    let written = drive_port(&mut client, |client, receipts| {
        client.snapshot().unwrap().pending_interactions.is_empty()
            && client.snapshot().unwrap().messages.iter().any(|m| {
                m.blocks.iter().any(
                    |b| matches!(b, MessageBlock::Text { text } if text.contains("q delivered")),
                )
            })
            && replies.iter().all(|ticket| {
                receipts
                    .iter()
                    .any(|r| &r.ticket == ticket && r.stage == AcpReceiptStage::Written)
            })
    });
    assert!(client.turn_running());
    assert!(
        !written
            .iter()
            .any(|r| (r.ticket == prompt || replies.contains(&r.ticket))
                && matches!(
                    r.stage,
                    AcpReceiptStage::ProtocolSuccess { .. } | AcpReceiptStage::TurnCompleted { .. }
                ))
    );
    assert!(
        client.has_pending_operations(),
        "consuming Written does not free the prompt lifecycle"
    );
    let cancel = client
        .enqueue(AcpCommand::Cancel, command_deadline())
        .unwrap();
    let terminal = drive_port(&mut client, |client, receipts| {
        !client.turn_running()
            && receipts.iter().any(|r| r.ticket == prompt && matches!(&r.stage, AcpReceiptStage::TurnCompleted { stop_reason } if stop_reason == "cancelled"))
            && receipts.iter().any(|r| r.ticket == cancel && r.stage == AcpReceiptStage::Written)
    });
    assert!(
        !terminal
            .iter()
            .any(|r| r.ticket == cancel
                && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. }))
    );
    assert!(!client.has_pending_operations());
    client.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acp_port_recorded_success_survives_terminal_barrier_and_optional_reply_failure() {
    let (mut client, root) = synthetic_client("accepted-disconnect");
    client.create(&root, "synthetic-acp").unwrap();
    client.enable_command_port().unwrap();
    let ticket = client
        .enqueue(
            AcpCommand::SetConfig {
                config_id: "opaque/model-id".into(),
                value: ConfigValue::Select("model/a".into()),
            },
            command_deadline(),
        )
        .unwrap();
    let deadline = command_deadline();
    let mut receipts = Vec::new();
    let mut failed = false;
    while Instant::now() < deadline {
        let result = client.poll();
        receipts.extend(client.take_receipts());
        if result.is_err() {
            failed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(failed);
    assert!(client.needs_cleanup());
    assert_eq!(
        client.config_options()[0].value,
        ConfigValue::Select("model/b".into())
    );
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket == ticket
                && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. }))
    );
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
    );
    assert!(!receipts.iter().any(
            |r| r.ticket == ticket && matches!(r.stage, AcpReceiptStage::Indeterminate { .. })
        ));
    client.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acp_port_absolute_deadline_does_not_release_blocked_writer_or_interactions() {
    let (mut client, root) = synthetic_client("blocked");
    client.create(&root, "synthetic-acp").unwrap();
    client.enable_command_port().unwrap();
    let deadline = Instant::now() + std::time::Duration::from_millis(200);
    let prompt = client
        .enqueue(
            AcpCommand::Prompt {
                text: "x".repeat(200_000),
                images: vec![],
                run_deadline: Some(deadline),
            },
            deadline,
        )
        .unwrap();
    std::fs::write(root.join("updates"), []).unwrap();
    drive_port(&mut client, |client, _| {
        client.snapshot().unwrap().pending_interactions.len() == 2
    });
    let interaction_id = client.snapshot().unwrap().pending_interactions[0]
        .id
        .clone();
    std::thread::sleep(
        deadline.saturating_duration_since(Instant::now()) + std::time::Duration::from_millis(10),
    );
    assert!(
        matches!(client.enqueue(AcpCommand::ReplyPermission { interaction_id, reply: RuntimeInteractionReply::Cancel }, command_deadline()),
            Err(AcpAdmissionError::NotDispatched { code, .. }) if code == "timeout")
    );
    assert!(client.poll().is_err());
    assert!(client.needs_cleanup());
    assert!(client.has_pending_operations());
    assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 2);
    let receipts = client.take_receipts();
    assert!(receipts.iter().any(
            |r| r.ticket == prompt && matches!(r.stage, AcpReceiptStage::Indeterminate { .. })
        ));
    assert!(!receipts.iter().any(|r| r.stage == AcpReceiptStage::Written));
    assert!(
        client.has_pending_operations(),
        "timeout is not cancellation of a real blocked write"
    );
    client.shutdown().unwrap();
    assert!(!client.has_pending_operations());
    std::fs::remove_dir_all(root).unwrap();
}
fn rejected_open_rolls_back(resume: bool) {
    let (mut client, root) = synthetic_client("reject-open");
    client.enable_command_port().unwrap();
    let command = if resume {
        AcpCommand::Resume {
            native_id: "rejected".into(),
            cwd: root.clone(),
            runtime_id: "synthetic-acp".into(),
        }
    } else {
        AcpCommand::Load {
            native_id: "rejected".into(),
            cwd: root.clone(),
            runtime_id: "synthetic-acp".into(),
        }
    };
    let ticket = client.enqueue(command, command_deadline()).unwrap();
    drive_port(&mut client, |_, receipts| {
        receipts.iter().any(|r| {
            r.ticket == ticket && matches!(r.stage, AcpReceiptStage::ProtocolRejected { .. })
        })
    });
    assert!(
        client.snapshot().is_none(),
        "rejected identity/history must not remain published"
    );
    assert!(
        client
            .enqueue(
                AcpCommand::Prompt {
                    text: "must not reach rejected ID".into(),
                    images: vec![],
                    run_deadline: None
                },
                command_deadline()
            )
            .is_err()
    );
    assert!(!client.needs_cleanup());
    let retry = client
        .enqueue(
            AcpCommand::New {
                cwd: root.clone(),
                runtime_id: "synthetic-acp".into(),
            },
            command_deadline(),
        )
        .unwrap();
    drive_port(&mut client, |_, receipts| {
        receipts.iter().any(|r| {
            r.ticket == retry && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
        })
    });
    assert_eq!(
        client.snapshot().unwrap().conversation.id,
        "synthetic-acp:synthetic"
    );
    client.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acp_port_rejected_load_revokes_unconfirmed_identity_and_history() {
    rejected_open_rolls_back(false);
}

#[test]
fn acp_port_rejected_resume_revokes_unconfirmed_identity_and_history() {
    rejected_open_rolls_back(true);
}

#[test]
fn acp_port_agent_request_ids_are_unique_across_interaction_kinds_in_both_orders() {
    for mode in ["duplicate-permission-first", "duplicate-elicitation-first"] {
        let (mut client, root) = synthetic_client(mode);
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        client
            .enqueue(
                AcpCommand::Prompt {
                    text: "question".into(),
                    images: vec![],
                    run_deadline: None,
                },
                command_deadline(),
            )
            .unwrap();
        let deadline = command_deadline();
        let error = loop {
            if let Err(error) = client.poll() {
                break error;
            }
            assert!(
                Instant::now() < deadline,
                "second duplicate interaction must fail"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(error.to_string().contains("身份重复"));
        assert!(client.needs_cleanup());
        assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!root.join("duplicate-reply").exists());
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn acp_idle_human_form_has_no_implicit_ttl_in_port_or_sync_facade() {
    let (mut port, port_root) = synthetic_client("idle-form");
    let (mut sync, sync_root) = synthetic_client("idle-form");
    port.create(&port_root, "synthetic-acp").unwrap();
    port.enable_command_port().unwrap();
    sync.create(&sync_root, "synthetic-acp").unwrap();
    drive_port(&mut port, |client, _| {
        !client.snapshot().unwrap().pending_interactions.is_empty()
    });
    let deadline = command_deadline();
    while sync.snapshot().unwrap().pending_interactions.is_empty() {
        sync.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        port.elicitations.values().next().unwrap().parent_deadline,
        None
    );
    assert_eq!(
        sync.elicitations.values().next().unwrap().parent_deadline,
        None
    );
    // Real elapsed time is intentional: this is the original 15s TTL regression.
    std::thread::sleep(CONTROL_TIMEOUT + std::time::Duration::from_millis(100));
    port.poll().unwrap();
    sync.poll().unwrap();
    assert_eq!(port.snapshot().unwrap().pending_interactions.len(), 1);
    assert_eq!(sync.snapshot().unwrap().pending_interactions.len(), 1);
    assert!(!port.needs_cleanup());
    let id = port.snapshot().unwrap().pending_interactions[0].id.clone();
    let ticket = port
        .enqueue(
            AcpCommand::ReplyElicitation {
                interaction_id: id,
                reply: RuntimeInteractionReply::Answers {
                    answers: vec![InteractionAnswer {
                        question_id: "value".into(),
                        values: vec!["x".repeat(200_000)],
                    }],
                },
            },
            command_deadline(),
        )
        .unwrap();
    port.poll().unwrap();
    assert_eq!(
        port.snapshot().unwrap().pending_interactions.len(),
        1,
        "a queued answer is not a written answer"
    );
    assert!(!port.take_receipts().iter().any(|r| r.ticket == ticket));
    std::fs::write(port_root.join("release"), []).unwrap();
    let receipts = drive_port(&mut port, |client, receipts| {
        client.snapshot().unwrap().pending_interactions.is_empty()
                && receipts
                    .iter()
                    .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
                // Written does not prove the peer has published its counter:
                // opening/truncating the file can make it visible while empty.
                && std::fs::read_to_string(port_root.join("reply-count"))
                    .is_ok_and(|contents| contents == "1")
    });
    assert_eq!(receipts.iter().filter(|r| r.ticket == ticket).count(), 1);
    assert_eq!(
        std::fs::read_to_string(port_root.join("reply-count")).unwrap(),
        "1"
    );
    std::fs::write(sync_root.join("release"), []).unwrap();
    let id = sync.snapshot().unwrap().pending_interactions[0].id.clone();
    sync.reply(
        &id,
        RuntimeInteractionReply::Answers {
            answers: vec![InteractionAnswer {
                question_id: "value".into(),
                values: vec!["legacy answer".into()],
            }],
        },
    )
    .unwrap();
    assert!(sync.snapshot().unwrap().pending_interactions.is_empty());
    port.shutdown().unwrap();
    sync.shutdown().unwrap();
    std::fs::remove_dir_all(port_root).unwrap();
    std::fs::remove_dir_all(sync_root).unwrap();
}

#[test]
fn acp_prompt_written_only_settles_dispatch_budget_not_run_or_human_input() {
    for mode in ["budget-complete", "budget-error"] {
        let (mut client, root) = synthetic_client(mode);
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        let write_deadline = Instant::now() + std::time::Duration::from_millis(350);
        let ticket = client
            .enqueue(
                AcpCommand::Prompt {
                    text: "once".into(),
                    images: vec![],
                    run_deadline: None,
                },
                write_deadline,
            )
            .unwrap();
        let written = drive_port(&mut client, |_, receipts| {
            receipts
                .iter()
                .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
        });
        std::thread::sleep(
            write_deadline.saturating_duration_since(Instant::now())
                + std::time::Duration::from_millis(20),
        );
        std::fs::write(root.join("after-write"), []).unwrap();
        drive_port(&mut client, |client, _| {
            client.snapshot().unwrap().pending_interactions.len() == 2
        });
        assert!(client.turn_running());
        assert!(client.has_pending_operations());
        assert!(matches!(
            client.enqueue(
                AcpCommand::Prompt {
                    text: "not a retry".into(),
                    images: vec![],
                    run_deadline: None
                },
                command_deadline()
            ),
            Err(AcpAdmissionError::NeedsPreparation(_))
        ));
        assert!(client.snapshot().unwrap().messages.iter().any(|m| {
            m.blocks
                .iter()
                .any(|b| matches!(b, MessageBlock::Text { text } if text == "late output"))
        }));
        assert!(
            client
                .permissions
                .values()
                .all(|pending| pending.parent_deadline.is_none())
        );
        assert!(
            client
                .elicitations
                .values()
                .all(|pending| pending.parent_deadline.is_none())
        );
        let interactions = client.snapshot().unwrap().pending_interactions.clone();
        let mut tickets = Vec::new();
        for interaction in interactions {
            let command = if matches!(interaction.kind, InteractionKind::Approval { .. }) {
                AcpCommand::ReplyPermission {
                    interaction_id: interaction.id,
                    reply: RuntimeInteractionReply::Decision {
                        option_id: "yes".into(),
                    },
                }
            } else {
                AcpCommand::ReplyElicitation {
                    interaction_id: interaction.id,
                    reply: RuntimeInteractionReply::Answers {
                        answers: vec![InteractionAnswer {
                            question_id: "value".into(),
                            values: vec!["answer".into()],
                        }],
                    },
                }
            };
            tickets.push(client.enqueue(command, command_deadline()).unwrap());
        }
        drive_port(&mut client, |client, receipts| {
            client.snapshot().unwrap().pending_interactions.is_empty()
                && tickets.iter().all(|ticket| {
                    receipts
                        .iter()
                        .any(|r| &r.ticket == ticket && r.stage == AcpReceiptStage::Written)
                })
        });
        assert!(client.turn_running());
        assert!(client.has_pending_operations());
        std::fs::write(root.join("release"), []).unwrap();
        let terminal = drive_port(&mut client, |client, receipts| {
            !client.turn_running()
                && receipts.iter().any(|r| {
                    r.ticket == ticket
                        && matches!(
                            r.stage,
                            AcpReceiptStage::TurnCompleted { .. }
                                | AcpReceiptStage::TurnError { .. }
                        )
                })
        });
        assert_eq!(written.iter().filter(|r| r.ticket == ticket).count(), 1);
        assert_eq!(terminal.iter().filter(|r| r.ticket == ticket).count(), 1);
        assert!(
            matches!(
                &terminal.iter().find(|r| r.ticket == ticket).unwrap().stage,
                AcpReceiptStage::TurnCompleted { .. }
            ) == (mode == "budget-complete")
        );
        assert!(!client.has_pending_operations());
        assert!(!client.needs_cleanup());
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn acp_explicit_run_and_control_deadlines_preserve_written_and_require_cleanup() {
    for control in [false, true] {
        let (mut client, root) = synthetic_client(if control {
            "control-budget"
        } else {
            "budget-explicit"
        });
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        let deadline = Instant::now() + std::time::Duration::from_millis(600);
        let command = if control {
            AcpCommand::SetConfig {
                config_id: "opaque/model-id".into(),
                value: ConfigValue::Select("model/b".into()),
            }
        } else {
            AcpCommand::Prompt {
                text: "explicit budget".into(),
                images: vec![],
                run_deadline: Some(deadline),
            }
        };
        let ticket = client
            .enqueue(
                command,
                if control {
                    deadline
                } else {
                    command_deadline()
                },
            )
            .unwrap();
        let written = drive_port(&mut client, |_, receipts| {
            receipts
                .iter()
                .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
        });
        if !control {
            std::fs::write(root.join("after-write"), []).unwrap();
        }
        drive_port(&mut client, |client, _| {
            !client.snapshot().unwrap().pending_interactions.is_empty()
        });
        let parent = client.elicitations.values().next().unwrap().parent_deadline;
        assert_eq!(
            parent,
            if control { None } else { Some(deadline) },
            "session scope must not guess a control parent"
        );
        if !control {
            let permission_id = client.permissions.keys().next().unwrap().clone();
            let reply = client
                .enqueue(
                    AcpCommand::ReplyPermission {
                        interaction_id: permission_id.clone(),
                        reply: RuntimeInteractionReply::Decision {
                            option_id: "yes".into(),
                        },
                    },
                    command_deadline(),
                )
                .unwrap();
            drive_port(&mut client, |_, receipts| {
                receipts
                    .iter()
                    .any(|r| r.ticket == reply && r.stage == AcpReceiptStage::Written)
            });
            // enqueue harvests physical writes. A small reply may finish
            // before the next enqueue, so check the settled state explicitly;
            // the blocked-pipe test covers rejection while still pending.
            let duplicate = client.enqueue(
                AcpCommand::ReplyPermission {
                    interaction_id: permission_id,
                    reply: RuntimeInteractionReply::Cancel,
                },
                command_deadline(),
            );
            assert!(
                matches!(&duplicate, Err(AcpAdmissionError::NotDispatched { code, message })
                if code == "internal" && message == "ACP 权限请求已失效"),
                "{duplicate:?}"
            );
            assert_eq!(
                client.elicitations.values().next().unwrap().parent_deadline,
                Some(deadline)
            );
        }
        std::thread::sleep(
            deadline.saturating_duration_since(Instant::now())
                + std::time::Duration::from_millis(20),
        );
        if !control {
            let interaction_id = client.elicitations.keys().next().unwrap().clone();
            assert!(
                matches!(client.enqueue(AcpCommand::ReplyElicitation { interaction_id,
                    reply: RuntimeInteractionReply::Cancel }, command_deadline()),
                    Err(AcpAdmissionError::NotDispatched { code, .. }) if code == "timeout")
            );
        }
        assert!(client.poll().is_err());
        assert!(client.needs_cleanup());
        assert_eq!(
            written
                .iter()
                .filter(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
                .count(),
            1
        );
        let terminal = client.take_receipts();
        assert!(terminal.iter().any(
            |r| r.ticket == ticket && matches!(r.stage, AcpReceiptStage::Indeterminate { .. })
        ));
        assert!(
            client.has_pending_operations(),
            "failure/receipt consumption cannot release the live owner lease"
        );
        assert!(!client.snapshot().unwrap().pending_interactions.is_empty());
        client.poll().unwrap();
        assert!(client.needs_cleanup());
        client.shutdown().unwrap();
        client.take_receipts();
        assert!(!client.has_pending_operations());
        std::fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn acp_expired_budget_latches_before_late_turn_or_control_terminal() {
    for mode in ["late-complete", "late-error", "late-control"] {
        let (mut client, root) = synthetic_client(mode);
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        let deadline = Instant::now() + std::time::Duration::from_millis(350);
        let command = if mode == "late-control" {
            AcpCommand::SetConfig {
                config_id: "opaque/model-id".into(),
                value: ConfigValue::Select("model/b".into()),
            }
        } else {
            AcpCommand::Prompt {
                text: "once".into(),
                images: vec![],
                run_deadline: Some(deadline),
            }
        };
        let ticket = client
            .enqueue(
                command,
                if mode == "late-control" {
                    deadline
                } else {
                    command_deadline()
                },
            )
            .unwrap();
        let mut receipts = drive_port(&mut client, |_, receipts| {
            receipts
                .iter()
                .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
        });
        std::thread::sleep(
            deadline.saturating_duration_since(Instant::now())
                + std::time::Duration::from_millis(20),
        );
        std::fs::write(root.join("release"), []).unwrap();
        // No owner poll until the peer has emitted its genuinely late terminal.
        let gate = command_deadline();
        while !root.join("terminal-emitted").exists() {
            assert!(Instant::now() < gate);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        let result = client.poll();
        receipts.extend(client.take_receipts());
        assert!(
            result.is_err(),
            "late terminal must not erase the expired {mode} budget"
        );
        assert_eq!(result.unwrap_err().code(), "timeout");
        assert!(client.needs_cleanup());
        // The peer gate and drain interval put terminal evidence into the
        // input queue before this first post-deadline owner observation.
        let stage = &receipts
            .iter()
            .find(|r| r.ticket == ticket && r.stage != AcpReceiptStage::Written)
            .unwrap()
            .stage;
        assert!(
            match mode {
                "late-complete" => matches!(stage, AcpReceiptStage::TurnCompleted { .. }),
                "late-error" => matches!(stage, AcpReceiptStage::TurnError { .. }),
                _ => matches!(stage, AcpReceiptStage::ProtocolSuccess { .. }),
            },
            "native terminal evidence must survive expiry: {receipts:?}"
        );
        assert!(
            client.has_pending_operations(),
            "consuming evidence cannot release the failed owner before cleanup"
        );
        assert!(matches!(
            client.enqueue(
                AcpCommand::List {
                    cwd: None,
                    cursor: None
                },
                command_deadline()
            ),
            Err(AcpAdmissionError::NeedsPreparation(_))
        ));
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!root.join("unexpected-reply").exists());
        client.shutdown().unwrap();
        client.take_receipts();
        assert!(!client.has_pending_operations());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn acp_automatic_wrong_session_reply_keeps_short_write_budget_under_long_run() {
    let (mut client, root) = synthetic_client("auto-short-write");
    client.create(&root, "synthetic-acp").unwrap();
    client.enable_command_port().unwrap();
    let long = Instant::now() + std::time::Duration::from_secs(3600);
    let prompt = client
        .enqueue(
            AcpCommand::Prompt {
                text: "once".into(),
                images: vec![],
                run_deadline: Some(long),
            },
            command_deadline(),
        )
        .unwrap();
    let written = drive_port(&mut client, |client, receipts| {
        !client.snapshot().unwrap().pending_interactions.is_empty()
            && receipts
                .iter()
                .any(|r| r.ticket == prompt && r.stage == AcpReceiptStage::Written)
    });
    let id = client.elicitations.keys().next().unwrap().clone();
    // This legal, long-budget answer really blocks stdin. No cancel command
    // or mark_cancellation path participates in this regression.
    let answer = client
        .enqueue(
            AcpCommand::ReplyElicitation {
                interaction_id: id,
                reply: RuntimeInteractionReply::Answers {
                    answers: vec![InteractionAnswer {
                        question_id: "value".into(),
                        values: vec!["x".repeat(200_000)],
                    }],
                },
            },
            long,
        )
        .unwrap();
    std::fs::write(root.join("automatic"), []).unwrap();
    let queued = drive_port(&mut client, |client, _| {
        client.snapshot().unwrap().messages.iter().any(|m| {
            m.blocks.iter().any(|b|
            matches!(b, MessageBlock::Text { text } if text == "automatic cancellation required"))
        })
    });
    assert!(
        !queued
            .iter()
            .any(|r| r.ticket == answer && r.stage == AcpReceiptStage::Written)
    );
    assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 1);
    std::thread::sleep(CONTROL_TIMEOUT + std::time::Duration::from_millis(100));
    let result = client.poll();
    assert!(
        result.is_err(),
        "automatic cancellation must use its short write budget, not the hour-long run"
    );
    assert_eq!(result.unwrap_err().code(), "timeout");
    assert!(client.needs_cleanup());
    let receipts = client.take_receipts();
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket.wire_id == json!("wrong-session")
                && matches!(r.stage, AcpReceiptStage::Indeterminate { .. }))
    );
    assert!(
        written
            .iter()
            .any(|r| r.ticket == prompt && r.stage == AcpReceiptStage::Written)
    );
    assert!(client.has_pending_operations());
    client.shutdown().unwrap();
    client.take_receipts();
    assert!(!client.has_pending_operations());
    std::fs::remove_dir_all(root).unwrap();
}
