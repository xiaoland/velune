use crate::{RemoteError, Result};
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_STORE_BYTES: u64 = 256 * 1024;
const MAX_DEVICES: usize = 256;

/// The secret key has no Debug/Serialize implementation in this public wrapper.
pub struct DeviceIdentity {
    pub(crate) key: SecretKey,
    pub(crate) name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityFile {
    version: u16,
    name: String,
    secret_key: [u8; 32],
}

impl DeviceIdentity {
    pub fn generate(name: String) -> Result<Self> {
        validate_name(&name)?;
        Ok(Self {
            key: SecretKey::from_bytes(&random_bytes()?),
            name,
        })
    }

    /// Only the explicit host-owned path is accessed. Existing identity files are never reset.
    pub fn load_or_create(path: &Path, name: String) -> Result<Self> {
        validate_name(&name)?;
        match read_private(path) {
            Ok(bytes) => {
                let stored: IdentityFile =
                    serde_json::from_slice(&bytes).map_err(|_| RemoteError::IdentityStorage)?;
                if stored.version != 1 || validate_name(&stored.name).is_err() {
                    return Err(RemoteError::IdentityStorage);
                }
                Ok(Self {
                    key: SecretKey::from_bytes(&stored.secret_key),
                    name: stored.name,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let identity = Self::generate(name)?;
                let stored = IdentityFile {
                    version: 1,
                    name: identity.name.clone(),
                    secret_key: identity.key.to_bytes(),
                };
                let bytes =
                    serde_json::to_vec(&stored).map_err(|_| RemoteError::IdentityStorage)?;
                create_identity_file(path, &bytes).map_err(|_| RemoteError::IdentityStorage)?;
                Ok(identity)
            }
            Err(_) => Err(RemoteError::IdentityStorage),
        }
    }

    pub fn id(&self) -> String {
        self.key.public().to_string()
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedDevice {
    pub id: String,
    pub name: String,
}

/// Host storage is updated before a pairing or revocation is reported successful.
pub trait DeviceTrust: Send + Sync {
    fn devices(&self) -> Result<Vec<TrustedDevice>>;
    fn trust(&self, device: TrustedDevice) -> Result<()>;
    fn revoke(&self, id: &str) -> Result<()>;
    fn is_trusted(&self, id: &str) -> Result<bool> {
        Ok(self.devices()?.iter().any(|device| device.id == id))
    }
}

#[derive(Default)]
pub struct MemoryTrustStore {
    devices: Mutex<BTreeMap<String, TrustedDevice>>,
}
impl MemoryTrustStore {
    pub fn new() -> Self {
        Self::default()
    }
}
impl DeviceTrust for MemoryTrustStore {
    fn devices(&self) -> Result<Vec<TrustedDevice>> {
        Ok(self
            .devices
            .lock()
            .map_err(|_| RemoteError::TrustStorage)?
            .values()
            .cloned()
            .collect())
    }
    fn trust(&self, device: TrustedDevice) -> Result<()> {
        validate_device(&device)?;
        let mut devices = self.devices.lock().map_err(|_| RemoteError::TrustStorage)?;
        if devices.len() >= MAX_DEVICES && !devices.contains_key(&device.id) {
            return Err(RemoteError::Capacity);
        }
        devices.insert(device.id.clone(), device);
        Ok(())
    }
    fn revoke(&self, id: &str) -> Result<()> {
        self.devices
            .lock()
            .map_err(|_| RemoteError::TrustStorage)?
            .remove(id);
        Ok(())
    }
}

/// Explicit application-owned file adapter; callers share one instance per path.
pub struct FileTrustStore {
    path: PathBuf,
    devices: Mutex<BTreeMap<String, TrustedDevice>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustFile {
    version: u16,
    devices: Vec<TrustedDevice>,
}
impl FileTrustStore {
    pub fn open(path: &Path) -> Result<Self> {
        let records = match read_private(path) {
            Ok(bytes) => {
                let file: TrustFile =
                    serde_json::from_slice(&bytes).map_err(|_| RemoteError::TrustStorage)?;
                if file.version != 1 || file.devices.len() > MAX_DEVICES {
                    return Err(RemoteError::TrustStorage);
                }
                file.devices
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err(RemoteError::TrustStorage),
        };
        let mut devices = BTreeMap::new();
        for device in records {
            validate_device(&device)?;
            if devices.insert(device.id.clone(), device).is_some() {
                return Err(RemoteError::TrustStorage);
            }
        }
        Ok(Self {
            path: path.to_owned(),
            devices: Mutex::new(devices),
        })
    }
    fn change(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, TrustedDevice>) -> Result<()>,
    ) -> Result<()> {
        let mut guard = self.devices.lock().map_err(|_| RemoteError::TrustStorage)?;
        let mut devices = guard.clone();
        change(&mut devices)?;
        let bytes = serde_json::to_vec(&TrustFile {
            version: 1,
            devices: devices.values().cloned().collect(),
        })
        .map_err(|_| RemoteError::TrustStorage)?;
        atomic_write(&self.path, &bytes).map_err(|_| RemoteError::TrustStorage)?;
        *guard = devices;
        Ok(())
    }
}
impl DeviceTrust for FileTrustStore {
    fn devices(&self) -> Result<Vec<TrustedDevice>> {
        Ok(self
            .devices
            .lock()
            .map_err(|_| RemoteError::TrustStorage)?
            .values()
            .cloned()
            .collect())
    }
    fn trust(&self, device: TrustedDevice) -> Result<()> {
        validate_device(&device)?;
        self.change(|devices| {
            if devices.len() >= MAX_DEVICES && !devices.contains_key(&device.id) {
                return Err(RemoteError::Capacity);
            }
            devices.insert(device.id.clone(), device);
            Ok(())
        })
    }
    fn revoke(&self, id: &str) -> Result<()> {
        self.change(|devices| {
            devices.remove(id);
            Ok(())
        })
    }
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(RemoteError::Configuration);
    }
    Ok(())
}
fn validate_device(device: &TrustedDevice) -> Result<()> {
    validate_name(&device.name)?;
    device
        .id
        .parse::<EndpointId>()
        .map_err(|_| RemoteError::TrustStorage)?;
    Ok(())
}
pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| RemoteError::Randomness)?;
    Ok(bytes)
}
pub(crate) fn random_id() -> Result<String> {
    use std::fmt::Write;
    let bytes: [u8; 16] = random_bytes()?;
    let mut id = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(id, "{byte:02x}");
    }
    Ok(id)
}
fn private_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}
fn read_private(path: &Path) -> std::io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_STORE_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_STORE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}
fn create_identity_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let suffix = random_id().map_err(|_| std::io::ErrorKind::Other)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".velune-identity-{suffix}"));
    let result = (|| {
        let mut file = private_options().create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // Linking publishes a complete file atomically and cannot replace a concurrent identity.
        fs::hard_link(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let suffix = random_id().map_err(|_| std::io::ErrorKind::Other)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".velune-trust-{suffix}"));
    let result = (|| {
        let mut file = private_options().create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
