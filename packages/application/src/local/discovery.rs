use super::*;
use crate::{RuntimeDiscoveryCandidate, RuntimeDiscoveryProbe};
impl CoreRuntime {
    pub(crate) fn discover_runtimes(
        &self,
        probes: Vec<RuntimeDiscoveryProbe>,
    ) -> Result<Vec<RuntimeDiscoveryCandidate>, RuntimeError> {
        if probes.len() > 32 {
            return Err(RuntimeError::Invalid("运行时发现候选过多".into()));
        }
        let mut results = Vec::new();
        for mut probe in probes {
            let resolved = if probe.family_id == "pi" {
                velune_agent_runtime::version::resolve_pi_binary(
                    Path::new(&probe.binary),
                    Path::new(&probe.node_binary),
                )
                .map(|binary| {
                    probe.binary = binary.to_string_lossy().into_owned();
                })
            } else {
                Ok(())
            };

            let version = resolved.and_then(|()| {
                velune_agent_runtime::version::probe_version(
                    &probe.family_id,
                    Path::new(&probe.binary),
                    Some(Path::new(&probe.node_binary)),
                )
            });
            let (version, adapter, detail) = match version {
                Ok(version) => {
                    let adapter =
                        velune_agent_runtime::version::matching_variant(&probe.family_id, &version);
                    let detail = if adapter.is_some() {
                        "已识别支持的运行时版本"
                    } else {
                        "此版本尚无适配器，不能导入"
                    };
                    (Some(version), adapter, detail.to_owned())
                }
                Err(error) => (None, None, format!("无法读取公开版本：{error}")),
            };
            // The runtime's own directory: a runtime installed but not yet run
            // has not created it, and creates it on its first run.
            let directory_valid = Path::new(&probe.agent_directory).is_absolute()
                && !probe.agent_directory.contains('\0');
            // A version without an adapter is shown as its family's, unsupported.
            let kind = adapter
                .or_else(|| {
                    velune_agent_runtime::version::variants()
                        .iter()
                        .find(|variant| variant.family_id == probe.family_id)
                })
                .map(|variant| variant.kind)
                .ok_or_else(|| RuntimeError::invalid("未知的运行时类别"))?;
            let runtime = RuntimeInstance {
                enabled: true,
                id: crate::new_record_key(),
                name: adapter.map_or(probe.family_id.as_str(), |v| v.name).into(),
                type_id: kind,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    ("binary".into(), probe.binary),
                    ("nodeBinary".into(), probe.node_binary),
                    ("agentDir".into(), probe.agent_directory),
                ]),
            };
            let already_configured = self.runtime_instances.iter().any(|r| {
                r.type_id == runtime.type_id
                    && r.execution().is_ok_and(|execution| {
                        matches!(execution.transport, crate::RuntimeTransport::Local)
                            && !matches!(execution.home, crate::RuntimeHome::Managed { .. })
                    })
                    && ["binary", "agentDir"].iter().all(|key| {
                        // A directory not created yet compares as written.
                        let resolve =
                            |p: &String| fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
                        let existing = r.settings.get(*key).map(resolve);
                        let candidate = runtime.settings.get(*key).map(resolve);
                        matches!((existing, candidate), (Some(a), Some(b)) if a == b)
                    })
            });
            results.push(RuntimeDiscoveryCandidate {
                runtime,
                version,
                supported: adapter.is_some() && directory_valid,
                already_configured,
                detail: if directory_valid || adapter.is_none() {
                    detail
                } else {
                    "运行时目录无效，请在手动添加中配置".into()
                },
            });
        }
        Ok(results)
    }
}
