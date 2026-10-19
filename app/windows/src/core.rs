//! This PC's execution core and the remote service opened with it.
//!
//! Every call into the bindings blocks, so views run them on GPUI's background
//! executor and apply the result on the main thread (see [`run`]).
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use gpui_kit::Context;
use velune_bindings::{
    BindingDiscoveryConfig, BindingError, BindingOptions, BindingRelayConfig, BindingRemoteOptions,
    BindingRemotePathPolicy, VeluneApplication, VeluneRemoteEndpoint,
};

/// The device a view works on. The interface is the same for each; only the
/// application it calls changes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Device {
    Local,
    Remote(String),
}

#[derive(Clone)]
pub struct Core(Arc<Inner>);

struct Inner {
    home: PathBuf,
    local: Arc<VeluneApplication>,
    endpoint: OnceLock<Arc<VeluneRemoteEndpoint>>,
    connections: Mutex<HashMap<String, Arc<VeluneApplication>>>,
    /// One view per process: a controlled device tells this app's windows apart by it.
    view: String,
}

impl Core {
    pub fn open() -> Result<Self, String> {
        let home = application_home()?;
        let resources = resources_directory()?;
        let local = VeluneApplication::open(BindingOptions {
            home_directory: home.to_string_lossy().into_owned(),
            resources_directory: resources.to_string_lossy().into_owned(),
        })
        .map_err(|error| describe(&error))?;
        Ok(Self(Arc::new(Inner {
            home,
            local,
            endpoint: OnceLock::new(),
            connections: Mutex::new(HashMap::new()),
            view: format!("windows-{}", uuid::Uuid::new_v4()),
        })))
    }

    pub fn local(&self) -> Arc<VeluneApplication> {
        self.0.local.clone()
    }

    /// Opens the remote service once; incoming control must be reachable
    /// whenever the app runs.
    pub fn open_endpoint(&self) -> Result<Arc<VeluneRemoteEndpoint>, BindingError> {
        if let Some(endpoint) = self.0.endpoint.get() {
            return Ok(endpoint.clone());
        }
        let directory = self.0.home.join("remote");
        std::fs::create_dir_all(&directory).map_err(|error| BindingError::Io {
            detail: error.to_string(),
        })?;
        let endpoint = VeluneRemoteEndpoint::open(
            BindingRemoteOptions {
                identity_file: path_string(&directory.join("device-identity")),
                state_file: path_string(&directory.join("remote-state.json")),
                device_name: device_name(),
                relay: BindingRelayConfig::DevelopmentPublic,
                discovery: BindingDiscoveryConfig::DevelopmentPublic,
                path_policy: BindingRemotePathPolicy::Automatic,
                bind_addresses: Vec::new(),
                request_timeout_ms: 30_000,
            },
            Some(self.0.local.clone()),
        )?;
        Ok(self.0.endpoint.get_or_init(|| endpoint).clone())
    }

    pub fn endpoint(&self) -> Result<Arc<VeluneRemoteEndpoint>, BindingError> {
        self.0
            .endpoint
            .get()
            .cloned()
            .ok_or(BindingError::Unavailable)
    }

    /// The application of a device: this PC's own core, or a paired device's
    /// through the same named methods.
    pub fn application(&self, device: &Device) -> Result<Arc<VeluneApplication>, BindingError> {
        let peer = match device {
            Device::Local => return Ok(self.0.local.clone()),
            Device::Remote(peer) => peer,
        };
        if let Some(application) = self.connections().get(peer) {
            return Ok(application.clone());
        }
        let application = self
            .endpoint()?
            .connect(peer.clone(), self.0.view.clone())?;
        Ok(self
            .connections()
            .entry(peer.clone())
            .or_insert(application)
            .clone())
    }

    pub fn forget_connection(&self, peer: &str) {
        self.connections().remove(peer);
    }

    fn connections(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<VeluneApplication>>> {
        self.0
            .connections
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Closes connections and the remote service; running conversations stay
    /// with their owner.
    pub fn shutdown(&self) {
        for (_, application) in self.connections().drain() {
            let _ = application.shutdown();
        }
        if let Some(endpoint) = self.0.endpoint.get() {
            let _ = endpoint.shutdown();
        }
        let _ = self.0.local.shutdown();
    }
}

/// `VELUNE_HOME`, else `%USERPROFILE%\.velune`.
fn application_home() -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("VELUNE_HOME").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(configured);
        if !path.is_absolute() {
            return Err("VELUNE_HOME 必须使用绝对目录路径；留空可使用默认目录。".into());
        }
        return Ok(path);
    }
    let profile = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or("找不到用户目录")?;
    Ok(PathBuf::from(profile).join(".velune"))
}

/// Runtime helpers ship in `resources` beside the executable.
fn resources_directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = executable
        .parent()
        .ok_or("找不到应用目录")?
        .join("resources");
    if !directory.is_dir() {
        return Err(format!("缺少运行时资源目录：{}", directory.display()));
    }
    Ok(directory)
}

pub fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Windows".into())
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A message the user can act on; details stay in the core's own diagnostics.
pub fn describe(error: &BindingError) -> String {
    match error {
        BindingError::Closed => "连接已关闭".into(),
        BindingError::Unavailable => "远程控制服务不可用".into(),
        other => other.to_string(),
    }
}

/// Runs blocking work off the main thread, then applies its result to the view.
pub fn run<T: 'static, R: Send + 'static>(
    cx: &mut Context<T>,
    work: impl FnOnce() -> R + Send + 'static,
    done: impl FnOnce(&mut T, R, &mut Context<T>) + 'static,
) {
    cx.spawn(async move |this, cx| {
        let result = cx.background_executor().spawn(async move { work() }).await;
        let _ = this.update(cx, |this, cx| {
            done(this, result, cx);
            cx.notify();
        });
    })
    .detach();
}
