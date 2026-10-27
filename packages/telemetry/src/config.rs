use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, path::Path};

use crate::{Error, Result};

pub const MAX_PROFILE_BYTES: usize = 16 * 1024;
const MAX_FILE_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preference {
    #[default]
    Unset,
    Off,
    On,
}

/// Connection material contains credentials. Never include it in diagnostics or ordinary snapshots.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Profile {
    pub endpoint: String,
    pub headers: BTreeMap<String, String>,
    pub sample_ratio: f64,
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        let url = url::Url::parse(&self.endpoint).map_err(|_| Error::InvalidEndpoint)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
            || self.endpoint.len() > 2048
        {
            return Err(Error::InvalidEndpoint);
        }
        if !self.sample_ratio.is_finite() || !(0.0..=1.0).contains(&self.sample_ratio) {
            return Err(Error::InvalidSampling);
        }
        if self.headers.len() > 16 {
            return Err(Error::InvalidHeaders);
        }
        for (key, value) in &self.headers {
            if key.is_empty()
                || key.len() > 64
                || key.starts_with("grpc-")
                || key.ends_with("-bin")
                || !key
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
                || matches!(
                    key.as_str(),
                    "content-type"
                        | "content-length"
                        | "content-encoding"
                        | "connection"
                        | "proxy-connection"
                        | "keep-alive"
                        | "transfer-encoding"
                        | "upgrade"
                        | "te"
                        | "host"
                        | "user-agent"
                )
                || value.len() > 4096
                || value.bytes().any(|c| !(32..=126).contains(&c))
            {
                return Err(Error::InvalidHeaders);
            }
        }
        if serde_json::to_vec(self)?.len() > MAX_PROFILE_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(())
    }

    pub fn is_loopback(&self) -> bool {
        url::Url::parse(&self.endpoint).ok().is_some_and(|url| {
            url.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Stored {
    pub version: u16,
    pub revision: u64,
    pub installation_id: String,
    pub preference: Preference,
    pub profile: Option<Profile>,
}

impl Stored {
    pub fn empty() -> Result<Self> {
        Ok(Self {
            version: 1,
            revision: 0,
            installation_id: random_id()?,
            preference: Preference::Unset,
            profile: None,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.installation_id.len() != 32
            || !self.installation_id.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(Error::StorageFormat);
        }
        if let Some(profile) = &self.profile {
            profile.validate()?;
        }
        if self.preference == Preference::On && self.profile.is_none() {
            return Err(Error::MissingProfile);
        }
        Ok(())
    }
}

pub(crate) fn random_id() -> Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::Random)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub(crate) fn read<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T> {
    use std::io::Read;
    #[cfg(windows)]
    crate::windows_private::protect(path, false)?;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or(Error::StorageFormat)?;
    private_directory(parent)?;
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge);
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(windows)]
    crate::windows_private::protect(file.path(), false)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    // Rename is the commit boundary. A later directory fsync failure cannot mean "not saved".
    #[cfg(unix)]
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

pub(crate) fn private_directory(parent: &Path) -> Result<()> {
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    crate::windows_private::protect(parent, true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> Profile {
        Profile {
            endpoint: "http://127.0.0.1:4317".into(),
            headers: BTreeMap::new(),
            sample_ratio: 1.0,
        }
    }
    #[test]
    fn rejects_credential_urls_paths_and_non_grpc_metadata() {
        let mut p = profile();
        assert!(p.validate().is_ok());
        for endpoint in [
            "file:///tmp/a",
            "http://u:p@localhost",
            "http://localhost/v1/traces",
            "http://localhost?token=secret",
        ] {
            p.endpoint = endpoint.into();
            assert!(p.validate().is_err());
        }
        p = profile();
        p.headers
            .insert("authorization".into(), "Bearer synthetic".into());
        assert!(p.validate().is_ok());
        p.headers.insert("x-token".into(), "a\nb".into());
        assert!(p.validate().is_err());
        p = profile();
        p.sample_ratio = f64::NAN;
        assert!(p.validate().is_err());
    }
    #[test]
    fn private_atomic_file_replaces_whole_profile() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("telemetry/config.json");
        let mut value = Stored::empty().unwrap();
        write(&path, &value).unwrap();
        value.profile = Some(profile());
        value.revision = 1;
        write(&path, &value).unwrap();
        assert_eq!(read::<Stored>(&path).unwrap().revision, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn largest_legal_profile_round_trips_inside_the_storage_envelope() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut p = profile();
        p.headers = (0..4)
            .map(|i| (format!("h{i}"), "x".repeat(4060)))
            .collect();
        p.validate().unwrap();
        let mut stored = Stored::empty().unwrap();
        stored.profile = Some(p);
        stored.preference = Preference::On;
        assert!(serde_json::to_vec(&stored).unwrap().len() > MAX_PROFILE_BYTES);
        write(&path, &stored).unwrap();
        read::<Stored>(&path).unwrap().validate().unwrap();
    }
}
