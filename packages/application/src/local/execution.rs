//! On-demand execution preparation and process/gateway lifecycle.
use super::*;

pub(super) struct ExecutionConfiguration {
    pub(super) runtime: RuntimeInstance,
    pub(super) gateway: Option<GatewayConfig>,
    pub(super) gateway_session: Option<velune_gateway::SessionIdentity>,
    pub(super) backends: velune_gateway::ExecutionBackends,
}

impl CoreRuntime {
    pub(super) fn capture_execution_configuration(
        &mut self,
        runtime: &RuntimeInstance,
        gateway: Option<(&GatewayConfig, velune_gateway::SessionIdentity)>,
    ) {
        self.session.execution_configuration = Some(ExecutionConfiguration {
            runtime: runtime.clone(),
            gateway: gateway.as_ref().map(|(gateway, _)| (*gateway).clone()),
            gateway_session: gateway.map(|(_, session)| session),
            backends: self.execution_backends.clone(),
        });
        self.session.configuration_changed = false;
    }

    pub(super) fn session_backends(&self) -> &velune_gateway::ExecutionBackends {
        self.session
            .execution_configuration
            .as_ref()
            .map_or(&self.execution_backends, |configuration| {
                &configuration.backends
            })
    }

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
        let session = self
            .session
            .conversation_id()
            .map(|id| velune_gateway::SessionIdentity {
                namespace: "velune.conversation".into(),
                id: id.into(),
            })
            .unwrap_or_else(|| velune_gateway::SessionIdentity {
                namespace: "velune.prepared_execution".into(),
                id: crate::new_record_key(),
            });
        if runtime.type_id != RuntimeKind::Pi {
            self.shutdown_active()?;
            self.capture_execution_configuration(&runtime, Some((&gateway, session)));
            let runtime_projection = self.execution_context(&runtime)?.projection;
            let lane = self.lane_projection(&runtime_projection);
            let runner = self
                .start_gateway_runner(&gateway, self.native_aliases(&runtime, model_record_key)?)?;
            self.session.gateway_runner = Some(runner);
            let servers = self.collaboration_servers();
            let result = (|| {
                let injection = self.native_injection(&runtime, model_record_key)?;
                let mut config = self.native_config(&runtime, Some(injection))?;
                config.projection_directory = lane;
                config.mcp_servers = servers;
                config.session_agents = self.turn_subagents(&runtime);
                telemetry::observe(self.telemetry.as_ref(), "harness.start", || {
                    NativeSession::connect(config)
                })
                .map_err(|error| RuntimeError::context("运行时连接失败", error))
            })();
            match result {
                Ok(session) => {
                    self.session.active_state = ActiveState::Native(Box::new(session));
                    self.session.execution_runtime_id = Some(runtime.id);
                    self.session.model_record_key = None;
                    return Ok(());
                }
                Err(error) => {
                    self.session.gateway_runner = None;
                    return Err(error);
                }
            }
        }
        runnable_pi_model(&gateway, model_record_key)?;
        let physical_model_id = gateway
            .pi_binding_id(
                model_record_key,
                gateway.authentication_revision(model_record_key),
            )
            .map_err(RuntimeError::invalid)?;
        let route_aliases = gateway
            .selectable_keys()
            .map(|key| {
                gateway
                    .pi_binding_id(key, gateway.authentication_revision(key))
                    .map(|alias| (alias, key.to_owned()))
                    .map_err(RuntimeError::invalid)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        self.shutdown_active()?;
        self.capture_execution_configuration(&runtime, Some((&gateway, session)));
        let runner = self.start_gateway_runner(&gateway, route_aliases)?;
        let context = self.prepare_execution_context(&runtime)?;
        let tunnel = velune_agent_runtime::execution::LoopbackTunnel::open(
            &context.target,
            runner.endpoint(),
        )
        .map_err(|error| RuntimeError::context("网关 SSH 转发", error))?;
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
            gateway_token: Some(runner.token().into()),
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
        let protocol = match gateway
            .resolve_dispatch(model_record_key)
            .map_err(RuntimeError::invalid)?
            .provider()
            .protocol
        {
            Protocol::ChatCompletions => "openai-completions",
            Protocol::Responses => "openai-responses",
            Protocol::Messages => "anthropic-messages",
        };
        if let Err(error) = materialize_models(&config, &gateway, tunnel.endpoint(), protocol) {
            drop(runner);
            return Err(error);
        }
        write_selection_file(&config, model_record_key, &physical_model_id)?;
        self.session.gateway_runner = Some(runner);
        self.session.gateway_tunnel = Some(tunnel);
        self.session.active_state = ActiveState::Pi(Box::new(pi::PiSession::new(config)));
        self.session.execution_runtime_id = Some(runtime_id.into());
        self.session.model_record_key = None;
        Ok(())
    }

    /// Aliases, credentials, quota bindings and session identity belong to one
    /// prepared execution. Editing saved configuration affects the next one.
    fn start_gateway_runner(
        &self,
        gateway: &GatewayConfig,
        aliases: BTreeMap<String, String>,
    ) -> Result<Runner, RuntimeError> {
        let runner = Runner::start_with_scheduling(
            gateway.to_gateway_config()?,
            crate::authentication_resolver::ProviderResolver::capture_for_dispatch(
                gateway,
                &self.options,
                self.subscription_quota.clone(),
            ),
            aliases,
            Some(self.analytics.clone()),
            self.scheduling_state.clone(),
            self.subscription_quota.source(&gateway.id),
            self.session
                .execution_configuration
                .as_ref()
                .and_then(|configuration| configuration.gateway_session.clone()),
            self.execution_backends.clone(),
        )
        .map_err(|error| RuntimeError::context("gateway startup", error))?;
        runner.set_telemetry(self.telemetry.clone(), None, None);
        Ok(runner)
    }

    pub(super) fn start_pi_for_session(
        &mut self,
        cwd: Option<&Path>,
        session: Option<&Path>,
        model_record_key: Option<&str>,
        physical_model_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let runtime = self
            .session
            .execution_runtime_id
            .as_deref()
            .ok_or_else(|| RuntimeError::invalid("Pi 执行运行时不可用"))
            .and_then(|_| self.active_instance().cloned())?;
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
        let mut config = self.session.pi()?.config().clone();
        self.shutdown_pi()?;
        config.working_dir = cwd.clone();
        config.session = session.map(Path::to_owned);
        config.side = self.session.side.is_some();
        config.name = None;
        if let (Some(logical), Some(physical)) = (model_record_key, physical_model_id) {
            write_selection_file(&config, logical, physical)?;
        }
        let mut client = telemetry::observe(self.telemetry.as_ref(), "harness.start", || {
            pi::Client::spawn(config.clone())
        })
        .map_err(|error| runtime_start_error("startup", error))?;
        // Establish readiness through RPC before exposing bootstrap events to
        // the projection. Pi can emit initialization records before its first
        // command response; those records must not settle a new turn.
        client
            .state()
            .map_err(|error| runtime_start_error("startup", error))?;
        let runtime_id = self
            .session
            .execution_runtime_id
            .as_deref()
            .unwrap_or_default();
        let mut projection = PiProjection::new(ConversationSummary {
            native_read_state: None,
            can_rename: false,
            can_delete: false,
            id: session
                .map(|path| format!("{runtime_id}:{}", path.display()))
                .unwrap_or_else(|| "active".into()),
            title: velune_conversation::ConversationTitle::Untitled,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            runtime_id: runtime_id.into(),
            cwd: cwd.as_ref().map(|path| path.to_string_lossy().into_owned()),
            parent_conversation_id: None,
        });
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
        let cancellation_requested = self.session.cancel_requested;
        if let ActiveState::Native(session) = &self.session.active_state
            && let Some(pending) = session.pending_fork()
        {
            self.session.pending_native_fork = Some(pending);
        }
        let mut state = std::mem::replace(&mut self.session.active_state, ActiveState::Empty);
        self.session.compacting = false;
        let result = match state.live_mut() {
            Some(live) => live
                .shutdown()
                .map_err(|error| RuntimeError::context("runtime shutdown", error)),
            None => Ok(()),
        };
        self.session.gateway_runner = None;
        self.session.gateway_tunnel = None;
        self.session.execution_runtime_id = None;
        self.session.execution_configuration = None;
        self.session.configuration_changed = false;
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
mod gateway_execution_tests {
    use super::*;
    use crate::conversation::ConversationSnapshot;
    use axum::{
        Json, Router,
        http::{HeaderMap, Uri},
    };
    use std::{os::unix::fs::PermissionsExt, sync::Mutex, time::Duration};

    fn provider(id: &str, endpoint: &str) -> crate::ProviderDraft {
        crate::ProviderDraft {
            id: id.into(),
            name: id.into(),
            protocol: Protocol::Responses,
            endpoint: endpoint.into(),
            execution_backend: crate::config::ExecutionBackend::Http,
            request_profile: crate::config::RequestProfile::Native,
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

    fn save(core: &mut CoreRuntime, draft: crate::ProviderDraft, key: &str) {
        core.save_provider(
            "default".into(),
            draft,
            crate::AuthenticationEdit::SetApiKey { value: key.into() },
            None,
        )
        .unwrap();
    }

    fn snapshot(id: &str) -> ConversationSnapshot {
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
        core.session.active_state = ActiveState::History(Box::new(snapshot("pi:first")));
        core.prepare_runtime("pi", "a").unwrap();
        assert_eq!(core.backend_session_identity().unwrap().1.id, "pi:first");
        let old_gateway = core
            .session
            .execution_configuration
            .as_ref()
            .unwrap()
            .gateway
            .as_ref()
            .unwrap()
            .clone();
        let old_alias = old_gateway
            .pi_binding_id("a", old_gateway.authentication_revision("a"))
            .unwrap();
        let runner = core.session.gateway_runner.as_ref().unwrap();
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
            core.session
                .execution_configuration
                .as_ref()
                .unwrap()
                .gateway
                .as_ref(),
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
        let second_identity = core.backend_session_identity().unwrap().1;
        let runner = core.session.gateway_runner.as_ref().unwrap();
        assert_ne!(runner.endpoint(), old_endpoint);
        assert_eq!(
            send(runner.endpoint(), runner.token(), "shared-model-name")
                .await
                .status(),
            200
        );
        core.focus_new();
        core.prepare_runtime("codex", "a").unwrap();
        assert_ne!(core.backend_session_identity().unwrap().1, second_identity);
        let runner = core.session.gateway_runner.as_ref().unwrap();
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
        assert!(core.session.gateway_runner.is_none());
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
