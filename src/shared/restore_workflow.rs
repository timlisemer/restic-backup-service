use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::shared::commands::{ResticCommandExecutor, S3CommandExecutor};
use crate::shared::operations::{RepositoryOperations, RepositorySelectionItem};
use crate::shared::ui::{
    confirm_action, select_host, select_repositories, select_timestamp, HostSelection,
    RepositorySelection, TimestampSelection,
};
use crate::utils::validate_credentials;
use chrono::{DateTime, Duration, Utc};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

/// Manage the entire restore workflow
pub struct RestoreWorkflow {
    config: Config,
    host_opt: Option<String>,
    path_opt: Option<String>,
    timestamp_opt: Option<String>,
}

impl RestoreWorkflow {
    pub fn new(
        config: Config,
        host_opt: Option<String>,
        path_opt: Option<String>,
        timestamp_opt: Option<String>,
    ) -> Result<Self, BackupServiceError> {
        Ok(Self {
            config,
            host_opt,
            path_opt,
            timestamp_opt,
        })
    }

    /// Execute the complete interactive restore workflow
    pub async fn execute_interactive_restore(&self) -> Result<(), BackupServiceError> {
        self.config.set_aws_env()?;
        info!("Restic Interactive Restore Tool");

        validate_credentials(&self.config).await?;

        // Phase 1: Host selection
        let host_selection = self.execute_host_selection_phase().await?;

        // Phase 2: Backup data collection
        let backup_data = self
            .collect_backup_data(&host_selection.selected_host)
            .await?;

        // Phase 3: Repository selection
        let repository_selection = self.execute_repository_selection_phase(backup_data).await?;

        // Phase 4: Timestamp selection
        let timestamp_selection = self
            .execute_timestamp_selection_phase(&repository_selection.selected_repos)
            .await?;

        // Phase 5: Restoration
        self.execute_restoration_phase(
            &repository_selection.selected_repos,
            &timestamp_selection.selected_timestamp,
        )
        .await?;

        Ok(())
    }

    /// Phase 1: Host selection
    async fn execute_host_selection_phase(&self) -> Result<HostSelection, BackupServiceError> {
        let s3_executor = S3CommandExecutor::new(self.config.clone())?;
        let hosts = s3_executor.get_hosts().await?;

        if hosts.is_empty() {
            error!("No hosts found in backup repository");
            return Err(BackupServiceError::ConfigurationError(
                "No hosts found".to_string(),
            ));
        }

        let current_host = self.config.hostname.clone();
        let host_selection = select_host(hosts, current_host, self.host_opt.clone()).await?;

        info!(host = %host_selection.selected_host, "Selected host");
        Ok(host_selection)
    }

    /// Phase 2: Backup data collection
    async fn collect_backup_data(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositorySelectionItem>, BackupServiceError> {
        info!(host = %hostname, "Querying backups");
        let operations = RepositoryOperations::new(self.config.clone())?;

        let repo_infos = operations.scan_repositories(hostname).await?;
        info!(repo_count = %repo_infos.len(), "Converting repository data for UI");

        let repos = operations.convert_to_selection_items(repo_infos)?;

        if repos.is_empty() {
            error!(host = %hostname, "No backups found for host");
            return Err(BackupServiceError::ConfigurationError(
                "No backups found for host".to_string(),
            ));
        }

        info!(final_repo_count = %repos.len(), "Repository data converted successfully");
        Ok(repos)
    }

    /// Phase 3: Repository selection
    async fn execute_repository_selection_phase(
        &self,
        backup_data: Vec<RepositorySelectionItem>,
    ) -> Result<RepositorySelection, BackupServiceError> {
        info!(repo_count = %backup_data.len(), "Found repositories, starting selection phase");

        let repository_selection = select_repositories(backup_data, self.path_opt.clone()).await?;

        info!(repo_count = %repository_selection.selected_repos.len(), "Selected repositories for restoration");
        Ok(repository_selection)
    }

    /// Phase 4: Timestamp selection
    async fn execute_timestamp_selection_phase(
        &self,
        selected_repos: &[RepositorySelectionItem],
    ) -> Result<TimestampSelection, BackupServiceError> {
        let timestamp_selection =
            select_timestamp(selected_repos, self.timestamp_opt.clone()).await?;

        info!(timestamp = %timestamp_selection.selected_timestamp.format("%Y-%m-%d %H:%M"), "🕐 Selected time window");
        Ok(timestamp_selection)
    }

    /// Phase 5: Restoration
    async fn execute_restoration_phase(
        &self,
        selected_repos: &[RepositorySelectionItem],
        selected_timestamp: &DateTime<Utc>,
    ) -> Result<(), BackupServiceError> {
        let dest_dir = PathBuf::from("/tmp/restic/interactive");

        if dest_dir.exists() {
            if fs::read_dir(&dest_dir)?.next().is_some() {
                warn!(destination = %dest_dir.display(), "Destination directory is not empty");

                if !confirm_action("Continue and clear the directory?", false).await? {
                    error!("Operation cancelled by user");
                    return Ok(());
                }
            }
            fs::remove_dir_all(&dest_dir)?;
        }
        fs::create_dir_all(&dest_dir)?;

        info!(destination = %dest_dir.display(), "Restoring to destination");

        let (restored_count, skipped_count) = self
            .restore_repositories(selected_repos, selected_timestamp, &dest_dir)
            .await?;

        // Display detailed summary
        info!("");
        info!("Restoration Summary:");
        info!("  Successfully restored: {} repositories", restored_count);
        if skipped_count > 0 {
            info!("  Skipped: {} repositories", skipped_count);
        }
        info!("  Destination: {}", dest_dir.display());

        if restored_count > 0 {
            info!("Restoration completed successfully");
            self.handle_restored_files(selected_repos, &dest_dir)
                .await?;
        } else {
            warn!("No repositories were restored");
        }

        Ok(())
    }

    /// Restore all selected repositories
    async fn restore_repositories(
        &self,
        selected_repos: &[RepositorySelectionItem],
        selected_timestamp: &DateTime<Utc>,
        dest_dir: &Path,
    ) -> Result<(usize, usize), BackupServiceError> {
        let mut restored_count = 0;
        let mut skipped_count = 0;

        info!("Starting restoration process");

        for (idx, repo) in selected_repos.iter().enumerate() {
            info!(
                path = %repo.path.display(),
                repo_subpath = %repo.repo_subpath,
                progress = format!("({}/{})", idx + 1, selected_repos.len()),
                "Restoring repository"
            );

            let repo_url = self.config.get_repo_url(&repo.repo_subpath)?;

            let window_end = *selected_timestamp + Duration::minutes(5);
            let best_snapshot = repo
                .snapshots
                .iter()
                .filter(|s| s.time >= *selected_timestamp && s.time < window_end)
                .max_by_key(|s| s.time)
                .or_else(|| {
                    repo.snapshots
                        .iter()
                        .filter(|s| s.time < *selected_timestamp)
                        .max_by_key(|s| s.time)
                });

            if let Some(snapshot) = best_snapshot {
                info!(
                    path = %repo.path.display(),
                    snapshot_id = %snapshot.id,
                    timestamp = %snapshot.time.format("%Y-%m-%dT%H:%M:%S"),
                    "Found snapshot, starting restore"
                );

                let restic_cmd = ResticCommandExecutor::new(self.config.clone(), repo_url)?;
                let restore_output = restic_cmd
                    .restore(
                        &snapshot.id,
                        &repo.path.to_string_lossy(),
                        &dest_dir.to_string_lossy(),
                    )
                    .await?;

                // Check if the restoration was empty (like old script detection)
                let restored_path = dest_dir.join(repo.path.strip_prefix("/").unwrap_or(&repo.path));
                let is_empty = if restored_path.exists() {
                    std::fs::read_dir(&restored_path)
                        .map(|mut entries| entries.next().is_none())
                        .unwrap_or(true)
                } else {
                    true
                };

                if is_empty && restore_output.contains("0 B") {
                    info!(
                        path = %repo.path.display(),
                        snapshot_id = %snapshot.id,
                        timestamp = %snapshot.time.format("%Y-%m-%dT%H:%M:%S"),
                        "Restored (empty volume - directories only)"
                    );
                } else {
                    info!(
                        path = %repo.path.display(),
                        snapshot_id = %snapshot.id,
                        timestamp = %snapshot.time.format("%Y-%m-%dT%H:%M:%S"),
                        "Restored successfully"
                    );
                }
                restored_count += 1;
            } else {
                warn!(
                    path = %repo.path.display(),
                    "No suitable snapshots found, skipping"
                );
                skipped_count += 1;
            }
        }

        Ok((restored_count, skipped_count))
    }

    /// Handle post-restoration actions
    async fn handle_restored_files(
        &self,
        selected_repos: &[RepositorySelectionItem],
        dest_dir: &Path,
    ) -> Result<(), BackupServiceError> {
        use dialoguer::Select;

        info!(destination = %dest_dir.display(), "Restoration completed successfully! You can now access your restored files");

        info!("");
        let actions = vec![
            "Copy to original location (replace existing files)",
            "Move to original location (replace existing files)",
            "Leave files in temporary location",
        ];

        let selection = Select::new()
            .with_prompt("What would you like to do with the restored files?")
            .items(&actions)
            .default(2)
            .interact()?;

        match selection {
            0 => {
                self.copy_files_to_original_locations(selected_repos, dest_dir)
                    .await?
            }
            1 => {
                self.move_files_to_original_locations(selected_repos, dest_dir)
                    .await?
            }
            _ => {
                info!(location = %dest_dir.display(), "Files remain at temporary location");
            }
        }

        Ok(())
    }

    /// Copy restored files to original locations
    async fn copy_files_to_original_locations(
        &self,
        selected_repos: &[RepositorySelectionItem],
        dest_dir: &Path,
    ) -> Result<(), BackupServiceError> {
        info!("Copying files to original locations...");

        for repo in selected_repos {
            let src = dest_dir.join(repo.path.strip_prefix("/").unwrap_or(&repo.path));
            if src.exists() {
                // Ensure the full destination path exists, not just the parent
                if let Some(parent) = repo.path.parent() {
                    fs::create_dir_all(parent)?;
                }

                // Remove existing destination if it exists
                if repo.path.exists() {
                    if repo.path.is_dir() {
                        fs::remove_dir_all(&repo.path)?;
                    } else {
                        fs::remove_file(&repo.path)?;
                    }
                }

                // Use recursive copy function
                copy_recursively(&src, &repo.path)?;
                info!(path = %repo.path.display(), "Copied");
            }
        }

        Ok(())
    }

    /// Move restored files to original locations
    async fn move_files_to_original_locations(
        &self,
        selected_repos: &[RepositorySelectionItem],
        dest_dir: &Path,
    ) -> Result<(), BackupServiceError> {
        info!("Moving files to original locations...");

        for repo in selected_repos {
            let src = dest_dir.join(repo.path.strip_prefix("/").unwrap_or(&repo.path));
            if src.exists() {
                // Ensure the full destination path structure exists
                if let Some(parent) = repo.path.parent() {
                    fs::create_dir_all(parent)?;
                }

                // Remove existing destination if it exists
                if repo.path.exists() {
                    if repo.path.is_dir() {
                        fs::remove_dir_all(&repo.path)?;
                    } else {
                        fs::remove_file(&repo.path)?;
                    }
                }

                // Try rename first, fallback to copy+delete for cross-filesystem
                if fs::rename(&src, &repo.path).is_err() {
                    copy_recursively(&src, &repo.path)?;
                    if src.is_dir() {
                        fs::remove_dir_all(&src)?;
                    } else {
                        fs::remove_file(&src)?;
                    }
                }
                info!(path = %repo.path.display(), "Moved");
            }
        }

        fs::remove_dir_all(dest_dir).ok();
        Ok(())
    }
}

/// Recursively copy files and directories
fn copy_recursively(src: &Path, dst: &Path) -> Result<(), BackupServiceError> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            copy_recursively(&src_path, &dst_path)?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
    }
    Ok(())
}
