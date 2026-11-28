use super::*;
use crate::{
    AcpLaunch, RuntimeDiscoveryCandidate, RuntimeDiscoveryProbe, RuntimeHome, RuntimeTransport,
};
use velune_agent_runtime::version::{self, ANTIGRAVITY_ACP_MINIMUM, ANTIGRAVITY_FAMILY};

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
            if probe.family_id == ANTIGRAVITY_FAMILY {
                results.push(self.discover_antigravity(probe)?);
                continue;
            }
            let resolved = if probe.family_id == "pi" {
                version::resolve_pi_binary(Path::new(&probe.binary), Path::new(&probe.node_binary))
                    .map(|binary| {
                        probe.binary = binary.to_string_lossy().into_owned();
                    })
            } else {
                Ok(())
            };

            let node = (!probe.node_binary.is_empty()).then(|| Path::new(&probe.node_binary));
            let version = resolved.and_then(|()| {
                version::probe_version(&probe.family_id, Path::new(&probe.binary), node)
            });
            let has_node = node.is_some();
            let (version, adapter, detail) = match version {
                Ok(version) => {
                    let adapter = version::matching_variant(&probe.family_id, &version);
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
                    version::variants()
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
                        matches!(execution.transport, RuntimeTransport::Local)
                            && !matches!(execution.home, RuntimeHome::Managed { .. })
                    })
                    && ["binary", "agentDir"]
                        .iter()
                        .all(|key| same_setting_path(r, &runtime, key))
            });
            let node_available = has_node || kind == RuntimeKind::ClaudeCode;
            results.push(RuntimeDiscoveryCandidate {
                runtime,
                version,
                supported: adapter.is_some() && directory_valid && node_available,
                already_configured,
                detail: if adapter.is_none() {
                    detail
                } else if !directory_valid {
                    "运行时目录无效，请在手动添加中配置".into()
                } else if !node_available {
                    "未发现 Node，请安装 Node 或在手动添加中选择路径".into()
                } else {
                    detail
                },
            });
        }
        Ok(results)
    }

    fn discover_antigravity(
        &self,
        probe: RuntimeDiscoveryProbe,
    ) -> Result<RuntimeDiscoveryCandidate, RuntimeError> {
        let (version, supported, detail) =
            match version::probe_version(ANTIGRAVITY_FAMILY, Path::new(&probe.binary), None) {
                Ok(version) if version::supports_antigravity_acp(&version) => (
                    Some(version),
                    true,
                    "已识别支持 ACP 的 Antigravity CLI".into(),
                ),
                Ok(version) => (
                    Some(version),
                    false,
                    format!("请将 Antigravity CLI 升级到 {ANTIGRAVITY_ACP_MINIMUM} 或更新版本"),
                ),
                Err(error) => (None, false, format!("无法读取公开版本：{error}")),
            };
        let launch = AcpLaunch {
            args: vec!["acp".into()],
            environment: BTreeMap::new(),
        };
        let mut runtime = RuntimeInstance {
            enabled: true,
            id: crate::new_record_key(),
            name: "Antigravity".into(),
            type_id: RuntimeKind::Acp,
            gateway_id: "default".into(),
            settings: BTreeMap::from([("binary".into(), probe.binary)]),
        };
        runtime.set_acp_launch(launch.clone())?;
        let already_configured = self.runtime_instances.iter().any(|existing| {
            existing.type_id == RuntimeKind::Acp
                && existing.execution().is_ok_and(|execution| {
                    execution.transport == RuntimeTransport::Local
                        && execution.home == RuntimeHome::Inherit
                })
                && same_setting_path(existing, &runtime, "binary")
                && existing
                    .acp_launch()
                    .is_ok_and(|configured| configured == launch)
        });
        Ok(RuntimeDiscoveryCandidate {
            runtime,
            version,
            supported,
            already_configured,
            detail,
        })
    }
}

fn same_setting_path(left: &RuntimeInstance, right: &RuntimeInstance, key: &str) -> bool {
    // A directory that the Agent has not created yet compares as written.
    let resolve = |path: &String| fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    matches!(
        (left.settings.get(key).map(resolve), right.settings.get(key).map(resolve)),
        (Some(a), Some(b)) if a == b
    )
}
