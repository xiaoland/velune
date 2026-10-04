//! Local same-user prototype IPC. No TCP/LAN listener, no credentials, no launch agent.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use velune_core::Core;
use velune_core::gateway::{
    GatewayConfig, ModelDefinition, RuntimeInstance, RuntimeTypeDescriptor,
};
use velune_core::harness::{
    Connection, ConversationSummary, PiProjection, SettingAction, SettingField,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_IPC_BYTES: u64 = 256 * 1024;
const GENERIC_IPC_VERSION: u64 = 3;
const HOST_APP_VERSION: &str = "0.1 beta.1";
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn request_value(dir: &Path, value: &Value) -> Result<String> {
    let mut stream = UnixStream::connect(dir.join("ipc.sock"))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    writeln!(stream, "{value}")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    if line.is_empty() {
        return Err("host closed without response".into());
    }
    Ok(line)
}
pub fn serve(dir: &Path) -> Result<()> {
    if dir.exists() && fs::symlink_metadata(dir)?.file_type().is_symlink() {
        return Err("host directory must not be a symlink".into());
    }
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    // Acquire DB ownership before removing a stale socket. A second Host cannot unlink a live one.
    let mut core = Core::open(dir.join("core.sqlite"))?;
    let mut pi: Option<velune_core::pi::Client> = None;
    let mut pi_config: Option<velune_core::pi::Config> = None;
    let mut pi_busy = false;
    let mut pi_projection: Option<PiProjection> = None;
    let persisted = load_generic_config(&dir.join("generic-config.json"))?;
    let mut gateways = persisted.gateways;
    let mut runtime_instances = persisted.runtime_instances;
    let mut gateway_runner: Option<velune_core::gateway_runtime::Runner> = None;
    let mut active_runtime_id: Option<String> = None;
    let mut pi_records = Vec::new();
    let mut pi_record_seq = 1_u64;
    let socket = dir.join("ipc.sock");
    if socket.exists() {
        fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let mut last = Instant::now();
    let mut last_error: Option<String> = None;
    let generic_config_path = dir.join("generic-config.json");
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // Some Unix implementations reject socket timeouts on an
                // accepted local stream. The bounded line read below still
                // protects the host; a timeout failure must not kill it.
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                let mut line = String::new();
                // Bound payloads while allowing real prompt/config requests.
                let read = std::io::Read::take(&mut stream, MAX_IPC_BYTES);
                if BufReader::new(read).read_line(&mut line).is_err() {
                    continue;
                }
                let value: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                let command = value["command"].as_str().unwrap_or("");
                let mut stop = false;
                let mut data = Value::Null;
                let mut requires_reconnect = false;
                let generic = value["version"] == GENERIC_IPC_VERSION;
                let previous_gateways = generic.then(|| gateways.clone());
                let previous_runtime_instances = generic.then(|| runtime_instances.clone());
                let mut outcome: velune_core::Result<()> = (|| {
                    if generic {
                        match value["action"].as_str().unwrap_or("") {
                            "handshake" => {
                                data = json!({
                                    "ipcVersion": GENERIC_IPC_VERSION,
                                    "hostPID": std::process::id(),
                                    "appVersion": HOST_APP_VERSION,
                                    "running": true,
                                });
                                return Ok(());
                            }
                            "retireIfIdle" => {
                                ensure_generic_idle(pi_busy)?;
                                if let Some(mut client) = pi.take() {
                                    client.shutdown().map_err(|_| {
                                        velune_core::Error::Invalid("runtime shutdown")
                                    })?;
                                }
                                pi_config = None;
                                pi_projection = None;
                                gateway_runner = None;
                                active_runtime_id = None;
                                stop = true;
                                data = json!({"retired": true});
                                return Ok(());
                            }
                            _ => {}
                        }
                        let mut state = GenericState {
                            pi: &mut pi,
                            pi_config: &mut pi_config,
                            pi_busy: &mut pi_busy,
                            projection: &mut pi_projection,
                            gateways: &mut gateways,
                            runtime_instances: &mut runtime_instances,
                            gateway_runner: &mut gateway_runner,
                            active_runtime_id: &mut active_runtime_id,
                        };
                        return handle_generic_action(&value, &mut data, &mut state);
                    }
                    if value["version"] != 1 {
                        return Err(velune_core::Error::Unsupported("IPC version"));
                    }
                    match command {
                        "run" => {
                            last = Instant::now();
                            core.enqueue_fixture(now()).map(|_| ())
                        }
                        "status" => Ok(()),
                        "cancel" => core.cancel_pending(),
                        "stop" => {
                            stop = true;
                            if let Some(mut client) = pi.take() {
                                client
                                    .shutdown()
                                    .map_err(|_| velune_core::Error::Invalid("Pi shutdown"))?;
                            }
                            pi_config = None;
                            pi_busy = false;
                            pi_projection = None;
                            gateway_runner = None;
                            active_runtime_id = None;
                            core.cancel_pending()
                        }
                        "pi_start" => {
                            if pi.is_some() {
                                return Err(velune_core::Error::Invalid(
                                    "Pi runner already started",
                                ));
                            }
                            let config: velune_core::pi::Config =
                                serde_json::from_value(value["config"].clone())
                                    .map_err(|_| velune_core::Error::Invalid("Pi configuration"))?;
                            materialize_models(&config, None).map_err(|_| {
                                velune_core::Error::Invalid("Pi models configuration")
                            })?;
                            let client = velune_core::pi::Client::spawn(config.clone())
                                .map_err(|_| velune_core::Error::Invalid("Pi RPC startup"))?;
                            pi = Some(client);
                            pi_config = Some(config);
                            pi_busy = false;
                            pi_projection = Some(PiProjection::new(ConversationSummary {
                                id: "active".into(),
                                title: "当前会话".into(),
                                updated_at: None,
                                runtime_id: "native-runtime".into(),
                            }));
                            Ok(())
                        }
                        "pi_sessions" => {
                            let config = pi_config
                                .as_ref()
                                .ok_or(velune_core::Error::Invalid("Pi is not configured"))?;
                            data = list_pi_sessions(config, value["all"] == true)
                                .map_err(|_| velune_core::Error::Invalid("Pi session list"))?;
                            Ok(())
                        }
                        "pi_stop" => {
                            if let Some(mut client) = pi.take() {
                                client
                                    .shutdown()
                                    .map_err(|_| velune_core::Error::Invalid("Pi shutdown"))?;
                            }
                            pi_config = None;
                            pi_busy = false;
                            pi_projection = None;
                            Ok(())
                        }
                        "pi_prompt" => {
                            if pi_busy {
                                return Err(velune_core::Error::Invalid("Pi is busy"));
                            }
                            let message = value["message"]
                                .as_str()
                                .ok_or(velune_core::Error::Invalid("Pi prompt"))?;
                            let client = pi
                                .as_mut()
                                .ok_or(velune_core::Error::Invalid("Pi is not started"))?;
                            data = client
                                .prompt(message)
                                .map_err(|_| velune_core::Error::Invalid("Pi prompt"))?;
                            pi_busy = data["data"]["disposition"] != "handled";
                            Ok(())
                        }
                        "pi_cancel" => {
                            let client = pi
                                .as_mut()
                                .ok_or(velune_core::Error::Invalid("Pi is not started"))?;
                            data = serde_json::to_value(
                                client
                                    .cancel()
                                    .map_err(|_| velune_core::Error::Invalid("Pi cancel"))?,
                            )
                            .unwrap_or(Value::Null);
                            Ok(())
                        }
                        "pi_state" => {
                            let client = pi
                                .as_mut()
                                .ok_or(velune_core::Error::Invalid("Pi is not started"))?;
                            let (state, messages) = client
                                .state()
                                .map_err(|_| velune_core::Error::Invalid("Pi state"))?;
                            data = json!({"state":state,"messages":messages});
                            Ok(())
                        }
                        "pi_new" => {
                            if pi_busy {
                                return Err(velune_core::Error::Invalid("Pi is busy"));
                            }
                            let client = pi
                                .as_mut()
                                .ok_or(velune_core::Error::Invalid("Pi is not started"))?;
                            let switched = client
                                .request(json!({"type":"new_session"}))
                                .map_err(|_| velune_core::Error::Invalid("Pi new session"))?;
                            if switched["data"]["cancelled"] != true {
                                let (state, messages) = client
                                    .state()
                                    .map_err(|_| velune_core::Error::Invalid("Pi state"))?;
                                data = json!({"switch":switched,"state":state,"messages":messages});
                            } else {
                                data = json!({"switch":switched});
                            }
                            Ok(())
                        }
                        "pi_entries" => {
                            let client = pi
                                .as_mut()
                                .ok_or(velune_core::Error::Invalid("Pi is not started"))?;
                            data = client
                                .request(json!({"type":"get_entries"}))
                                .map_err(|_| velune_core::Error::Invalid("Pi entries"))?;
                            Ok(())
                        }
                        "pi_tree" => {
                            let client = pi
                                .as_mut()
                                .ok_or(velune_core::Error::Invalid("Pi is not started"))?;
                            data = client
                                .request(json!({"type":"get_tree"}))
                                .map_err(|_| velune_core::Error::Invalid("Pi tree"))?;
                            Ok(())
                        }
                        "pi_switch" => {
                            if pi_busy {
                                return Err(velune_core::Error::Invalid("Pi is busy"));
                            }
                            let path = value["session"]
                                .as_str()
                                .ok_or(velune_core::Error::Invalid("Pi session path"))?;
                            let path = std::path::Path::new(path);
                            if !path.is_absolute() {
                                return Err(velune_core::Error::Invalid(
                                    "Pi session path must be absolute",
                                ));
                            }
                            let client = pi
                                .as_mut()
                                .ok_or(velune_core::Error::Invalid("Pi is not started"))?;
                            let switched = client
                                .switch_session(path)
                                .map_err(|_| velune_core::Error::Invalid("Pi switch"))?;
                            if switched["data"]["cancelled"] != true {
                                let (state, messages) = client
                                    .state()
                                    .map_err(|_| velune_core::Error::Invalid("Pi state"))?;
                                data = json!({"switch":switched,"state":state,"messages":messages});
                            } else {
                                data = json!({"switch":switched});
                            }
                            Ok(())
                        }
                        _ => Err(velune_core::Error::Unsupported("IPC command")),
                    }
                })();
                let config_changed = generic
                    && outcome.is_ok()
                    && (previous_gateways
                        .as_ref()
                        .is_some_and(|previous| previous != &gateways)
                        || previous_runtime_instances
                            .as_ref()
                            .is_some_and(|previous| previous != &runtime_instances));
                let runner_config_changed = config_changed && value["action"] != "selectModel";
                if runner_config_changed
                    && outcome.is_ok()
                    && let Err(error) = disconnect_for_config_change(
                        &mut pi,
                        &mut pi_config,
                        &mut pi_busy,
                        &mut pi_projection,
                        &mut gateway_runner,
                        &mut active_runtime_id,
                        &mut requires_reconnect,
                    )
                {
                    outcome = Err(error);
                }
                if config_changed
                    && outcome.is_ok()
                    && let Err(error) =
                        save_generic_config(&generic_config_path, &gateways, &runtime_instances)
                {
                    if let Some(previous) = previous_gateways {
                        gateways = previous;
                    }
                    if let Some(previous) = previous_runtime_instances {
                        runtime_instances = previous;
                    }
                    last_error = Some(error.to_string());
                    outcome = Err(velune_core::Error::Invalid("configuration persistence"));
                }
                if let Some(client) = pi.as_mut() {
                    if collect_pi_records(
                        client,
                        &mut pi_records,
                        &mut pi_record_seq,
                        pi_projection.as_mut(),
                    ) {
                        pi_busy = false;
                    }
                    if pi_records.len() > 256 {
                        let keep_from = pi_records.len() - 256;
                        pi_records.drain(..keep_from);
                    }
                }
                let response = match outcome {
                    Ok(()) => {
                        if generic {
                            if value["action"] != "handshake"
                                && let Some(object) = data.as_object_mut()
                            {
                                object
                                    .insert("requiresReconnect".into(), json!(requires_reconnect));
                            }
                            json!({"version":GENERIC_IPC_VERSION,"ok":true,"data":data})
                        } else {
                            json!({"ok":true,"diagnostics":core.diagnostics()?,"host_error":last_error,"data":data,"pi":pi_snapshot(pi_config.as_ref(), &pi_records)})
                        }
                    }
                    Err(error) if generic => {
                        json!({"version":GENERIC_IPC_VERSION,"ok":false,"error":error.to_string()})
                    }
                    Err(error) => json!({"ok":false,"error":error.to_string()}),
                };
                // A disconnected UI must not terminate the independent Host.
                let _ = writeln!(stream, "{response}");
                if stop {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        if last.elapsed() >= Duration::from_millis(900) {
            if let Err(error) = core.tick(now()) {
                last_error = Some(error.to_string());
            }
            last = Instant::now();
        }
        if let Some(client) = pi.as_mut() {
            if collect_pi_records(
                client,
                &mut pi_records,
                &mut pi_record_seq,
                pi_projection.as_mut(),
            ) {
                pi_busy = false;
            }
            if pi_records.len() > 256 {
                let keep_from = pi_records.len() - 256;
                pi_records.drain(..keep_from);
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    fs::remove_file(socket)?;
    Ok(())
}

struct GenericState<'a> {
    pi: &'a mut Option<velune_core::pi::Client>,
    pi_config: &'a mut Option<velune_core::pi::Config>,
    pi_busy: &'a mut bool,
    projection: &'a mut Option<PiProjection>,
    gateways: &'a mut Vec<GatewayConfig>,
    runtime_instances: &'a mut Vec<RuntimeInstance>,
    gateway_runner: &'a mut Option<velune_core::gateway_runtime::Runner>,
    active_runtime_id: &'a mut Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedConfig {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    gateways: Vec<GatewayConfig>,
    #[serde(default)]
    runtime_instances: Vec<RuntimeInstance>,
}

fn load_generic_config(path: &Path) -> Result<PersistedConfig> {
    match fs::read(path) {
        Ok(bytes) => {
            let config: PersistedConfig = serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid generic configuration {}: {error}", path.display())
            })?;
            if config.schema_version != 2 {
                return Err(format!(
                    "unsupported generic configuration schema {} in {}; save a new configuration",
                    config.schema_version,
                    path.display()
                )
                .into());
            }
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PersistedConfig {
            schema_version: 2,
            gateways: Vec::new(),
            runtime_instances: Vec::new(),
        }),
        Err(error) => Err(format!(
            "cannot read generic configuration {}: {error}",
            path.display()
        )
        .into()),
    }
}

fn save_generic_config(
    path: &Path,
    gateways: &[GatewayConfig],
    runtime_instances: &[RuntimeInstance],
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&PersistedConfig {
        schema_version: 2,
        gateways: gateways.to_vec(),
        runtime_instances: runtime_instances.to_vec(),
    })
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn handle_generic_action(
    value: &Value,
    data: &mut Value,
    state: &mut GenericState<'_>,
) -> velune_core::Result<()> {
    let GenericState {
        pi,
        pi_config,
        pi_busy,
        projection,
        gateways,
        runtime_instances,
        gateway_runner,
        active_runtime_id,
    } = state;
    let action = value["action"].as_str().unwrap_or("");
    match action {
        "list" => {
            let conversations = if let Some(config) = pi_config.as_ref() {
                list_pi_sessions(config, false)
                    .map(|value| generic_conversations(&value, active_runtime_id.as_deref()))
                    .map_err(|_| velune_core::Error::Invalid("conversation list"))?
            } else {
                Vec::new()
            };
            *data = json!({
                "conversations": conversations,
                "connections": generic_connections(pi_config.as_ref(), active_runtime_id.as_deref()),
                "models": gateway_models(gateways),
                "gateways": gateways,
                "runtimeInstances": runtime_instances,
                "runtimeTypes": generic_runtime_types(),
                "protocols": [{"id":"chatCompletionsV1","name":"OpenAI Chat Completions v1","supported":true}],
                "activeRuntimeInstanceID": active_runtime_id,
                "runtimeInstanceID": active_runtime_id,
            });
            Ok(())
        }
        "gateways" => {
            match value["payload"]["operation"].as_str().unwrap_or("list") {
                "upsert" => {
                    ensure_generic_idle(**pi_busy)?;
                    let raw = value["payload"]["gateway"]
                        .as_str()
                        .ok_or(velune_core::Error::Invalid("gateway"))?;
                    let gateway: GatewayConfig = serde_json::from_str(raw)
                        .map_err(|_| velune_core::Error::Invalid("gateway"))?;
                    gateway.validate().map_err(velune_core::Error::Invalid)?;
                    gateways.retain(|item| item.id != gateway.id);
                    gateways.push(gateway);
                }
                "delete" => {
                    ensure_generic_idle(**pi_busy)?;
                    let id = value["payload"]["gatewayID"]
                        .as_str()
                        .ok_or(velune_core::Error::Invalid("gateway id"))?;
                    if runtime_instances
                        .iter()
                        .any(|runtime| runtime.gateway_id == id)
                    {
                        return Err(velune_core::Error::Invalid("gateway is used by a runtime"));
                    }
                    gateways.retain(|item| item.id != id);
                }
                "list" => {}
                _ => return Err(velune_core::Error::Invalid("gateway operation")),
            }
            *data = json!({"models":gateway_models(gateways),"gateways":gateways});
            Ok(())
        }
        "runtimeInstances" => {
            match value["payload"]["operation"].as_str().unwrap_or("list") {
                "upsert" => {
                    ensure_generic_idle(**pi_busy)?;
                    let raw = value["payload"]["runtimeInstance"]
                        .as_str()
                        .ok_or(velune_core::Error::Invalid("runtime instance"))?;
                    let runtime: RuntimeInstance = serde_json::from_str(raw)
                        .map_err(|_| velune_core::Error::Invalid("runtime instance"))?;
                    if runtime.id.is_empty()
                        || runtime.name.is_empty()
                        || runtime.type_id != "pi"
                        || !gateways
                            .iter()
                            .any(|gateway| gateway.id == runtime.gateway_id)
                    {
                        return Err(velune_core::Error::Invalid("runtime instance"));
                    }
                    runtime_instances.retain(|item| item.id != runtime.id);
                    runtime_instances.push(runtime);
                }
                "delete" => {
                    ensure_generic_idle(**pi_busy)?;
                    let id = value["payload"]["runtimeInstanceID"]
                        .as_str()
                        .ok_or(velune_core::Error::Invalid("runtime instance id"))?;
                    runtime_instances.retain(|item| item.id != id);
                }
                "list" => {}
                _ => return Err(velune_core::Error::Invalid("runtime operation")),
            }
            *data = json!({
                "runtimeInstances":runtime_instances,
                "runtimeTypes":generic_runtime_types(),
            });
            Ok(())
        }
        "runtimeAction" => {
            let runtime_id = value["payload"]["runtimeInstanceID"]
                .as_str()
                .ok_or(velune_core::Error::Invalid("runtime instance id"))?;
            if value["payload"]["actionID"].as_str() != Some("connect") {
                return Err(velune_core::Error::Unsupported("runtime action"));
            }
            ensure_generic_idle(**pi_busy)?;
            connect_runtime_instance(
                pi,
                pi_config,
                pi_busy,
                projection,
                gateway_runner,
                active_runtime_id,
                runtime_instances,
                gateways,
                runtime_id,
            )?;
            *data = json!({
                "runtimeInstanceID": active_runtime_id,
                "connections": generic_connections(pi_config.as_ref(), active_runtime_id.as_deref()),
            });
            Ok(())
        }
        "getSnapshot" => {
            ensure_active_runtime(value, active_runtime_id.as_deref())?;
            sync_generic(pi, projection, active_runtime_id.as_deref())?;
            *data =
                json!({"snapshot": projection.as_ref().and_then(|value| value.snapshot.as_ref())});
            Ok(())
        }
        "create" => {
            ensure_active_runtime(value, active_runtime_id.as_deref())?;
            ensure_generic_idle(**pi_busy)?;
            let client = pi
                .as_mut()
                .ok_or(velune_core::Error::Invalid("runtime is not connected"))?;
            client
                .request(json!({"type":"new_session"}))
                .map_err(|_| velune_core::Error::Invalid("conversation create"))?;
            sync_generic(pi, projection, active_runtime_id.as_deref())?;
            *data =
                json!({"snapshot": projection.as_ref().and_then(|value| value.snapshot.as_ref())});
            Ok(())
        }
        "open" => {
            ensure_active_runtime(value, active_runtime_id.as_deref())?;
            ensure_generic_idle(**pi_busy)?;
            let id = value["payload"]["conversationID"]
                .as_str()
                .ok_or(velune_core::Error::Invalid("conversation id"))?;
            let runtime_id = active_runtime_id
                .as_deref()
                .ok_or(velune_core::Error::Invalid("runtime is not connected"))?;
            let path = decode_conversation_id(runtime_id, id)?;
            let client = pi
                .as_mut()
                .ok_or(velune_core::Error::Invalid("runtime is not connected"))?;
            client
                .switch_session(path)
                .map_err(|_| velune_core::Error::Invalid("conversation open"))?;
            if let Some(value) = projection.as_mut() {
                value.set_conversation(ConversationSummary {
                    id: id.into(),
                    title: path
                        .file_stem()
                        .and_then(|item| item.to_str())
                        .unwrap_or("会话")
                        .into(),
                    updated_at: None,
                    runtime_id: runtime_id.into(),
                });
            }
            sync_generic(pi, projection, active_runtime_id.as_deref())?;
            *data =
                json!({"snapshot": projection.as_ref().and_then(|value| value.snapshot.as_ref())});
            Ok(())
        }
        "send" => {
            ensure_active_runtime(value, active_runtime_id.as_deref())?;
            if **pi_busy {
                return Err(velune_core::Error::Invalid("runtime is busy"));
            }
            let text = value["payload"]["text"]
                .as_str()
                .filter(|text| !text.trim().is_empty())
                .ok_or(velune_core::Error::Invalid("message text"))?;
            let client = pi
                .as_mut()
                .ok_or(velune_core::Error::Invalid("runtime is not connected"))?;
            let response = client
                .prompt(text)
                .map_err(|_| velune_core::Error::Invalid("message send"))?;
            if let Some(value) = projection.as_mut() {
                value.append_user(text);
            }
            **pi_busy = response["data"]["disposition"] != "handled";
            *data =
                json!({"snapshot": projection.as_ref().and_then(|value| value.snapshot.as_ref())});
            Ok(())
        }
        "cancel" => {
            ensure_active_runtime(value, active_runtime_id.as_deref())?;
            let client = pi
                .as_mut()
                .ok_or(velune_core::Error::Invalid("runtime is not connected"))?;
            client
                .cancel()
                .map_err(|_| velune_core::Error::Invalid("message cancel"))?;
            *data =
                json!({"snapshot": projection.as_ref().and_then(|value| value.snapshot.as_ref())});
            Ok(())
        }
        "settings" => Err(velune_core::Error::Unsupported(
            "runtime settings belong to runtimeInstances",
        )),
        "resources" => Err(velune_core::Error::Unsupported(
            "gateway providers and models belong to gateways",
        )),
        "selectModel" => {
            ensure_generic_idle(**pi_busy)?;
            if let Some(runtime_id) = value["payload"]["runtimeInstanceID"].as_str() {
                let model_id = value["payload"]["modelID"]
                    .as_str()
                    .ok_or(velune_core::Error::Invalid("model id"))?;
                let runtime_index = runtime_instances
                    .iter()
                    .position(|item| item.id == runtime_id)
                    .ok_or(velune_core::Error::Invalid("runtime instance id"))?;
                let runtime = runtime_instances
                    .get(runtime_index)
                    .ok_or(velune_core::Error::Invalid("runtime instance id"))?;
                let gateway = gateways
                    .iter()
                    .find(|item| item.id == runtime.gateway_id)
                    .ok_or(velune_core::Error::Invalid("gateway id"))?;
                gateway
                    .validate_dispatch(model_id)
                    .map_err(velune_core::Error::Invalid)?;
                if active_runtime_id.as_deref() == Some(runtime_id) {
                    let config = pi_config
                        .as_ref()
                        .ok_or(velune_core::Error::Invalid("runtime is not connected"))?;
                    let mut config = config.clone();
                    config.model = Some(model_id.into());
                    let model = gateway
                        .model(model_id)
                        .ok_or(velune_core::Error::Invalid("model id"))?;
                    materialize_models(&config, Some(model))
                        .map_err(|_| velune_core::Error::Invalid("models configuration"))?;
                    let client = pi
                        .as_mut()
                        .ok_or(velune_core::Error::Invalid("runtime is not connected"))?;
                    client
                        .request(json!({"type":"set_model","provider":"velune-gateway","modelId":model_id}))
                        .map_err(|_| velune_core::Error::Invalid("model selection"))?;
                    **pi_config = Some(config);
                    runtime_instances[runtime_index].model_id = Some(model_id.into());
                    if let Some(value) = projection.as_mut()
                        && let Some(snapshot) = value.snapshot.as_mut()
                    {
                        snapshot.model_id = Some(model_id.into());
                        snapshot.revision = snapshot.revision.saturating_add(1);
                    }
                } else {
                    runtime_instances[runtime_index].model_id = Some(model_id.into());
                }
                *data = json!({"snapshot":projection.as_ref().and_then(|value| value.snapshot.as_ref())});
                return Ok(());
            }
            Err(velune_core::Error::Invalid(
                "runtime instance id is required",
            ))
        }
        "connect" => {
            if let Some(runtime_id) = value["payload"]["runtimeInstanceID"].as_str() {
                ensure_generic_idle(**pi_busy)?;
                connect_runtime_instance(
                    pi,
                    pi_config,
                    pi_busy,
                    projection,
                    gateway_runner,
                    active_runtime_id,
                    runtime_instances,
                    gateways,
                    runtime_id,
                )?;
            } else {
                return Err(velune_core::Error::Unsupported(
                    "gateway runtime instance is required",
                ));
            }
            *data = json!({
                "connections": generic_connections(pi_config.as_ref(), active_runtime_id.as_deref()),
                "runtimeInstanceID": active_runtime_id,
            });
            Ok(())
        }
        "beginAuthorization" => Err(velune_core::Error::Unsupported(
            "provider authorization is owned by the gateway platform",
        )),
        _ => Err(velune_core::Error::Unsupported("generic IPC action")),
    }
}

fn ensure_generic_idle(busy: bool) -> velune_core::Result<()> {
    if busy {
        Err(velune_core::Error::Invalid("runtime is busy"))
    } else {
        Ok(())
    }
}

fn ensure_active_runtime(
    value: &Value,
    active_runtime_id: Option<&str>,
) -> velune_core::Result<()> {
    let requested = value["payload"]["runtimeInstanceID"]
        .as_str()
        .ok_or(velune_core::Error::Invalid("runtime instance id"))?;
    if active_runtime_id != Some(requested) {
        return Err(velune_core::Error::Invalid(
            "runtime instance is not active",
        ));
    }
    Ok(())
}

fn disconnect_for_config_change(
    pi: &mut Option<velune_core::pi::Client>,
    pi_config: &mut Option<velune_core::pi::Config>,
    pi_busy: &mut bool,
    projection: &mut Option<PiProjection>,
    gateway_runner: &mut Option<velune_core::gateway_runtime::Runner>,
    active_runtime_id: &mut Option<String>,
    requires_reconnect: &mut bool,
) -> velune_core::Result<()> {
    if let Some(mut client) = pi.take() {
        client
            .shutdown()
            .map_err(|_| velune_core::Error::Invalid("runtime shutdown"))?;
    }
    *pi_config = None;
    *pi_busy = false;
    *projection = None;
    *gateway_runner = None;
    *active_runtime_id = None;
    *requires_reconnect = true;
    Ok(())
}

fn start_generic_with_config(
    pi: &mut Option<velune_core::pi::Client>,
    pi_config: &mut Option<velune_core::pi::Config>,
    pi_busy: &mut bool,
    projection: &mut Option<PiProjection>,
    config: velune_core::pi::Config,
    runtime_id: Option<&str>,
    model_definition: Option<&ModelDefinition>,
) -> velune_core::Result<()> {
    if pi.is_some() {
        return Err(velune_core::Error::Invalid("runtime already connected"));
    }
    materialize_models(&config, model_definition)
        .map_err(|_| velune_core::Error::Invalid("runtime configuration"))?;
    *pi = Some(
        velune_core::pi::Client::spawn(config.clone())
            .map_err(|_| velune_core::Error::Invalid("runtime startup"))?,
    );
    *pi_config = Some(config);
    let mut value = PiProjection::new(ConversationSummary {
        id: "active".into(),
        title: "当前会话".into(),
        updated_at: None,
        runtime_id: runtime_id.unwrap_or("native-runtime").into(),
    });
    if let Some(snapshot) = value.snapshot.as_mut() {
        snapshot.model_id = pi_config.as_ref().and_then(|config| config.model.clone());
    }
    *projection = Some(value);
    *pi_busy = false;
    Ok(())
}

fn config_from_runtime(
    runtime: &RuntimeInstance,
    gateway: &velune_core::gateway_runtime::Runner,
    bundled_sdk_helper: Option<&Path>,
) -> velune_core::Result<velune_core::pi::Config> {
    let path = |key: &str| {
        runtime
            .settings
            .get(key)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
    };
    let binary = path("binary").ok_or(velune_core::Error::Invalid("runtime executable"))?;
    let agent_dir = path("agentDir").ok_or(velune_core::Error::Invalid("runtime directory"))?;
    let working_dir = path("workingDir").ok_or(velune_core::Error::Invalid("runtime workdir"))?;
    let model = runtime
        .model_id
        .clone()
        .ok_or(velune_core::Error::Invalid("runtime model id"))?;
    Ok(velune_core::pi::Config {
        binary,
        node_binary: path("nodeBinary"),
        sdk_helper: path("sdkHelper").or_else(|| bundled_sdk_helper.map(Path::to_path_buf)),
        extension: None,
        agent_dir: Some(agent_dir),
        working_dir: Some(working_dir),
        provider: Some("velune-gateway".into()),
        model: Some(model),
        protocol: Some("openai-completions".into()),
        endpoint: Some(gateway.endpoint().into()),
        credential_ref: None,
        credential_resolver: None,
        gateway_token: Some(gateway.token().into()),
        session_dir: path("sessionDir"),
        session: None,
        name: Some(runtime.name.clone()),
    })
}

#[allow(clippy::too_many_arguments)]
fn connect_runtime_instance(
    pi: &mut Option<velune_core::pi::Client>,
    pi_config: &mut Option<velune_core::pi::Config>,
    pi_busy: &mut bool,
    projection: &mut Option<PiProjection>,
    gateway_runner: &mut Option<velune_core::gateway_runtime::Runner>,
    active_runtime_id: &mut Option<String>,
    runtime_instances: &[RuntimeInstance],
    gateways: &[GatewayConfig],
    runtime_id: &str,
) -> velune_core::Result<()> {
    let runtime = runtime_instances
        .iter()
        .find(|item| item.id == runtime_id)
        .ok_or(velune_core::Error::Invalid("runtime instance id"))?;
    let gateway = gateways
        .iter()
        .find(|item| item.id == runtime.gateway_id)
        .ok_or(velune_core::Error::Invalid("gateway id"))?;
    let model_id = runtime
        .model_id
        .as_deref()
        .ok_or(velune_core::Error::Invalid("runtime model id"))?;
    gateway
        .validate_dispatch(model_id)
        .map_err(velune_core::Error::Invalid)?;
    if let Some(mut client) = pi.take() {
        client
            .shutdown()
            .map_err(|_| velune_core::Error::Invalid("runtime shutdown"))?;
    }
    *pi_config = None;
    *pi_busy = false;
    *projection = None;
    *gateway_runner = None;
    *active_runtime_id = None;
    let (bundled_sdk_helper, bundled_credential_resolver) = bundled_helpers();
    let runner =
        velune_core::gateway_runtime::Runner::start(gateway.clone(), bundled_credential_resolver)
            .map_err(|_| velune_core::Error::Invalid("gateway startup"))?;
    let config = match config_from_runtime(runtime, &runner, bundled_sdk_helper.as_deref()) {
        Ok(config) => config,
        Err(error) => {
            drop(runner);
            return Err(error);
        }
    };
    let model_definition = gateway
        .model(model_id)
        .ok_or(velune_core::Error::Invalid("runtime model id"))?;
    if let Err(error) = start_generic_with_config(
        pi,
        pi_config,
        pi_busy,
        projection,
        config,
        Some(runtime_id),
        Some(model_definition),
    ) {
        drop(runner);
        return Err(error);
    }
    *gateway_runner = Some(runner);
    *active_runtime_id = Some(runtime_id.into());
    Ok(())
}

fn sync_generic(
    pi: &mut Option<velune_core::pi::Client>,
    projection: &mut Option<PiProjection>,
    runtime_id: Option<&str>,
) -> velune_core::Result<()> {
    let client = pi
        .as_mut()
        .ok_or(velune_core::Error::Invalid("runtime is not connected"))?;
    let (state, messages) = client
        .state()
        .map_err(|_| velune_core::Error::Invalid("conversation snapshot"))?;
    if let Some(projection) = projection.as_mut() {
        if let Some(path) = state["data"]["sessionFile"].as_str() {
            let title = std::path::Path::new(path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("会话")
                .to_owned();
            let runtime_id = runtime_id.unwrap_or("native-runtime");
            projection.set_conversation(ConversationSummary {
                id: conversation_id(runtime_id, path),
                title,
                updated_at: None,
                runtime_id: runtime_id.into(),
            });
        }
        projection.replace_history(&messages);
    }
    Ok(())
}

fn generic_conversations(value: &Value, runtime_id: Option<&str>) -> Vec<ConversationSummary> {
    let runtime_id = runtime_id.unwrap_or("native-runtime");
    value["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|session| {
            let path = session["path"].as_str()?.to_owned();
            Some(ConversationSummary {
                title: session["name"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .or_else(|| session["firstMessage"].as_str())
                    .unwrap_or("未命名会话")
                    .chars()
                    .take(80)
                    .collect(),
                updated_at: session["modified"].as_str().map(str::to_owned),
                runtime_id: runtime_id.into(),
                id: conversation_id(runtime_id, &path),
            })
        })
        .collect()
}

fn conversation_id(runtime_id: &str, path: &str) -> String {
    format!("{runtime_id}:{path}")
}

fn decode_conversation_id<'a>(runtime_id: &str, id: &'a str) -> velune_core::Result<&'a Path> {
    let prefix = format!("{runtime_id}:");
    let path = id
        .strip_prefix(&prefix)
        .ok_or(velune_core::Error::Invalid("conversation runtime"))?;
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(velune_core::Error::Invalid(
            "conversation id must be absolute",
        ));
    }
    Ok(path)
}

fn generic_connections(
    config: Option<&velune_core::pi::Config>,
    runtime_id: Option<&str>,
) -> Vec<Connection> {
    if config.is_some() {
        vec![Connection {
            id: runtime_id.unwrap_or("native-runtime").into(),
            name: "Agent runtime".into(),
            state: "ready".into(),
            capabilities: vec!["conversation".into(), "streaming".into(), "cancel".into()],
        }]
    } else {
        Vec::new()
    }
}

fn generic_runtime_types() -> Vec<RuntimeTypeDescriptor> {
    vec![RuntimeTypeDescriptor {
        id: "pi".into(),
        name: "Pi Agent 运行时".into(),
        fields: [
            ("binary", "运行时入口", "filePath", true),
            ("nodeBinary", "Node 可执行文件", "filePath", false),
            ("workingDir", "工作目录", "directoryPath", true),
            ("agentDir", "运行时目录", "directoryPath", true),
            ("sessionDir", "会话目录", "directoryPath", false),
        ]
        .into_iter()
        .map(|(key, label, kind, required)| SettingField {
            key: key.into(),
            label: label.into(),
            kind: kind.into(),
            required,
            value: String::new(),
            options: Vec::new(),
            help: match key {
                "binary" => Some("Pi CLI 的绝对路径；应用不会使用全局 PATH。".into()),
                "nodeBinary" => Some("Node 22.19+ 的绝对路径；用于启动 JavaScript CLI。".into()),
                "workingDir" => Some("该运行时使用的工作目录。".into()),
                "agentDir" => Some("该运行时专属的 Pi 配置目录。".into()),
                "sessionDir" => Some("可选的 Pi 会话目录。".into()),
                _ => None,
            },
        })
        .collect(),
        actions: vec![SettingAction {
            id: "connect".into(),
            label: "连接运行时".into(),
        }],
    }]
}

fn bundled_helpers() -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let Some(executable) = std::env::current_exe().ok() else {
        return (None, None);
    };
    let Some(helpers) = executable.parent() else {
        return (None, None);
    };
    let credential = helpers.join("velune-credential");
    let resources = helpers
        .parent()
        .map(|bundle| bundle.join("Resources").join("pi_sessions.mjs"));
    (
        resources.filter(|path| path.is_file()),
        credential.is_file().then_some(credential),
    )
}

fn gateway_models(gateways: &[GatewayConfig]) -> Vec<ModelDefinition> {
    let mut models = BTreeMap::new();
    for gateway in gateways {
        for model in &gateway.models {
            models
                .entry(model.id.clone())
                .or_insert_with(|| model.clone());
        }
    }
    models.into_values().collect()
}

fn pi_snapshot(config: Option<&velune_core::pi::Config>, records: &[Value]) -> Value {
    json!({
        "running": config.is_some(),
        "config": config,
        "records": records,
        "simulation": false,
    })
}

fn collect_pi_records(
    client: &mut velune_core::pi::Client,
    records: &mut Vec<Value>,
    next_seq: &mut u64,
    mut projection: Option<&mut PiProjection>,
) -> bool {
    let mut settled = false;
    for mut record in client.poll() {
        if let Some(value) = projection.as_deref_mut() {
            value.apply_event(&record);
        }
        settled |= record["type"] == "agent_settled";
        if let Some(object) = record.as_object_mut() {
            object.insert("velune_seq".into(), json!(*next_seq));
        }
        *next_seq += 1;
        records.push(record);
    }
    settled
}

fn materialize_models(
    config: &velune_core::pi::Config,
    model_definition: Option<&ModelDefinition>,
) -> std::result::Result<(), std::io::Error> {
    let Some(agent_dir) = &config.agent_dir else {
        return Ok(());
    };
    let (Some(provider), Some(model), Some(endpoint), Some(protocol)) = (
        config.provider.as_deref(),
        config.model.as_deref(),
        config.endpoint.as_deref(),
        config.protocol.as_deref(),
    ) else {
        return Ok(());
    };
    fs::create_dir_all(agent_dir)?;
    let marker = agent_dir.join(".velune-managed");
    let target = agent_dir.join("models.json");
    if target.exists() && !marker.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "refusing to replace an unmanaged Pi models.json",
        ));
    }
    let mut model_config_entry = json!({
        "id": model,
        "input": ["text"],
    });
    if let Some(model) = model_definition {
        model_config_entry["name"] = Value::String(model.nickname.clone());
        model_config_entry["maxTokens"] = json!(model.max_output_tokens);
        if !model.reasoning_levels.is_empty() {
            model_config_entry["reasoning"] = Value::Bool(true);
            model_config_entry["compat"] = json!({"supportsReasoningEffort": true});
            let supported_levels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
            let thinking_level_map = supported_levels
                .into_iter()
                .map(|level| {
                    let value = if model
                        .reasoning_levels
                        .iter()
                        .any(|configured| configured == level)
                    {
                        Value::String(level.into())
                    } else {
                        Value::Null
                    };
                    (level.to_owned(), value)
                })
                .collect::<serde_json::Map<_, _>>();
            model_config_entry["thinkingLevelMap"] = Value::Object(thinking_level_map);
        }
    }
    let provider_config = json!({
        "baseUrl": endpoint,
        "api": protocol,
        "models": [model_config_entry]
    });
    let mut model_config = json!({"providers": {}});
    model_config["providers"][provider] = provider_config;
    if config.gateway_token.is_some() {
        model_config["providers"][provider]["apiKey"] =
            Value::String("$VELUNE_GATEWAY_TOKEN".into());
    } else if let (Some(resolver), Some(reference)) =
        (&config.credential_resolver, &config.credential_ref)
    {
        let command = format!("!{} {}", shell_quote(resolver), shell_quote_str(reference));
        model_config["providers"][provider]["apiKey"] = Value::String(command);
    }
    let bytes = serde_json::to_vec_pretty(&model_config)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let temporary = agent_dir.join("models.json.velune.tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, target)?;
    fs::write(marker, b"velune managed models config v1\n")?;
    fs::set_permissions(
        agent_dir.join("models.json"),
        fs::Permissions::from_mode(0o600),
    )?;
    fs::set_permissions(
        agent_dir.join(".velune-managed"),
        fs::Permissions::from_mode(0o600),
    )?;
    Ok(())
}

fn shell_quote(value: &Path) -> String {
    shell_quote_str(&value.to_string_lossy())
}

fn shell_quote_str(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn list_pi_sessions(
    config: &velune_core::pi::Config,
    all: bool,
) -> std::result::Result<Value, std::io::Error> {
    let helper = config
        .sdk_helper
        .as_ref()
        .ok_or_else(|| std::io::Error::other("Pi SDK session helper is not configured"))?;
    let mut command = Command::new(config.node_binary.as_deref().unwrap_or(Path::new("node")));
    command.arg(helper);
    if all {
        command.arg("--all");
    }
    if let Some(cwd) = &config.working_dir {
        command.arg("--cwd").arg(cwd);
    }
    if let Some(session_dir) = &config.session_dir {
        command.arg("--session-dir").arg(session_dir);
    }
    if let Some(agent_dir) = &config.agent_dir {
        command.env("PI_CODING_AGENT_DIR", agent_dir);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("Pi SDK session helper failed"));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| std::io::Error::other(error.to_string()))
}
