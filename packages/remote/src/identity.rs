use crate::{RemoteError, Result};
use iroh::SecretKey;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const MAX_STORE_BYTES: u64 = 1024 * 1024;

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

/// Validate these three host-owned files before creating an identity or resetting ordinary state.
/// Resolving existing parents prevents aliases such as a symlinked configuration directory.
pub fn validate_device_storage_paths(identity: &Path, state: &Path) -> Result<()> {
    fn target(path: &Path) -> Result<std::path::PathBuf> {
        if !path.is_absolute() {
            return Err(RemoteError::Configuration);
        }
        if let Ok(metadata) = fs::symlink_metadata(path)
            && metadata.file_type().is_symlink()
        {
            return Err(RemoteError::Configuration);
        }
        match path.canonicalize() {
            Ok(path) => Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = path
                    .parent()
                    .ok_or(RemoteError::Configuration)?
                    .canonicalize()
                    .map_err(|_| RemoteError::Configuration)?;
                Ok(parent.join(path.file_name().ok_or(RemoteError::Configuration)?))
            }
            Err(_) => Err(RemoteError::Configuration),
        }
    }
    let paths = [
        target(identity)?,
        target(state)?,
        target(&crate::settings::private_settings_path(state)?)?,
    ];
    for (index, path) in paths.iter().enumerate() {
        for other in &paths[index + 1..] {
            if path == other
                || (path.parent() == other.parent()
                    && path
                        .file_name()
                        .zip(other.file_name())
                        .is_some_and(|(one, two)| {
                            one.to_string_lossy()
                                .eq_ignore_ascii_case(&two.to_string_lossy())
                        }))
            {
                return Err(RemoteError::Configuration);
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(RemoteError::Configuration);
    }
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
pub(crate) fn read_private(path: &Path) -> std::io::Result<Vec<u8>> {
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
        // Android sandboxes deny hard links; NOREPLACE keeps publication atomic without replacement.
        #[cfg(target_os = "android")]
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            &temporary,
            rustix::fs::CWD,
            path,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
        #[cfg(not(target_os = "android"))]
        fs::hard_link(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let suffix = random_id().map_err(|_| std::io::ErrorKind::Other)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".velune-state-{suffix}"));
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
