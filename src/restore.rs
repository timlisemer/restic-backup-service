use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::shared::restore_workflow::RestoreWorkflow;

// CLI command for interactive restore with optional pre-filled parameters
pub async fn restore_interactive(
    config: Config,
    host_opt: Option<String>,
    path_opt: Option<String>,
    timestamp_opt: Option<String>,
) -> Result<(), BackupServiceError> {
    let workflow = RestoreWorkflow::new(config, host_opt, path_opt, timestamp_opt)?;
    workflow.execute_interactive_restore().await
}
