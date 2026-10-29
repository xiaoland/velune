//! Native resource locations for the resource library, resolved on the execution machine.
use super::*;
use velune_agent_runtime::management;

impl CoreRuntime {
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
