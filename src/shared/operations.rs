use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::helpers::{RepositoryInfo, RepositoryScanner, SnapshotCollector, SnapshotInfo};
use crate::repository::BackupRepo;
use crate::shared::commands::S3CommandExecutor;

/// Comprehensive repository operations for data collection and management
pub struct RepositoryOperations {
    config: Config,
    scanner: RepositoryScanner,
    snapshot_collector: SnapshotCollector,
}

/// Combined repository data with snapshots
#[derive(Debug, Clone)]
pub struct RepositoryData {
    pub info: RepositoryInfo,
    pub snapshots: Vec<SnapshotInfo>,
    pub snapshot_count: usize,
}

impl RepositoryOperations {
    pub fn new(config: Config) -> Result<Self, BackupServiceError> {
        let scanner = RepositoryScanner::new(config.clone())?;
        let snapshot_collector = SnapshotCollector::new(config.clone())?;

        Ok(Self {
            config,
            scanner,
            snapshot_collector,
        })
    }

    /// Collect complete backup data for a hostname (used by restore and list operations)
    pub async fn collect_backup_data(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryData>, BackupServiceError> {
        let repo_infos = self.scanner.scan_repositories(hostname).await?;
        let mut repositories = Vec::new();

        for repo_info in repo_infos {
            let (count, snapshots) = self
                .snapshot_collector
                .get_snapshots(hostname, &repo_info.repo_subpath, &repo_info.native_path)
                .await?;

            if count > 0 {
                repositories.push(RepositoryData {
                    info: repo_info,
                    snapshots,
                    snapshot_count: count,
                });
            }
        }

        Ok(repositories)
    }

    /// Get available hosts from S3 bucket
    pub async fn get_available_hosts(&self) -> Result<Vec<String>, BackupServiceError> {
        let s3_executor = S3CommandExecutor::new(self.config.clone())?;
        s3_executor.get_hosts().await
    }

    /// Convert RepositoryData to BackupRepo format (for backward compatibility)
    pub fn convert_to_backup_repos(
        &self,
        repo_data: Vec<RepositoryData>,
    ) -> Result<Vec<BackupRepo>, BackupServiceError> {
        let mut repos = Vec::new();

        for data in repo_data {
            let repo = BackupRepo::new(data.info.native_path)?.with_count(data.snapshot_count)?;
            repos.push(repo);
        }

        Ok(repos)
    }

    /// Get all snapshots across all repositories for timeline view
    pub fn extract_all_snapshots(&self, repo_data: &[RepositoryData]) -> Vec<SnapshotInfo> {
        repo_data
            .iter()
            .flat_map(|repo| &repo.snapshots)
            .cloned()
            .collect()
    }
}
