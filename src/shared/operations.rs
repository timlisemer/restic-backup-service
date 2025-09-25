use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::helpers::{RepositoryInfo, RepositoryScanner, SnapshotInfo};
use crate::repository::BackupRepo;
use crate::shared::commands::S3CommandExecutor;
use crate::shared::ui::RepositorySelectionItem;

// High-level operations manager for repository scanning and data collection
pub struct RepositoryOperations {
    config: Config,
    scanner: RepositoryScanner,
}

// Combined repository information with snapshot data
#[derive(Debug, Clone)]
pub struct RepositoryData {
    pub info: RepositoryInfo,
    pub snapshots: Vec<SnapshotInfo>,
    pub snapshot_count: usize,
}

impl RepositoryOperations {
    pub fn new(config: Config) -> Result<Self, BackupServiceError> {
        let scanner = RepositoryScanner::new(config.clone())?;

        Ok(Self {
            config,
            scanner,
        })
    }

    // Main entrypoint to collect all repository data for a hostname
    pub async fn collect_backup_data(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryData>, BackupServiceError> {
        self.scanner.scan_repositories(hostname).await
    }

    // Retrieve available backup hosts from S3 storage
    pub async fn get_available_hosts(&self) -> Result<Vec<String>, BackupServiceError> {
        let s3_executor = S3CommandExecutor::new(self.config.clone())?;
        s3_executor.get_hosts().await
    }

    // Convert repository data to BackupRepo format
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

    // Flatten all snapshots from all repositories into a single collection
    pub fn extract_all_snapshots(&self, repo_data: &[RepositoryData]) -> Vec<SnapshotInfo> {
        repo_data
            .iter()
            .flat_map(|repo| &repo.snapshots)
            .cloned()
            .collect()
    }

    // Convert repository data to UI selection format
    pub fn convert_to_selection_items(
        &self,
        repo_data: Vec<RepositoryData>,
    ) -> Result<Vec<RepositorySelectionItem>, BackupServiceError> {
        use crate::shared::ui::{RepositorySelectionItem, SnapshotItem};

        let mut repos = Vec::new();
        for repo_info in repo_data {
            if !repo_info.snapshots.is_empty() {
                let snapshots = repo_info.snapshots
                    .into_iter()
                    .map(|s| SnapshotItem {
                        id: s.id,
                        time: s.time,
                    })
                    .collect();

                repos.push(RepositorySelectionItem {
                    path: repo_info.info.native_path,
                    repo_subpath: repo_info.info.repo_subpath,
                    category: repo_info.info.category,
                    snapshots,
                });
            }
        }
        Ok(repos)
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

    fn create_test_repo_info(
        native_path: &str,
        repo_subpath: &str,
        category: &str,
    ) -> RepositoryInfo {
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

        let repo_data = vec![
            create_test_repo_data(
                "/home/tim/.local/share/My Documents",
                "user_home/tim/.local_share_My Documents",
                "user_home",
                vec![
                    create_test_snapshot(
                        "2025-01-15T10:30:00Z",
                        "/home/tim/.local/share/My Documents",
                        "snap1",
                    ),
                    create_test_snapshot(
                        "2025-01-15T11:00:00Z",
                        "/home/tim/.local/share/My Documents",
                        "snap2",
                    ),
                ],
            ),
            create_test_repo_data(
                "/mnt/docker-data/volumes/postgres backup",
                "docker_volume/postgres backup",
                "docker_volume",
                vec![create_test_snapshot(
                    "2025-01-15T09:30:00Z",
                    "/mnt/docker-data/volumes/postgres backup",
                    "snap3",
                )],
            ),
            create_test_repo_data(
                "/etc/systemd/system/my service.service",
                "system/etc_systemd_system_my service.service",
                "system",
                vec![], // No snapshots
            ),
        ];

        let backup_repos = ops.convert_to_backup_repos(repo_data)?;

        assert_eq!(backup_repos.len(), 3);

        assert_eq!(
            backup_repos[0].native_path,
            PathBuf::from("/home/tim/.local/share/My Documents")
        );
        assert_eq!(backup_repos[0].snapshot_count, 2);
        assert_eq!(backup_repos[0].category()?, "user_home");

        assert_eq!(
            backup_repos[1].native_path,
            PathBuf::from("/mnt/docker-data/volumes/postgres backup")
        );
        assert_eq!(backup_repos[1].snapshot_count, 1);
        assert_eq!(backup_repos[1].category()?, "docker_volume");

        assert_eq!(
            backup_repos[2].native_path,
            PathBuf::from("/etc/systemd/system/my service.service")
        );
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
                "/home/user/.config/Google Chrome",
                "user_home/user/.config_Google Chrome",
                "user_home",
                vec![
                    create_test_snapshot(
                        "2025-01-15T10:30:00Z",
                        "/home/user/.config/Google Chrome",
                        "snap1",
                    ),
                    create_test_snapshot(
                        "2025-01-15T11:00:00Z",
                        "/home/user/.config/Google Chrome",
                        "snap2",
                    ),
                ],
            ),
            create_test_repo_data(
                "/mnt/docker-data/volumes/database backup",
                "docker_volume/database backup",
                "docker_volume",
                vec![
                    create_test_snapshot(
                        "2025-01-15T09:30:00Z",
                        "/mnt/docker-data/volumes/database backup",
                        "snap3",
                    ),
                    create_test_snapshot(
                        "2025-01-15T12:00:00Z",
                        "/mnt/docker-data/volumes/database backup",
                        "snap4",
                    ),
                ],
            ),
            create_test_repo_data(
                "/usr/share/applications/My App",
                "system/usr_share_applications_My App",
                "system",
                vec![], // No snapshots
            ),
        ];

        let all_snapshots = ops.extract_all_snapshots(&repo_data);

        assert_eq!(all_snapshots.len(), 4);

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

        let empty_repo_data: Vec<RepositoryData> = vec![];
        let snapshots = ops.extract_all_snapshots(&empty_repo_data);
        assert!(snapshots.is_empty());

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
                vec![create_test_snapshot(
                    "2025-01-15T10:32:00Z",
                    "/etc/nginx",
                    "third",
                )],
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
            create_test_repo_data(
                "/home/alice/projects",
                "user_home/alice/projects",
                "user_home",
                vec![create_test_snapshot(
                    "2025-01-15T10:30:00Z",
                    "/home/alice/projects",
                    "snap1",
                )],
            ),
            create_test_repo_data(
                "/mnt/docker-data/volumes/redis",
                "docker_volume/redis",
                "docker_volume",
                vec![],
            ),
            create_test_repo_data(
                "/var/log/app",
                "system/var_log_app",
                "system",
                vec![
                    create_test_snapshot("2025-01-15T09:00:00Z", "/var/log/app", "snap2"),
                    create_test_snapshot("2025-01-15T10:00:00Z", "/var/log/app", "snap3"),
                    create_test_snapshot("2025-01-15T11:00:00Z", "/var/log/app", "snap4"),
                ],
            ),
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

    #[test]
    fn test_operations_with_whitespace_paths() -> Result<(), BackupServiceError> {
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

        // Test with whitespace paths in all categories
        let repo_data = vec![
            create_test_repo_data(
                "/home/gamer/.local/share/Paradox Interactive",
                "user_home/gamer/.local_share_Paradox Interactive",
                "user_home",
                vec![
                    create_test_snapshot("2025-01-15T10:30:00Z", "/home/gamer/.local/share/Paradox Interactive", "game1"),
                    create_test_snapshot("2025-01-15T11:00:00Z", "/home/gamer/.local/share/Paradox Interactive", "game2"),
                ],
            ),
            create_test_repo_data(
                "/home/user/.config/Google Chrome",
                "user_home/user/.config_Google Chrome",
                "user_home",
                vec![
                    create_test_snapshot("2025-01-15T09:30:00Z", "/home/user/.config/Google Chrome", "browser1"),
                ],
            ),
            create_test_repo_data(
                "/mnt/docker-data/volumes/my app data",
                "docker_volume/my app data",
                "docker_volume",
                vec![
                    create_test_snapshot("2025-01-15T08:00:00Z", "/mnt/docker-data/volumes/my app data", "docker1"),
                    create_test_snapshot("2025-01-15T08:30:00Z", "/mnt/docker-data/volumes/my app data", "docker2"),
                    create_test_snapshot("2025-01-15T09:00:00Z", "/mnt/docker-data/volumes/my app data", "docker3"),
                ],
            ),
            create_test_repo_data(
                "/usr/share/applications/Visual Studio Code",
                "system/usr_share_applications_Visual Studio Code",
                "system",
                vec![
                    create_test_snapshot("2025-01-15T07:30:00Z", "/usr/share/applications/Visual Studio Code", "app1"),
                ],
            ),

            // NixOS-style backup paths (similar to user's configuration)
            create_test_repo_data(
                "/home/developer/Coding",
                "user_home/developer/Coding",
                "user_home",
                vec![
                    create_test_snapshot("2025-01-15T12:00:00Z", "/home/developer/Coding", "code1"),
                    create_test_snapshot("2025-01-15T12:30:00Z", "/home/developer/Coding", "code2"),
                ],
            ),
            create_test_repo_data(
                "/home/user/.vscode-server",
                "user_home/user/.vscode-server",
                "user_home",
                vec![
                    create_test_snapshot("2025-01-15T06:00:00Z", "/home/user/.vscode-server", "vscode1"),
                ],
            ),
            create_test_repo_data(
                "/home/gamer/.local/share/Steam/steamapps/compatdata/567890/pfx/drive_c/users/steamuser/Documents/Game Data",
                "user_home/gamer/.local_share_Steam_steamapps_compatdata_567890_pfx_drive_c_users_steamuser_Documents_Game Data",
                "user_home",
                vec![
                    create_test_snapshot("2025-01-15T05:00:00Z", "/home/gamer/.local/share/Steam/steamapps/compatdata/567890/pfx/drive_c/users/steamuser/Documents/Game Data", "steam1"),
                ],
            ),
        ];

        // Test conversion to BackupRepo format
        let backup_repos = ops.convert_to_backup_repos(repo_data.clone())?;
        assert_eq!(backup_repos.len(), 7);

        // Verify whitespace paths are handled correctly
        assert_eq!(
            backup_repos[0].native_path.display().to_string(),
            "/home/gamer/.local/share/Paradox Interactive"
        );
        assert_eq!(backup_repos[0].snapshot_count, 2);
        assert_eq!(backup_repos[0].category()?, "user_home");

        assert_eq!(
            backup_repos[1].native_path.display().to_string(),
            "/home/user/.config/Google Chrome"
        );
        assert_eq!(backup_repos[1].snapshot_count, 1);
        assert_eq!(backup_repos[1].category()?, "user_home");

        assert_eq!(
            backup_repos[2].native_path.display().to_string(),
            "/mnt/docker-data/volumes/my app data"
        );
        assert_eq!(backup_repos[2].snapshot_count, 3);
        assert_eq!(backup_repos[2].category()?, "docker_volume");

        assert_eq!(
            backup_repos[3].native_path.display().to_string(),
            "/usr/share/applications/Visual Studio Code"
        );
        assert_eq!(backup_repos[3].snapshot_count, 1);
        assert_eq!(backup_repos[3].category()?, "system");

        // Test snapshot extraction with whitespace paths
        let all_snapshots = ops.extract_all_snapshots(&repo_data);
        assert_eq!(all_snapshots.len(), 11); // 2 + 1 + 3 + 1 + 2 + 1 + 1 (original + NixOS paths)

        // Verify snapshot paths contain spaces correctly
        let snapshot_paths: Vec<String> = all_snapshots
            .iter()
            .map(|s| s.path.display().to_string())
            .collect();

        assert!(
            snapshot_paths.contains(&"/home/gamer/.local/share/Paradox Interactive".to_string())
        );
        assert!(snapshot_paths.contains(&"/home/user/.config/Google Chrome".to_string()));
        assert!(snapshot_paths.contains(&"/mnt/docker-data/volumes/my app data".to_string()));
        assert!(snapshot_paths.contains(&"/usr/share/applications/Visual Studio Code".to_string()));

        Ok(())
    }
}
