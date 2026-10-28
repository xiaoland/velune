//! Native resource operations share execution-machine path resolution, not session preparation.
use super::*;
use velune_agent_runtime::management;

impl CoreRuntime {
    /// List native source resources without launching the Harness or creating resource roots.
    /// Project scope requires a cwd on the execution machine. Disabled instances remain manageable.
    pub(crate) fn runtime_resource_catalog(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        kind: management::ResourceKind,
        scope: management::ResourceScope,
    ) -> Result<management::Catalog, RuntimeError> {
        Ok(management::catalog(
            &self.resource_location(&runtime_id, cwd)?,
            kind,
            scope,
        )?)
    }

    /// Read one native file or MCP server entry. The returned bytes may contain private settings.
    pub(crate) fn read_runtime_resource(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        key: management::ResourceKey,
    ) -> Result<management::Document, RuntimeError> {
        Ok(management::read(
            &self.resource_location(&runtime_id, cwd)?,
            &key,
        )?)
    }

    /// Save one native resource against its last observed revision. This does not reload or close
    /// an active session; when the Harness notices the change is owned by that Harness.
    pub(crate) fn write_runtime_resource(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        key: management::ResourceKey,
        expected: management::ExpectedRevision,
        bytes: Vec<u8>,
        executable: Option<bool>,
    ) -> Result<management::Document, RuntimeError> {
        self.require_no_execution_for_resources()?;
        Ok(management::write(
            &self.resource_location(&runtime_id, cwd)?,
            &key,
            &expected,
            &bytes,
            executable,
        )?)
    }

    /// Delete one native resource, or an empty directory. Never removes a complete skill tree.
    pub(crate) fn delete_runtime_resource(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        key: management::ResourceKey,
        expected: management::ExpectedRevision,
    ) -> Result<(), RuntimeError> {
        self.require_no_execution_for_resources()?;
        Ok(management::delete(
            &self.resource_location(&runtime_id, cwd)?,
            &key,
            &expected,
        )?)
    }

    fn require_no_execution_for_resources(&self) -> Result<(), RuntimeError> {
        if self.execution_active() {
            return Err(RuntimeError::invalid(
                "请先结束运行中的轮次并关闭协作范围，再修改原生资源",
            ));
        }
        Ok(())
    }

    fn resource_location(
        &self,
        runtime_id: &str,
        cwd: Option<String>,
    ) -> Result<management::Config, RuntimeError> {
        let mut config = self.management_config(runtime_id)?;
        config.cwd = cwd.map(PathBuf::from);
        Ok(config)
    }

    /// Where a runtime's native resources live, on the machine it runs on.
    pub(super) fn management_config(
        &self,
        runtime_id: &str,
    ) -> Result<management::Config, RuntimeError> {
        let runtime = self.runtime_instance(runtime_id)?;
        let (execution, os_home) = self.runtime_location(runtime)?;
        let agent_dir = self.native_agent_directory(runtime, os_home.as_deref())?;
        Ok(management::Config {
            runtime: runtime.type_id,
            execution,
            agent_dir,
            cwd: None,
        })
    }
}
