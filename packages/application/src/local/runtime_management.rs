//! Native resource operations share execution-machine path resolution, not session preparation.
use super::*;
use velune_agent_runtime::management;

impl CoreRuntime {
    pub(super) fn runtime_management_action(&self, request: &Value) -> Result<Value, RuntimeError> {
        let action = request["action"].as_str().unwrap_or_default();
        if matches!(action, "writeRuntimeResource" | "deleteRuntimeResource")
            && (self.busy() || self.collaboration.is_some())
        {
            return Err(RuntimeError::invalid(
                "请先结束运行中的轮次并关闭协作范围，再修改原生资源",
            ));
        }
        let payload = &request["payload"];
        let runtime = self.runtime_instance(
            payload["runtimeInstanceID"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?,
        )?;
        let (execution, os_home) = self.runtime_location(runtime)?;
        let agent_dir = self.native_agent_directory(runtime, os_home.as_deref())?;
        let config = management::Config {
            variant_id: runtime.type_id.clone(),
            execution,
            agent_dir,
            cwd: serde_json::from_value::<Option<PathBuf>>(payload["cwd"].clone())?,
        };
        match action {
            "runtimeResourceCatalog" => Ok(serde_json::to_value(management::catalog(
                &config,
                serde_json::from_value(payload["kind"].clone())?,
                serde_json::from_value(payload["scope"].clone())?,
            )?)?),
            "readRuntimeResource" => Ok(serde_json::to_value(management::read(
                &config,
                &serde_json::from_value(payload["key"].clone())?,
            )?)?),
            "writeRuntimeResource" => Ok(serde_json::to_value(management::write(
                &config,
                &serde_json::from_value(payload["key"].clone())?,
                &serde_json::from_value(payload["expected"].clone())?,
                &serde_json::from_value::<Vec<u8>>(payload["bytes"].clone())?,
                serde_json::from_value(payload["executable"].clone())?,
            )?)?),
            "deleteRuntimeResource" => {
                management::delete(
                    &config,
                    &serde_json::from_value(payload["key"].clone())?,
                    &serde_json::from_value(payload["expected"].clone())?,
                )?;
                Ok(Value::Null)
            }
            _ => Err(RuntimeError::invalid("原生资源操作无效")),
        }
    }
}
