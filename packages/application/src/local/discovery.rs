use super::*;
use crate::{RuntimeDiscoveryCandidate, RuntimeDiscoveryProbe};
impl CoreRuntime {
    pub(super) fn discover_runtimes(&self, request: &Value) -> Result<Value, RuntimeError> {
        let probes: Vec<RuntimeDiscoveryProbe> =
            serde_json::from_value(request["payload"]["probes"].clone())?;
        if probes.len() > 32 {
            return Err(RuntimeError::Invalid("运行时发现候选过多".into()));
        }
        let mut results = Vec::new();
        for probe in probes {
            let version = velune_agent_runtime::version::probe_version(
                &probe.family_id,
                Path::new(&probe.binary),
                Some(Path::new(&probe.node_binary)),
            );
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
                Err(_) => (
                    None,
                    None,
                    "无法读取公开版本，请检查可执行文件与 Node 配置".into(),
                ),
            };
            let directory_valid = Path::new(&probe.agent_directory).is_absolute()
                && !probe.agent_directory.contains('\0')
                && Path::new(&probe.agent_directory).is_dir();
            let runtime = RuntimeInstance {
                enabled: true,
                id: crate::new_record_key(),
                name: adapter.map_or(probe.family_id.as_str(), |v| v.name).into(),
                type_id: adapter.map_or("", |v| v.id).into(),
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    ("binary".into(), probe.binary),
                    ("nodeBinary".into(), probe.node_binary),
                    ("agentDir".into(), probe.agent_directory),
                ]),
            };
            let already_configured = self.runtime_instances.iter().any(|r| {
                r.type_id == runtime.type_id
                    && ["binary", "agentDir"].iter().all(|key| {
                        let existing = r.settings.get(*key).and_then(|p| fs::canonicalize(p).ok());
                        let candidate = runtime
                            .settings
                            .get(*key)
                            .and_then(|p| fs::canonicalize(p).ok());
                        matches!((existing, candidate), (Some(a), Some(b)) if a == b)
                    })
            });
            results.push(RuntimeDiscoveryCandidate {
                runtime,
                version,
                supported: adapter.is_some() && directory_valid,
                already_configured,
                detail: if directory_valid {
                    detail
                } else {
                    "运行时目录不存在，请在手动添加中配置".into()
                },
            });
        }
        Ok(serde_json::to_value(results)?)
    }
}
