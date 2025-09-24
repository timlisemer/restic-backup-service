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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::path::PathBuf;

    fn create_test_snapshot(time_str: &str, path: &str, id: &str) -> SnapshotInfo {
        let time = DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .with_timezone(&Utc);
        SnapshotInfo {
            time,
            path: PathBuf::from(path),
            id: id.to_string(),
        }
    }

    fn create_test_repo_info(native_path: &str, repo_subpath: &str, category: &str) -> RepositoryInfo {
        RepositoryInfo {
            native_path: PathBuf::from(native_path),
            repo_subpath: repo_subpath.to_string(),
            category: category.to_string(),
        }
    }

    fn create_test_repo_data(
        native_path: &str,
        repo_subpath: &str,
        category: &str,
        snapshots: Vec<SnapshotInfo>,
    ) -> RepositoryData {
        let snapshot_count = snapshots.len();
        RepositoryData {
            info: create_test_repo_info(native_path, repo_subpath, category),
            snapshots,
            snapshot_count,
        }
    }

    #[test]
    fn test_convert_to_backup_repos_basic() -> Result<(), BackupServiceError> {
        // Create a basic config for RepositoryOperations
        use crate::config::Config;
        use std::path::PathBuf;

        let config = Config {
            restic_password: "test".to_string(),
            restic_repo_base: "s3:https://test.com/bucket".to_string(),
            aws_access_key_id: "test".to_string(),
            aws_secret_access_key: "test".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://test.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        };

        let ops = RepositoryOperations::new(config)?;

        // Create test repository data
        let repo_data = vec![
            create_test_repo_data(
                "/home/tim/documents",
                "user_home/tim/documents",
                "user_home",
                vec![
                    create_test_snapshot("2025-01-15T10:30:00Z", "/home/tim/documents", "snap1"),
                    create_test_snapshot("2025-01-15T11:00:00Z", "/home/tim/documents", "snap2"),
                ],
            ),
            create_test_repo_data(
                "/mnt/docker-data/volumes/postgres",
                "docker_volume/postgres",
                "docker_volume",
                vec![
                    create_test_snapshot("2025-01-15T09:30:00Z", "/mnt/docker-data/volumes/postgres", "snap3"),
                ],
            ),
            create_test_repo_data(
                "/etc/nginx",
                "system/etc_nginx",
                "system",
                vec![], // No snapshots
            ),
        ];

        // Convert to BackupRepo format
        let backup_repos = ops.convert_to_backup_repos(repo_data)?;

        // Verify conversion
        assert_eq!(backup_repos.len(), 3);

        // Check first repo
        assert_eq!(backup_repos[0].native_path, PathBuf::from("/home/tim/documents"));
        assert_eq!(backup_repos[0].snapshot_count, 2);
        assert_eq!(backup_repos[0].category()?, "user_home");

        // Check second repo
        assert_eq!(backup_repos[1].native_path, PathBuf::from("/mnt/docker-data/volumes/postgres"));
        assert_eq!(backup_repos[1].snapshot_count, 1);
        assert_eq!(backup_repos[1].category()?, "docker_volume");

        // Check third repo (no snapshots)
        assert_eq!(backup_repos[2].native_path, PathBuf::from("/etc/nginx"));
        assert_eq!(backup_repos[2].snapshot_count, 0);
        assert_eq!(backup_repos[2].category()?, "system");

        Ok(())
    }

    #[test]
    fn test_convert_to_backup_repos_empty() -> Result<(), BackupServiceError> {
        use crate::config::Config;

        let config = Config {
            restic_password: "test".to_string(),
            restic_repo_base: "s3:https://test.com/bucket".to_string(),
            aws_access_key_id: "test".to_string(),
            aws_secret_access_key: "test".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://test.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        };

        let ops = RepositoryOperations::new(config)?;
        let empty_data: Vec<RepositoryData> = vec![];
        let backup_repos = ops.convert_to_backup_repos(empty_data)?;

        assert!(backup_repos.is_empty());

        Ok(())
    }

    #[test]
    fn test_extract_all_snapshots_basic() -> Result<(), BackupServiceError> {
        use crate::config::Config;

        let config = Config {
            restic_password: "test".to_string(),
            restic_repo_base: "s3:https://test.com/bucket".to_string(),
            aws_access_key_id: "test".to_string(),
            aws_secret_access_key: "test".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://test.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        };

        let ops = RepositoryOperations::new(config)?;

        let repo_data = vec![
            create_test_repo_data(
                "/home/tim/documents",
                "user_home/tim/documents",
                "user_home",
                vec![
                    create_test_snapshot("2025-01-15T10:30:00Z", "/home/tim/documents", "snap1"),
                    create_test_snapshot("2025-01-15T11:00:00Z", "/home/tim/documents", "snap2"),
                ],
            ),
            create_test_repo_data(
                "/mnt/docker-data/volumes/postgres",
                "docker_volume/postgres",
                "docker_volume",
                vec![
                    create_test_snapshot("2025-01-15T09:30:00Z", "/mnt/docker-data/volumes/postgres", "snap3"),
                    create_test_snapshot("2025-01-15T12:00:00Z", "/mnt/docker-data/volumes/postgres", "snap4"),
                ],
            ),
            create_test_repo_data(
                "/etc/nginx",
                "system/etc_nginx",
                "system",
                vec![], // No snapshots
            ),
        ];

        let all_snapshots = ops.extract_all_snapshots(&repo_data);

        // Should have 4 snapshots total (2 + 2 + 0)
        assert_eq!(all_snapshots.len(), 4);

        // Check that all snapshots are present
        let snapshot_ids: Vec<&String> = all_snapshots.iter().map(|s| &s.id).collect();
        assert!(snapshot_ids.contains(&&"snap1".to_string()));
        assert!(snapshot_ids.contains(&&"snap2".to_string()));
        assert!(snapshot_ids.contains(&&"snap3".to_string()));
        assert!(snapshot_ids.contains(&&"snap4".to_string()));

        Ok(())
    }

    #[test]
    fn test_extract_all_snapshots_empty() -> Result<(), BackupServiceError> {
        use crate::config::Config;

        let config = Config {
            restic_password: "test".to_string(),
            restic_repo_base: "s3:https://test.com/bucket".to_string(),
            aws_access_key_id: "test".to_string(),
            aws_secret_access_key: "test".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://test.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        };

        let ops = RepositoryOperations::new(config)?;

        // Test with no repo data
        let empty_repo_data: Vec<RepositoryData> = vec![];
        let snapshots = ops.extract_all_snapshots(&empty_repo_data);
        assert!(snapshots.is_empty());

        // Test with repo data but no snapshots
        let repo_data_no_snapshots = vec![
            create_test_repo_data(
                "/etc/nginx",
                "system/etc_nginx",
                "system",
                vec![], // No snapshots
            ),
            create_test_repo_data(
                "/var/log",
                "system/var_log",
                "system",
                vec![], // No snapshots
            ),
        ];

        let snapshots = ops.extract_all_snapshots(&repo_data_no_snapshots);
        assert!(snapshots.is_empty());

        Ok(())
    }

    #[test]
    fn test_extract_all_snapshots_ordering() -> Result<(), BackupServiceError> {
        use crate::config::Config;

        let config = Config {
            restic_password: "test".to_string(),
            restic_repo_base: "s3:https://test.com/bucket".to_string(),
            aws_access_key_id: "test".to_string(),
            aws_secret_access_key: "test".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://test.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        };

        let ops = RepositoryOperations::new(config)?;

        // Create repositories with snapshots in specific order
        let repo_data = vec![
            create_test_repo_data(
                "/home/tim/documents",
                "user_home/tim/documents",
                "user_home",
                vec![
                    create_test_snapshot("2025-01-15T10:30:00Z", "/home/tim/documents", "first"),
                    create_test_snapshot("2025-01-15T10:31:00Z", "/home/tim/documents", "second"),
                ],
            ),
            create_test_repo_data(
                "/etc/nginx",
                "system/etc_nginx",
                "system",
                vec![
                    create_test_snapshot("2025-01-15T10:32:00Z", "/etc/nginx", "third"),
                ],
            ),
        ];

        let all_snapshots = ops.extract_all_snapshots(&repo_data);

        // Snapshots should maintain repository order (flattened in order)
        assert_eq!(all_snapshots.len(), 3);
        assert_eq!(all_snapshots[0].id, "first");
        assert_eq!(all_snapshots[1].id, "second");
        assert_eq!(all_snapshots[2].id, "third");

        Ok(())
    }

    #[test]
    fn test_repository_data_integrity() -> Result<(), BackupServiceError> {
        // Test that RepositoryData properly maintains its integrity
        let snapshots = vec![
            create_test_snapshot("2025-01-15T10:30:00Z", "/test/path", "snap1"),
            create_test_snapshot("2025-01-15T11:00:00Z", "/test/path", "snap2"),
        ];

        let repo_data = create_test_repo_data(
            "/test/path",
            "system/test_path",
            "system",
            snapshots.clone(),
        );

        // Verify all fields are correctly set
        assert_eq!(repo_data.info.native_path, PathBuf::from("/test/path"));
        assert_eq!(repo_data.info.repo_subpath, "system/test_path");
        assert_eq!(repo_data.info.category, "system");
        assert_eq!(repo_data.snapshot_count, 2);
        assert_eq!(repo_data.snapshots.len(), 2);
        assert_eq!(repo_data.snapshots, snapshots);

        Ok(())
    }

    #[test]
    fn test_mixed_category_conversion() -> Result<(), BackupServiceError> {
        use crate::config::Config;

        let config = Config {
            restic_password: "test".to_string(),
            restic_repo_base: "s3:https://test.com/bucket".to_string(),
            aws_access_key_id: "test".to_string(),
            aws_secret_access_key: "test".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://test.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        };

        let ops = RepositoryOperations::new(config)?;

        // Test with mixed categories and different snapshot counts
        let repo_data = vec![
            create_test_repo_data("/home/alice/projects", "user_home/alice/projects", "user_home",
                vec![create_test_snapshot("2025-01-15T10:30:00Z", "/home/alice/projects", "snap1")]),
            create_test_repo_data("/mnt/docker-data/volumes/redis", "docker_volume/redis", "docker_volume",
                vec![]),
            create_test_repo_data("/var/log/app", "system/var_log_app", "system",
                vec![
                    create_test_snapshot("2025-01-15T09:00:00Z", "/var/log/app", "snap2"),
                    create_test_snapshot("2025-01-15T10:00:00Z", "/var/log/app", "snap3"),
                    create_test_snapshot("2025-01-15T11:00:00Z", "/var/log/app", "snap4"),
                ]),
        ];

        let backup_repos = ops.convert_to_backup_repos(repo_data.clone())?;
        let all_snapshots = ops.extract_all_snapshots(&repo_data);

        // Verify conversion maintained everything correctly
        assert_eq!(backup_repos.len(), 3);
        assert_eq!(all_snapshots.len(), 4); // 1 + 0 + 3

        // Check specific repositories
        assert_eq!(backup_repos[0].category()?, "user_home");
        assert_eq!(backup_repos[0].snapshot_count, 1);

        assert_eq!(backup_repos[1].category()?, "docker_volume");
        assert_eq!(backup_repos[1].snapshot_count, 0);

        assert_eq!(backup_repos[2].category()?, "system");
        assert_eq!(backup_repos[2].snapshot_count, 3);

        Ok(())
    }
}
