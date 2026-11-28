//! On-demand execution preparation and process/gateway lifecycle.
use super::*;
use crate::conversation::ConversationSnapshot;

use velune_agent_runtime::execution::{ExecutionTarget, LoopbackTunnel};
use velune_gateway::{
    CredentialResolver, ExecutionBackends, QuotaSource, Runner, SchedulingState, SessionIdentity,
    SharedAnalyticsSink,
};

pub(super) struct SessionExecution {
    pub(super) runtime: RuntimeInstance,
    /// Project preparation remains on the compatibility submission path.
    pub(super) native_project: bool,
    gateway: Option<GatewayExecution>,
}

impl SessionExecution {
    pub(super) fn native(runtime: RuntimeInstance) -> Self {
        Self {
            runtime,
            native_project: false,
            gateway: None,
        }
    }

    pub(super) fn managed(runtime: RuntimeInstance, gateway: GatewayExecution) -> Self {
        Self {
            runtime,
            native_project: false,
            gateway: Some(gateway),
        }
    }

    pub(super) fn gateway(&self) -> Option<&GatewayExecution> {
        self.gateway.as_ref()
    }
}

/// One Runner and the exact routing and backend control authority it captured.
pub(super) struct GatewayExecution {
    config: GatewayConfig,
    identity: SessionIdentity,
    backends: ExecutionBackends,
    runner: Runner,
    tunnel: Option<LoopbackTunnel>,
}

impl GatewayExecution {
    pub(super) fn config(&self) -> &GatewayConfig {
        &self.config
    }

    pub(super) fn runner(&self) -> &Runner {
        &self.runner
    }

    pub(super) fn endpoint(&self) -> &str {
        self.tunnel
            .as_ref()
            .map_or(self.runner.endpoint(), LoopbackTunnel::endpoint)
    }

    pub(super) fn via(mut self, target: &ExecutionTarget) -> Result<Self, RuntimeError> {
        self.tunnel = Some(
            LoopbackTunnel::open(target, self.runner.endpoint())
                .map_err(|error| RuntimeError::context("网关 SSH 转发", error))?,
        );
        Ok(self)
    }

    pub(super) fn begin_user_turn(&self) -> Result<(), velune_ai::InvalidContract> {
        self.backends
            .begin_user_turn(&self.config.id, &self.identity)
    }

    pub(super) fn cancel(&self) {
        self.backends
            .cancel_session(&self.config.id, &self.identity);
    }

    pub(super) fn finish_user_turn(&self) {
        self.backends
            .finish_user_turn(&self.config.id, &self.identity);
    }

    #[cfg(all(test, unix))]
    pub(super) fn uses_backends(&self, backends: &ExecutionBackends) -> bool {
        Arc::ptr_eq(&self.backends.claude_code, &backends.claude_code)
            && Arc::ptr_eq(&self.backends.antigravity, &backends.antigravity)
    }
}

/// Captures dispatch inputs on the Core owner, including for a worker launch.
pub(super) struct GatewayLaunch {
    config: GatewayConfig,
    resolver: Arc<dyn CredentialResolver>,
    aliases: BTreeMap<String, String>,
    analytics: SharedAnalyticsSink,
    scheduling: SchedulingState,
    quota: Arc<dyn QuotaSource>,
    identity: SessionIdentity,
    backends: ExecutionBackends,
    telemetry: Option<Arc<velune_telemetry::ClientTelemetry>>,
}

impl GatewayLaunch {
    pub(super) fn start(self) -> Result<GatewayExecution, RuntimeError> {
        let runner = Runner::start_with_scheduling(
            self.config.to_gateway_config()?,
            self.resolver,
            self.aliases,
            Some(self.analytics),
            self.scheduling,
            self.quota,
            Some(self.identity.clone()),
            self.backends.clone(),
        )
        .map_err(|error| RuntimeError::context("gateway startup", error))?;
        runner.set_telemetry(self.telemetry, None, None);
        Ok(GatewayExecution {
            config: self.config,
            identity: self.identity,
            backends: self.backends,
            runner,
            tunnel: None,
        })
    }
}

impl CoreRuntime {
    pub(super) fn prepare_runtime(
        &mut self,
        runtime_id: &str,
        model_record_key: &str,
    ) -> Result<(), RuntimeError> {
        let owner = self.telemetry.clone();
        telemetry::observe(owner.as_ref(), "harness.prepare", || {
            self.prepare_runtime_inner(runtime_id, model_record_key)
        })
    }
    fn prepare_runtime_inner(
        &mut self,
        runtime_id: &str,
        model_record_key: &str,
    ) -> Result<(), RuntimeError> {
        let runtime = self
            .runtime_instances
            .iter()
            .find(|item| item.id == runtime_id)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        let gateway = self
            .gateways
            .iter()
            .find(|item| item.id == runtime.gateway_id)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
        for key in gateway.members(model_record_key) {
            if let Ok(member) = gateway.resolve_dispatch(&key) {
                let _ = self
                    .subscription_quota
                    .query(&gateway.id, &member.provider().id, false);
            }
        }
        // Stopping the previous runtime removes its native projection. Capture
        // the public identity first, and give a new, unnamed conversation its
        // own execution identity until the harness creates its native ID.
        let conversation = self.session.conversation_id().map(str::to_owned);
        if runtime.type_id != RuntimeKind::Pi {
            self.shutdown_active()?;
            let runtime_projection = self.execution_context(&runtime)?.projection;
            let lane = self.lane_projection(&runtime_projection);
            let aliases = self.native_aliases(&runtime, model_record_key)?;
            let gateway = self
                .gateway_launch(gateway, aliases, conversation)
                .start()?;
            let injection = Self::native_injection(&runtime, &gateway, model_record_key)?;
            let mut config = self.native_config(&runtime, Some(injection))?;
            config.auto_compaction =
                runtime.model_auto_compaction(&crate::ModelSelection::Managed {
                    model_record_key: model_record_key.to_owned(),
                })?;
            config.projection_directory = lane;
            config.mcp_servers = self.collaboration_servers();
            config.session_agents = self.turn_subagents(&runtime);
            let session = telemetry::observe(self.telemetry.as_ref(), "harness.start", || {
                NativeSession::connect(config)
            })
            .map_err(|error| RuntimeError::context("运行时连接失败", error))?;
            self.session.install_execution(
                ActiveState::Native(Box::new(session)),
                SessionExecution::managed(runtime, gateway),
            );
            self.session.model_record_key = None;
            return Ok(());
        }
        let pi_composition::PiManagedModel {
            physical_id,
            aliases,
            protocol,
        } = pi_composition::PiManagedModel::resolve(&gateway, model_record_key)?;
        self.shutdown_active()?;
        let context = self.prepare_execution_context(&runtime)?;
        let gateway = self
            .gateway_launch(gateway, aliases, conversation)
            .start()?
            .via(&context.target)?;
        let projection_dir = self.lane_projection(&context.projection);
        let config = PiConfig {
            mcp_servers: self.collaboration_servers(),
            binary: setting_path(&runtime, "binary")?,
            node_binary: setting_path_optional(&runtime, "nodeBinary"),
            sdk_helper: setting_path_optional(&runtime, "sdkHelper")
                .or_else(|| Some(context.resources.join("pi_sessions.mjs"))),
            extension: Some(context.resources.join("pi_virtual_model.mjs")),
            agent_dir: Some(context.agent_dir),
            execution: context.target,
            os_home: context.os_home,
            working_dir: None,
            provider: Some("velune".into()),
            model: Some("auto".into()),
            rpc_entry: Some(context.resources.join("pi_rpc.mjs")),
            models_path: Some(projection_dir.join("models.json")),
            selection_file: Some(projection_dir.join("velune-selection.json")),
            model_compaction_reserves: pi_composition::pi_compaction_reserves(
                &runtime,
                Some(model_record_key),
            )?,
            gateway_token: Some(gateway.runner().token().into()),
            session_dir: setting_path_optional(&runtime, "sessionDir"),
            session: None,
            side: false,
            // A session's Pi name belongs to its persisted session_info entry.
            // Runtime names must not overwrite it when reopening a session.
            name: None,
        };
        config
            .validate_sdk()
            .map_err(|error| RuntimeError::Invalid(error.to_string()))?;
        materialize_models(&config, gateway.config(), gateway.endpoint(), protocol)?;
        write_selection_file(&config, model_record_key, &physical_id)?;
        self.session.install_execution(
            ActiveState::Pi(Box::new(pi::PiSession::new(config))),
            SessionExecution::managed(runtime, gateway),
        );
        self.session.model_record_key = None;
        Ok(())
    }

    pub(super) fn gateway_launch(
        &self,
        config: GatewayConfig,
        aliases: BTreeMap<String, String>,
        conversation_id: Option<String>,
    ) -> GatewayLaunch {
        let (namespace, id) = match conversation_id {
            Some(id) => ("velune.conversation", id),
            None => ("velune.prepared_execution", crate::new_record_key()),
        };
        GatewayLaunch {
            resolver: crate::authentication_resolver::ProviderResolver::capture_for_dispatch(
                &config,
                &self.options,
                self.subscription_quota.clone(),
            ),
            quota: self.subscription_quota.source(&config.id),
            config,
            aliases,
            analytics: self.analytics.clone(),
            scheduling: self.scheduling_state.clone(),
            identity: SessionIdentity {
                namespace: namespace.into(),
                id,
            },
            backends: self.execution_backends.clone(),
            telemetry: self.telemetry.clone(),
        }
    }

    pub(super) fn start_pi_for_session(
        &mut self,
        cwd: Option<&Path>,
        source: Option<&ConversationSnapshot>,
        model_record_key: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let runtime = self
            .session
            .execution()
            .ok_or_else(|| RuntimeError::invalid("Pi 执行运行时不可用"))?
            .runtime
            .clone();
        let runtime_context = format!(
            "runtime startup；{}",
            super::browsing::runtime_context(&runtime)
        );
        let runtime_start_error = |phase: &'static str, error| {
            tracing::warn!(
                target: "velune_application",
                event = "runtime_start_failed",
                phase,
                runtime_id = %runtime.id,
                runtime_name = %runtime.name,
                runtime_type_id = %runtime.type_id,
                binary = ?runtime.settings.get("binary"),
                node_binary = ?runtime.settings.get("nodeBinary"),
                agent_dir = ?runtime.settings.get("agentDir"),
                detail = %error,
            );
            RuntimeError::context(&runtime_context, error)
        };
        if let Some(cwd) = cwd {
            self.validate_runtime_cwd(&runtime, cwd)?;
        }
        let cwd = cwd.map(Path::to_owned);
        let session = source
            .map(|source| {
                source
                    .conversation
                    .id
                    .strip_prefix(&format!("{}:", runtime.id))
                    .filter(|path| !path.is_empty())
                    .map(Path::new)
                    .ok_or_else(|| RuntimeError::invalid("conversation id"))
            })
            .transpose()?;
        let mut config = self.session.pi()?.config().clone();
        self.shutdown_pi()?;
        config.working_dir = cwd.clone();
        config.session = session.map(Path::to_owned);
        config.side = self.session.side.is_some();
        config.name = None;
        let mut client = telemetry::observe(self.telemetry.as_ref(), "harness.start", || {
            pi::Client::spawn(config.clone())
        })
        .map_err(|error| runtime_start_error("startup", error))?;
        // Establish readiness through RPC before exposing bootstrap events to
        // the projection. Pi can emit initialization records before its first
        // command response; those records must not settle a new turn.
        let (_, readiness) = client
            .state()
            .map_err(|error| runtime_start_error("startup", error))?;
        let reported_path = readiness["nativeSession"]["sessionFile"]
            .as_str()
            .map(Path::new);
        if !config.side
            && let (Some(expected), Some(reported)) = (session, reported_path)
            && reported != expected
        {
            return Err(super::browsing::history_read_changed());
        }
        let runtime_id = runtime.id.as_str();
        let mut projection = if let Some(source) = source.filter(|_| !config.side) {
            PiProjection::from_history(source)
        } else {
            // The fresh SDK SessionManager already owns the new conversation.
            // Use its reserved path even before the first message persists it.
            let path = if config.side { session } else { reported_path };
            let id = match path.filter(|path| !path.as_os_str().is_empty()) {
                Some(path) => format!("{runtime_id}:{}", path.display()),
                None if config.side => "active".into(),
                None => {
                    return Err(runtime_start_error(
                        "startup",
                        pi::Error::Sdk("Pi 未返回新会话的原生文件身份".into()),
                    ));
                }
            };
            PiProjection::new(ConversationSummary {
                native_read_state: None,
                can_rename: false,
                can_delete: false,
                id,
                title: velune_conversation::ConversationTitle::Untitled,
                updated_at_unix_ms: None,
                created_at_unix_ms: None,
                runtime_id: runtime_id.into(),
                cwd: cwd.as_ref().map(|path| path.to_string_lossy().into_owned()),
                parent_conversation_id: None,
            })
        };
        drop(readiness);
        if let Some(snapshot) = projection.snapshot.as_mut() {
            snapshot.model_record_key = model_record_key.map(str::to_owned);
        }
        let session = self.session.pi()?;
        *session.config_mut() = config;
        session.start(client, projection);
        self.session.model_record_key = model_record_key.map(str::to_owned);
        self.drain_runtime()?;
        Ok(())
    }

    pub(super) fn shutdown_pi(&mut self) -> Result<(), RuntimeError> {
        let stopped = match &mut self.session.active_state {
            ActiveState::Pi(session) => session.stop(),
            _ => Ok(()),
        };
        self.session.model_record_key = None;
        stopped.map_err(|error| RuntimeError::context("runtime shutdown", error))
    }

    pub(super) fn shutdown_active(&mut self) -> Result<(), RuntimeError> {
        let owner = self.telemetry.clone();
        telemetry::observe(owner.as_ref(), "harness.stop", || {
            self.shutdown_active_inner()
        })
    }
    fn shutdown_active_inner(&mut self) -> Result<(), RuntimeError> {
        if matches!(self.session.active_state, ActiveState::Pending(_))
            || self.session.native_controls.pending()
        {
            return Err(RuntimeError::Busy);
        }
        let cancellation_requested = self.session.cancel_requested;
        self.session.refresh_pending_native_fork()?;
        let mut state = std::mem::replace(&mut self.session.active_state, ActiveState::Empty);
        self.session.native_controls = Default::default();
        self.session.compacting = false;
        let result = match state.live_mut() {
            Some(live) => live
                .shutdown()
                .map_err(|error| RuntimeError::context("runtime shutdown", error)),
            None => Ok(()),
        };
        self.session.clear_execution();
        self.session.model_record_key = None;
        self.complete_turn_observation(if result.is_ok() && cancellation_requested {
            velune_telemetry::Outcome::Cancelled
        } else {
            velune_telemetry::Outcome::Unknown
        });
        result
    }
}

#[cfg(all(test, unix))]
pub(super) mod gateway_execution_tests {
    use super::*;
    use crate::conversation::ConversationSnapshot;
    use axum::{
        Json, Router,
        http::{HeaderMap, Uri},
    };
    use std::{os::unix::fs::PermissionsExt, sync::Mutex, time::Duration};

    pub(in crate::local) fn provider(id: &str, endpoint: &str) -> crate::ProviderDraft {
        crate::ProviderDraft {
            id: id.into(),
            name: id.into(),
            protocol: Protocol::Responses,
            endpoint: endpoint.into(),
            execution_backend: crate::config::ExecutionBackend::Http,
            request_profile: crate::config::RequestProfile::Native,
            codex_client_version: None,
            authentication_source: crate::ProviderAuthenticationSource::ApiKey,
            quota_source: crate::config::ProviderQuotaSource::None,
            models: vec![velune_application_api::ProviderModel {
                record_key: id.into(),
                provider_model_id: "shared-model-name".into(),
                nickname: id.into(),
                icon: None,
                context_window: Some(8192),
                max_output_tokens: Some(1024),
                reasoning_levels: None,
                request_parameters: None,
            }],
            quota_limits: Vec::new(),
        }
    }

    pub(in crate::local) fn save(core: &mut CoreRuntime, draft: crate::ProviderDraft, key: &str) {
        core.save_provider(
            "default".into(),
            draft,
            crate::AuthenticationEdit::SetApiKey { value: key.into() },
            None,
        )
        .unwrap();
    }

    pub(in crate::local) fn snapshot(id: &str) -> ConversationSnapshot {
        PiProjection::new(ConversationSummary {
            native_read_state: None,
            id: id.into(),
            runtime_id: "pi".into(),
            title: velune_conversation::ConversationTitle::Untitled,
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            cwd: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        })
        .snapshot
        .unwrap()
    }

    const PI_EXECUTION_PEER: &str = r#"#!/usr/bin/python3
import json, pathlib, sys, threading
root = pathlib.Path(__file__).resolve().parent
if '--validate' in sys.argv:
    sys.exit(0)
if '--version' in sys.argv:
    print('1.4.0')
    sys.exit(0)
mode = (root / 'mode').read_text()
session_file = root / 'session.jsonl'
if mode == 'create':
    if '--session' in sys.argv:
        session_file = pathlib.Path(sys.argv[sys.argv.index('--session') + 1])
    with (root / 'launches').open('a') as out:
        out.write(str(session_file) + '\n')
history_file = root / 'messages.json'
messages = json.loads(history_file.read_text()) if history_file.exists() else []
release = threading.Event()
output = threading.Lock()

def emit(value):
    with output:
        print(json.dumps(value), flush=True)

def message(value):
    messages.append(value)
    history_file.write_text(json.dumps(messages))
    emit({'type':'message_start', 'message':value})
    emit({'type':'message_end', 'message':value})

def finish(number):
    if number == 1:
        while not (root / 'release').exists() and not release.wait(.01):
            pass
    reason = {'failed':'error', 'stopped':'aborted'}.get(mode, 'stop') if number == 1 else 'stop'
    message({'id':f'reply-{number}', 'role':'assistant', 'stopReason':reason,
             'errorMessage':'synthetic model failure' if reason == 'error' else None,
             'content':[{'type':'text', 'text':f'reply {number}'}]})
    emit({'type':'agent_settled', 'aborted':reason == 'aborted'})

for line in sys.stdin:
    request = json.loads(line)
    command, identity = request['type'], request['id']
    data = {}
    if command == 'get_state':
        data = {'nativeSession': {'sessionFile':str(session_file)}}
    elif command == 'get_commands':
        data = {'commands':[]}
    elif command == 'get_messages':
        data = {'messages':messages}
    elif command == 'new_session':
        session_file = root / 'replacement-session.jsonl'
        messages.clear()
        (root / 'new-session').touch()
    elif command == 'prompt':
        text = request['message']
        if text.startswith('/velune-projection-sync '):
            emit({'type':'extension_ui_request', 'method':'setStatus',
                  'statusKey':'velune.session-metadata',
                  'statusText':json.dumps({'requestId':identity, 'snapshotVersion':1,
                      'messages':messages, 'messageIds':[m['id'] for m in messages],
                      'identityConfirmations':[], 'sessionFile':str(session_file),
                      'name':'Synthetic Pi queue', 'cwd':str(root)})})
            data = {'disposition':'handled'}
        elif text.startswith('/velune-background-work '):
            frame = {'version':1, 'kind':'snapshot', 'parentSessionFile':str(session_file),
                     'tasks':[]}
            emit({'type':'extension_ui_request', 'method':'setStatus',
                  'statusKey':'velune.background-work-v1', 'statusText':json.dumps(frame)})
            data = {'disposition':'handled'}
        else:
            wire = root / 'wire'
            with wire.open('a') as out:
                out.write(text + '\n')
            number = len(wire.read_text().splitlines())
            emit({'type':'agent_start'})
            if mode == 'create':
                with session_file.open('a') as out:
                    out.write(text + '\n')
            message({'id':f'user-{number}', 'role':'user', 'content':text})
            threading.Thread(target=finish, args=(number,), daemon=True).start()
            data = {'disposition':'started'}
    elif command == 'abort':
        release.set()
    emit({'type':'response', 'id':identity, 'command':command, 'success':True, 'data':data})
"#;

    pub(in crate::local) fn pi_execution_fixture(mode: &str) -> (PathBuf, crate::Application) {
        let root =
            std::env::temp_dir().join(format!("velune-pi-execution-{}", crate::new_record_key()));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        fs::write(root.join("mode"), mode).unwrap();
        let cli = root.join("pi");
        fs::write(&cli, PI_EXECUTION_PEER).unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        let node = root.join("node");
        fs::write(
            &node,
            r#"#!/usr/bin/python3
import json, os, sys
if sys.argv[1] == '--input-type=module':
    print(json.dumps({'binary':sys.argv[-1]}))
else:
    os.execv(sys.executable, [sys.executable, *sys.argv[1:]])
"#,
        )
        .unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("pi_rpc.mjs"), PI_EXECUTION_PEER).unwrap();
        let mut app = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        app.runtime
            .upsert_runtime_with_execution(
                RuntimeInstance {
                    enabled: true,
                    id: "pi".into(),
                    name: "Synthetic Pi".into(),
                    type_id: RuntimeKind::Pi,
                    gateway_id: "default".into(),
                    settings: BTreeMap::from([
                        ("binary".into(), cli.display().to_string()),
                        ("nodeBinary".into(), node.display().to_string()),
                        ("agentDir".into(), root.join("agent").display().to_string()),
                    ]),
                },
                crate::RuntimeExecution {
                    home: crate::RuntimeHome::Existing {
                        path: root.display().to_string(),
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        (root, app)
    }

    #[test]
    fn pi_create_returns_the_native_identity_for_immediate_first_send() {
        use crate::{
            ClientViewId, Command, ComposedTurn, ModelSelection, Response, SubmissionReceipt,
        };
        use std::time::Instant;

        for selection in [
            ModelSelection::Native {
                provider_id: Some("synthetic-native".into()),
                model_id: "shared-model-name".into(),
            },
            ModelSelection::Managed {
                model_record_key: "synthetic".into(),
            },
        ] {
            let (root, mut app) = pi_execution_fixture("create");
            if matches!(&selection, ModelSelection::Managed { .. }) {
                save(
                    &mut app.runtime,
                    provider("synthetic", "http://127.0.0.1:9/v1"),
                    "fake",
                );
            }
            let view = ClientViewId("pi-create".into());
            let Response::CreateConversation(created) = app
                .execute_command(
                    Command::CreateConversation {
                        runtime_id: "pi".into(),
                        cwd: Some(root.display().to_string()),
                        project: None,
                        selection: Some(selection.clone()),
                    },
                    false,
                    &view,
                )
                .unwrap()
            else {
                panic!("expected creation response");
            };
            let session_file = root.join("session.jsonl");
            let id = created.snapshot.unwrap().conversation.id;
            assert_eq!(id, format!("pi:{}", session_file.display()));
            assert!(
                !session_file.exists(),
                "the native identity precedes persistence"
            );

            // Send directly with the creation receipt, with no snapshot or poll in between.
            let Response::SendTurn(sent) = app
                .execute_command(
                    Command::SendTurn {
                        submission_id: None,
                        source_runtime_id: "pi".into(),
                        conversation_id: id.clone(),
                        target_runtime_id: "pi".into(),
                        selection: Some(selection),
                        turn: ComposedTurn {
                            text: "first input".into(),
                            effort: None,
                            permission_mode: None,
                            attachments: Vec::new(),
                        },
                    },
                    false,
                    &view,
                )
                .unwrap()
            else {
                panic!("expected send response");
            };
            assert_eq!(sent.submission_receipt, Some(SubmissionReceipt::Sending));
            assert_eq!(sent.snapshot.unwrap().conversation.id, id);

            fs::write(root.join("release"), "").unwrap();
            let deadline = Instant::now() + Duration::from_secs(4);
            while !fs::metadata(&session_file).is_ok_and(|file| file.len() > 0)
                || app.runtime.session.executing()
            {
                assert!(
                    Instant::now() < deadline,
                    "first input did not settle in the created session"
                );
                app.runtime.drain_runtime().unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
            app.close().unwrap();
            assert_eq!(fs::read_to_string(&session_file).unwrap(), "first input\n");
            assert_eq!(
                fs::read_to_string(root.join("wire")).unwrap(),
                "first input\n"
            );
            assert_eq!(
                fs::read_to_string(root.join("launches")).unwrap(),
                format!("{}\n", session_file.display())
            );
            assert!(!root.join("new-session").exists());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn managed_pi_terminal_settles_queue_after_cleanup() {
        use crate::conversation::QueuePause;
        use crate::local::execution_workers::cleanup_test_gate;
        use std::time::Instant;

        fn until(core: &mut CoreRuntime, stage: &str, condition: impl Fn(&CoreRuntime) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(4);
            while !condition(core) {
                assert!(
                    Instant::now() < deadline,
                    "synthetic Pi did not reach {stage}"
                );
                core.drain_runtime().unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        for (mode, pause) in [
            ("completed", None),
            ("failed", Some(QueuePause::TurnFailed)),
            ("stopped", Some(QueuePause::TurnStopped)),
        ] {
            let (root, mut app) = pi_execution_fixture(mode);
            let core = &mut app.runtime;
            save(core, provider("synthetic", "http://127.0.0.1:9/v1"), "fake");
            let selection = Some(crate::ModelSelection::Managed {
                model_record_key: "synthetic".into(),
            });
            let id = format!("pi:{}", root.join("session.jsonl").display());
            let mut summary = snapshot(&id).conversation;
            summary.cwd = Some(root.display().to_string());
            let mut existing = PiProjection::new(summary);
            let prior = [
                json!({"id":"older-user", "role":"user", "content":"earlier question"}),
                json!({"id":"older-reply", "role":"assistant", "content":"earlier reply",
                       "stopReason":"stop"}),
            ];
            existing.begin_turn();
            existing.apply_event(&json!({"type":"agent_start"}));
            for message in &prior {
                existing.apply_event(&json!({"type":"message_start", "message":message}));
                existing.apply_event(&json!({"type":"message_end", "message":message}));
            }
            existing.apply_event(&json!({"type":"agent_settled"}));
            assert!(existing.turn_ended());
            fs::write(
                root.join("messages.json"),
                serde_json::to_vec(&prior).unwrap(),
            )
            .unwrap();
            core.session.set_history(existing.snapshot.unwrap());
            let turn = |text: &str| crate::ComposedTurn {
                text: text.into(),
                effort: None,
                permission_mode: None,
                attachments: Vec::new(),
            };
            core.send_turn(crate::local::TurnRequest {
                submission_id: None,
                source_runtime_id: "pi".into(),
                conversation_id: id.clone(),
                runtime_id: "pi".into(),
                selection: selection.clone(),
                turn: turn("first"),
            })
            .unwrap();
            until(core, "running first turn", |core| {
                core.turn_running()
                    && core.session.projected().is_some_and(|view| {
                        view.messages.iter().any(|message| message.id == "user-1")
                    })
            });
            core.send_queue(
                &id,
                crate::SendQueueOperation::Add {
                    source_runtime_id: "pi".into(),
                    target_runtime_id: "pi".into(),
                    selection,
                    turn: turn("queued"),
                },
            )
            .unwrap();
            let (entered, release) = cleanup_test_gate::register(&root.join("pi"));
            if mode == "stopped" {
                core.cancel("pi").unwrap();
            } else {
                fs::write(root.join("release"), "").unwrap();
            }
            until(core, "cleanup worker admission", |core| {
                matches!(core.session.active_state, ActiveState::Pending(_))
            });
            entered.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(fs::read_to_string(root.join("wire")).unwrap(), "first\n");
            assert_eq!(core.current_snapshot().unwrap().follow_up.queue.len(), 1);
            release.send(()).unwrap();

            until(core, "idle after cleanup", |core| {
                matches!(core.session.active_state, ActiveState::History(_))
                    && core.session.gateway().is_none()
            });
            let shown = core.current_snapshot().unwrap();
            assert!(
                shown
                    .messages
                    .iter()
                    .any(|message| message.id == "older-user")
            );
            assert_eq!(shown.follow_up.paused, pause, "{mode}");
            assert_eq!(
                shown.follow_up.queue.len(),
                usize::from(pause.is_some()),
                "{mode}"
            );
            let expected = if pause.is_some() {
                "first\n"
            } else {
                "first\nqueued\n"
            };
            assert_eq!(
                fs::read_to_string(root.join("wire")).unwrap(),
                expected,
                "{mode}"
            );
            app.close().unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn pi_resume_publishes_existing_transcript_before_another_native_sync() {
        let root =
            std::env::temp_dir().join(format!("velune-pi-resume-{}", crate::new_record_key()));
        fs::create_dir(&root).unwrap();
        let helper = root.join("rpc.py");
        fs::write(
            &helper,
            r#"
import json, pathlib, sys
def emit(record): print(json.dumps(record), flush=True)
session = sys.argv[sys.argv.index('--session') + 1]
for line in sys.stdin:
    request = json.loads(line)
    command, request_id = request['type'], request['id']
    data = {}
    if command == 'prompt':
        assert request['message'].startswith('/velune-projection-sync ')
        old = {'id':'pi:session:old:0','role':'user','content':'synthetic earlier prompt'}
        frame = {'requestId':request_id, 'snapshotVersion':1, 'sessionFile':session,
                 'messages':[old], 'messageIds':[old['id']], 'identityConfirmations':[]}
        emit({'type':'extension_ui_request','method':'setStatus',
              'statusKey':'velune.session-metadata','statusText':json.dumps(frame)})
        data = {'disposition':'handled'}
    emit({'type':'response','id':request_id,'command':command,'success':True,'data':data})
"#,
        )
        .unwrap();
        let mut app = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut app.runtime;
        core.runtime_instances.push(RuntimeInstance {
            enabled: true,
            id: "pi".into(),
            name: "Synthetic Pi".into(),
            type_id: RuntimeKind::Pi,
            gateway_id: "default".into(),
            settings: Default::default(),
        });
        let config: PiConfig = serde_json::from_value(json!({
            "execution":{"kind":"local"}, "os_home":root,
            "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3", "rpc_entry":helper,
        }))
        .unwrap();
        core.session.install_execution(
            ActiveState::Pi(Box::new(pi::PiSession::new(config))),
            SessionExecution::native(core.runtime_instance("pi").unwrap().clone()),
        );
        let id = format!("pi:{}", root.join("existing.jsonl").display());
        let mut source = PiProjection::new(snapshot(&id).conversation);
        source.replace_history(&json!({"messages":[
            {"id":"pi:session:old:0","role":"user","content":"synthetic earlier prompt"}
        ]}));
        let source = source.snapshot.unwrap();
        core.start_pi_for_session(None, Some(&source), None)
            .unwrap();
        let live = core.native_snapshot().expect("started Pi view");
        assert_eq!(live.conversation.id, source.conversation.id);
        assert_eq!(live.messages, source.messages);
        assert!(
            !core
                .session
                .pi()
                .unwrap()
                .projection()
                .unwrap()
                .history_synchronized()
        );
        core.shutdown_active().unwrap();
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    async fn send(endpoint: &str, token: &str, model: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{endpoint}/responses"))
            .bearer_auth(token)
            .timeout(Duration::from_secs(5))
            .json(&json!({"model":model, "input":"synthetic"}))
            .send()
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edits_preserve_running_snapshot_and_isolate_later_executions() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let admitted = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let captured = requests.clone();
        let arrived = admitted.clone();
        let released = release.clone();
        let upstream = Router::new().fallback(
            move |uri: Uri, headers: HeaderMap, Json(body): Json<Value>| {
                let captured = captured.clone();
                let arrived = arrived.clone();
                let released = released.clone();
                async move {
                    let first = {
                        let mut requests = captured.lock().unwrap();
                        requests.push((
                            uri.path().to_owned(),
                            headers["authorization"].to_str().unwrap().to_owned(),
                            body["model"].clone(),
                        ));
                        requests.len() == 1
                    };
                    if first {
                        arrived.notify_one();
                        released.notified().await;
                    }
                    Json(json!({
                        "id":"synthetic", "object":"response", "status":"completed", "output":[]
                    }))
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let root = std::env::temp_dir().join(format!(
            "velune-gateway-execution-{}",
            crate::new_record_key()
        ));
        fs::create_dir(&root).unwrap();
        let helper = r#"#!/usr/bin/python3
import json, sys
if '--validate' in sys.argv:
    sys.exit(0)
if '--version' in sys.argv:
    print('codex-cli 0.166.0')
    sys.exit(0)
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    if 'method' in request:
        reply = {'id':request['id'], 'result':{}}
    else:
        reply = {'type':'response', 'id':request['id'], 'command':request['type'], 'success':True, 'data':{}}
    print(json.dumps(reply), flush=True)
"#;
        fs::write(root.join("pi_rpc.mjs"), helper).unwrap();
        let codex = root.join("codex");
        fs::write(&codex, helper).unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o700)).unwrap();
        let mut app = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut app.runtime;
        for (id, kind, binary) in [
            ("pi", RuntimeKind::Pi, Path::new("/synthetic/pi")),
            ("codex", RuntimeKind::Codex166, codex.as_path()),
        ] {
            core.upsert_runtime_with_execution(
                RuntimeInstance {
                    enabled: true,
                    id: id.into(),
                    name: id.into(),
                    type_id: kind,
                    gateway_id: "default".into(),
                    settings: BTreeMap::from([
                        ("binary".into(), binary.to_string_lossy().into_owned()),
                        ("nodeBinary".into(), "/usr/bin/python3".into()),
                        (
                            "agentDir".into(),
                            root.join(id).to_string_lossy().into_owned(),
                        ),
                    ]),
                },
                crate::RuntimeExecution {
                    home: crate::RuntimeHome::Existing {
                        path: root.to_string_lossy().into_owned(),
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        }
        save(core, provider("a", &format!("{origin}/old/v1")), "old-key");
        save(core, provider("b", &format!("{origin}/b/v1")), "b-key");
        core.session.active_state = ActiveState::History(Box::new(snapshot("pi:first").into()));
        core.prepare_runtime("pi", "a").unwrap();
        assert_eq!(core.session.gateway().unwrap().identity.id, "pi:first");
        let old_gateway = core.session.gateway().unwrap().config().clone();
        let old_alias = old_gateway
            .pi_binding_id("a", old_gateway.authentication_revision("a"))
            .unwrap();
        let runner = core.session.gateway().unwrap().runner();
        let old_endpoint = runner.endpoint().to_owned();
        let old_token = runner.token().to_owned();
        let old_quota = Arc::downgrade(&core.subscription_quota);
        let pi = core.session.pi().unwrap();
        let client = pi::Client::spawn(pi.config().clone()).unwrap();
        let mut projection = PiProjection::new(snapshot("pi:first").conversation);
        projection.apply_event(&json!({"type":"agent_start"}));
        pi.start(client, projection);
        assert!(core.session.executing());
        let endpoint = old_endpoint.clone();
        let token = old_token.clone();
        let alias = old_alias.clone();
        let pending = tokio::spawn(async move { send(&endpoint, &token, &alias).await });
        tokio::time::timeout(Duration::from_secs(5), admitted.notified())
            .await
            .unwrap();
        save(core, provider("a", &format!("{origin}/new/v1")), "new-key");
        assert!(core.session.executing());
        assert!(core.session.configuration_changed);
        let revision = core.gateways[0].authentication_revision("a");
        core.save_provider(
            "default".into(),
            provider("a", &format!("{origin}/kept-key/v1")),
            crate::AuthenticationEdit::Keep,
            None,
        )
        .unwrap();
        assert!(core.gateways[0].authentication_revision("a") > revision);
        let committed = core.gateways.clone();
        fs::create_dir(root.join("generic-config.json.tmp")).unwrap();
        assert!(
            core.save_provider(
                "default".into(),
                provider("a", &format!("{origin}/rejected/v1")),
                crate::AuthenticationEdit::Keep,
                None,
            )
            .is_err()
        );
        assert_eq!(core.gateways, committed);
        fs::remove_dir(root.join("generic-config.json.tmp")).unwrap();
        assert_eq!(
            core.session.gateway().map(GatewayExecution::config),
            Some(&old_gateway)
        );
        release.notify_one();
        assert_eq!(pending.await.unwrap().status(), 200);
        assert_eq!(
            send(&old_endpoint, &old_token, &old_alias).await.status(),
            200
        );

        core.focus_new();
        core.prepare_runtime("codex", "b").unwrap();
        let second_identity = core.session.gateway().unwrap().identity.clone();
        let runner = core.session.gateway().unwrap().runner();
        assert_ne!(runner.endpoint(), old_endpoint);
        assert_eq!(
            send(runner.endpoint(), runner.token(), "shared-model-name")
                .await
                .status(),
            200
        );
        core.focus_new();
        core.prepare_runtime("codex", "a").unwrap();
        assert_ne!(core.session.gateway().unwrap().identity, second_identity);
        let runner = core.session.gateway().unwrap().runner();
        assert_eq!(
            send(runner.endpoint(), runner.token(), "shared-model-name")
                .await
                .status(),
            200
        );
        assert_eq!(
            *requests.lock().unwrap(),
            vec![
                (
                    "/old/v1/responses".into(),
                    "Bearer old-key".into(),
                    json!("shared-model-name")
                ),
                (
                    "/old/v1/responses".into(),
                    "Bearer old-key".into(),
                    json!("shared-model-name")
                ),
                (
                    "/b/v1/responses".into(),
                    "Bearer b-key".into(),
                    json!("shared-model-name")
                ),
                (
                    "/kept-key/v1/responses".into(),
                    "Bearer new-key".into(),
                    json!("shared-model-name")
                ),
            ]
        );
        assert!(core.focus_existing("pi:first"));
        core.shutdown_active().unwrap();
        assert!(old_quota.upgrade().is_none());
        let address = url::Url::parse(&old_endpoint)
            .unwrap()
            .socket_addrs(|| None)
            .unwrap()[0];
        assert!(std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_err());

        let mut runtime = core.runtime_instance("codex").unwrap().clone();
        runtime.settings.insert(
            "binary".into(),
            root.join("missing").to_string_lossy().into_owned(),
        );
        core.upsert_runtime(runtime.clone()).unwrap();
        assert!(core.prepare_runtime("codex", "a").is_err());
        assert!(core.session.execution().is_none());
        assert!(core.active_instance().is_err());
        runtime
            .settings
            .insert("binary".into(), codex.to_string_lossy().into_owned());
        core.upsert_runtime(runtime).unwrap();
        core.prepare_runtime("codex", "a").unwrap();
        core.close_if_idle(true).unwrap();
        drop(app);
        server.abort();
        fs::remove_dir_all(root).unwrap();
    }
}
