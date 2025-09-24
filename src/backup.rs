use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::shared::backup_workflow::execute_backup_workflow;

/// Main entry point for backup operations - now uses the modular BackupWorkflow
pub async fn run_backup(
    config: Config,
    additional_paths: Vec<String>,
) -> Result<(), BackupServiceError> {
    execute_backup_workflow(config, additional_paths).await
}
