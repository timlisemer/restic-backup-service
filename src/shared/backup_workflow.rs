use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::shared::commands::ResticCommandExecutor;
use crate::shared::paths::{PathMapper, PathUtilities};
use crate::shared::ui::create_backup_progress_bar;
use crate::utils::validate_credentials;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

/// Overall backup summary
#[derive(Debug)]
struct BackupSummary {
    success_count: usize,
    skip_count: usize,
}

/// Manages the complete backup workflow
pub struct BackupWorkflow {
    config: Config,
    additional_paths: Vec<String>,
}

impl BackupWorkflow {
    pub fn new(config: Config, additional_paths: Vec<String>) -> Result<Self, BackupServiceError> {
        Ok(Self {
            config,
            additional_paths,
        })
    }

    /// Execute the complete backup workflow
    pub async fn execute_backup(&self) -> Result<(), BackupServiceError> {
        let hostname = &self.config.hostname.clone();
        info!(hostname = %hostname, "Starting backup process");

        self.config.set_aws_env()?;
        validate_credentials(&self.config).await?;

        // Phase 1: Prepare backup paths
        let all_paths = self.prepare_backup_paths().await?;

        if all_paths.is_empty() {
            warn!("No paths configured for backup. Use BACKUP_PATHS in .env or specify paths via command line.");
            return Ok(());
        }

        // Phase 2: Execute backups with progress tracking
        let backup_summary = self.execute_backup_operations(&all_paths, hostname).await?;

        // Phase 3: Report results
        self.report_backup_results(&backup_summary).await?;

        Ok(())
    }

    /// Phase 1: Prepare all paths to backup
    async fn prepare_backup_paths(&self) -> Result<Vec<PathBuf>, BackupServiceError> {
        let mut all_paths: Vec<PathBuf> = self.config.backup_paths.clone();

        // Add additional paths from command line
        for path in &self.additional_paths {
            all_paths.push(PathBuf::from(path));
        }

        // Discover and add docker volumes
        let docker_volumes = PathUtilities::discover_docker_volumes()?;
        all_paths.extend(docker_volumes);

        // Validate and filter paths
        let valid_paths = PathUtilities::validate_and_filter_paths(all_paths)?;

        Ok(valid_paths)
    }

    /// Phase 2: Execute backup operations with progress tracking
    async fn execute_backup_operations(
        &self,
        all_paths: &[PathBuf],
        hostname: &str,
    ) -> Result<BackupSummary, BackupServiceError> {
        let pb = create_backup_progress_bar(all_paths.len())?;
        let mut success_count = 0;
        let mut skip_count = 0;

        for (idx, path) in all_paths.iter().enumerate() {
            pb.set_position(idx as u64);
            pb.set_message(format!("Backing up: {}", path.display()));

            let success = self.execute_single_backup(path, hostname).await?;
            if success {
                success_count += 1;
            } else {
                skip_count += 1;
            }
        }

        pb.finish_and_clear();

        Ok(BackupSummary {
            success_count,
            skip_count,
        })
    }

    /// Execute backup for a single path
    async fn execute_single_backup(
        &self,
        path: &Path,
        hostname: &str,
    ) -> Result<bool, BackupServiceError> {
        // Validate path exists (redundant check for safety)
        if !path.exists() {
            warn!(path = %path.display(), "Path does not exist, skipping");
            return Ok(false);
        }

        let repo_subpath = PathMapper::path_to_repo_subpath(path)?;
        let repo_url = self.config.get_repo_url(&repo_subpath)?;
        let restic_cmd = ResticCommandExecutor::new(self.config.clone(), repo_url)?;

        // Initialize repository if needed
        restic_cmd.init_if_needed().await?;

        // Run backup
        let output = restic_cmd.backup(path, hostname).await?;

        // Parse backup output
        if output.contains("snapshot") && output.contains("saved") {
            let snapshot_id = self.extract_snapshot_id(&output);
            let has_warnings = output.contains("at least one source file could not be read");

            if has_warnings {
                warn!(
                    path = %path.display(),
                    snapshot_id = %snapshot_id.as_deref().unwrap_or("unknown"),
                    "Backed up with some files skipped due to I/O errors"
                );
            } else {
                info!(
                    path = %path.display(),
                    snapshot_id = %snapshot_id.as_deref().unwrap_or("unknown"),
                    "Backup completed"
                );
            }
            Ok(true)
        } else {
            warn!(path = %path.display(), "Failed to backup");
            Ok(false)
        }
    }

    /// Phase 3: Report backup results
    async fn report_backup_results(
        &self,
        summary: &BackupSummary,
    ) -> Result<(), BackupServiceError> {
        if summary.success_count == 0 && summary.skip_count > 0 {
            error!(
                success_count = %summary.success_count,
                skip_count = %summary.skip_count,
                "BACKUP FAILED: No data was backed up! Please check the errors above"
            );
        } else if summary.skip_count > 0 {
            warn!(
                success_count = %summary.success_count,
                skip_count = %summary.skip_count,
                "Backup partially completed"
            );
        } else {
            info!(
                success_count = %summary.success_count,
                "Backup completed successfully"
            );
        }

        Ok(())
    }

    /// Extract snapshot ID from backup output
    fn extract_snapshot_id(&self, output: &str) -> Option<String> {
        output
            .lines()
            .find(|line| line.contains("snapshot") && line.contains("saved"))
            .and_then(|line| line.split_whitespace().nth(1))
            .map(|s| s.to_string())
    }
}

/// Simplified public interface that maintains API compatibility
pub async fn execute_backup_workflow(
    config: Config,
    additional_paths: Vec<String>,
) -> Result<(), BackupServiceError> {
    let workflow = BackupWorkflow::new(config, additional_paths)?;
    workflow.execute_backup().await
}
