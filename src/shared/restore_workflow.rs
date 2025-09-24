use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::helpers::RepositoryScanner;
use crate::shared::commands::{ResticCommandExecutor, S3CommandExecutor};
use crate::shared::ui::{
    confirm_action, select_host, select_repositories, select_timestamp, HostSelection,
    RepositorySelection, RepositorySelectionItem, SnapshotItem, TimestampSelection,
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
        let hosts = self.get_available_hosts().await?;

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
        let scanner = RepositoryScanner::new(self.config.clone())?;

        let repo_infos = scanner.scan_repositories(hostname).await?;
        let mut repos = Vec::new();

        for repo_info in repo_infos {
            if let Some(snapshots) = self
                .get_repo_snapshots(hostname, &repo_info.repo_subpath)
                .await?
            {
                repos.push(RepositorySelectionItem {
                    path: repo_info.native_path,
                    repo_subpath: repo_info.repo_subpath,
                    category: repo_info.category,
                    snapshots,
                });
            }
        }

        if repos.is_empty() {
            error!(host = %hostname, "No backups found for host");
            return Err(BackupServiceError::ConfigurationError(
                "No backups found for host".to_string(),
            ));
        }

        Ok(repos)
    }

    /// Phase 3: Repository selection
    async fn execute_repository_selection_phase(
        &self,
        backup_data: Vec<RepositorySelectionItem>,
    ) -> Result<RepositorySelection, BackupServiceError> {
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

        info!(
            restored_count = %restored_count,
            skipped_count = %skipped_count,
            destination = %dest_dir.display(),
            "Restoration Summary"
        );

        if restored_count > 0 {
            self.handle_restored_files(selected_repos, &dest_dir)
                .await?;
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

        for repo in selected_repos {
            info!(
                path = %repo.path.display(),
                repo_subpath = %repo.repo_subpath,
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
                let restic_cmd = ResticCommandExecutor::new(self.config.clone(), repo_url)?;
                restic_cmd
                    .restore(
                        &snapshot.id,
                        &repo.path.to_string_lossy(),
                        &dest_dir.to_string_lossy(),
                    )
                    .await?;

                info!(
                    path = %repo.path.display(),
                    snapshot_id = %snapshot.id,
                    timestamp = %snapshot.time.format("%Y-%m-%d %H:%M:%S"),
                    "Restore completed"
                );
                restored_count += 1;
            } else {
                warn!(
                    path = %repo.path.display(),
                    "No suitable snapshots found"
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
                let parent = repo.path.parent().unwrap_or(Path::new("/"));
                fs::create_dir_all(parent)?;

                let result = std::process::Command::new("cp")
                    .args(["-rf", &src.to_string_lossy(), &parent.to_string_lossy()])
                    .output()?;

                if result.status.success() {
                    info!(path = %repo.path.display(), "Copied");
                } else {
                    error!(path = %repo.path.display(), "Failed to copy");
                }
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
                if repo.path.exists() {
                    fs::remove_dir_all(&repo.path)?;
                }
                let parent = repo.path.parent().unwrap_or(Path::new("/"));
                fs::create_dir_all(parent)?;
                fs::rename(&src, &repo.path)?;
                info!(path = %repo.path.display(), "Moved");
            }
        }

        fs::remove_dir_all(dest_dir).ok();
        Ok(())
    }

    /// Get available hosts using S3CommandExecutor
    async fn get_available_hosts(&self) -> Result<Vec<String>, BackupServiceError> {
        let s3_executor = S3CommandExecutor::new(self.config.clone())?;
        s3_executor.get_hosts().await
    }

    /// Get repository snapshots
    async fn get_repo_snapshots(
        &self,
        hostname: &str,
        repo_subpath: &str,
    ) -> Result<Option<Vec<SnapshotItem>>, BackupServiceError> {
        let repo_url = format!(
            "{}/{}/{}",
            self.config.restic_repo_base, hostname, repo_subpath
        );
        let restic_cmd = ResticCommandExecutor::new(self.config.clone(), repo_url)?;

        let snapshots = restic_cmd.snapshots(None).await?;

        let snapshot_list: Vec<SnapshotItem> = snapshots
            .into_iter()
            .filter_map(|s| {
                let time = s["time"].as_str()?.parse::<DateTime<Utc>>().ok()?;
                let id = s["short_id"].as_str()?.to_string();
                Some(SnapshotItem { id, time })
            })
            .collect();

        if !snapshot_list.is_empty() {
            Ok(Some(snapshot_list))
        } else {
            Ok(None)
        }
    }
}
