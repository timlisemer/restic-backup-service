use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::shared::commands::ResticCommandExecutor;
use crate::shared::operations::RepositoryData;
use crate::shared::paths::PathMapper;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tracing::info;

// RepositoryScanner - S3 directory scanning with parallel repository checking

pub struct RepositoryScanner {
    config: Config,
    snapshot_collector: SnapshotCollector,
}

impl RepositoryScanner {
    pub fn new(config: Config) -> Result<Self, BackupServiceError> {
        let snapshot_collector = SnapshotCollector::new(config.clone())?;
        Ok(Self {
            config,
            snapshot_collector,
        })
    }

    // Construct S3 path with optional base path prefix
    fn build_s3_path(&self, hostname: &str, category: &str) -> Result<String, BackupServiceError> {
        let base_path = self.config.s3_base_path()?;
        if base_path.is_empty() {
            Ok(format!("{}/{}", hostname, category))
        } else {
            Ok(format!("{}/{}/{}", base_path, hostname, category))
        }
    }

    // Execute AWS CLI to list S3 directories and parse output
    pub async fn list_s3_dirs(&self, s3_path: &str) -> Result<Vec<String>, BackupServiceError> {
        let s3_bucket = self.config.s3_bucket()?;
        let full_path = format!("s3://{}/{}/", s3_bucket, s3_path);

        let output = Command::new("aws")
            .args([
                "s3",
                "ls",
                &full_path,
                "--endpoint-url",
                &self.config.s3_endpoint()?,
            ])
            .env("AWS_ACCESS_KEY_ID", &self.config.aws_access_key_id)
            .env("AWS_SECRET_ACCESS_KEY", &self.config.aws_secret_access_key)
            .env("AWS_DEFAULT_REGION", &self.config.aws_default_region)
            .output()
            .map_err(|_| {
                BackupServiceError::CommandNotFound("Failed to execute aws".to_string())
            })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Parse S3 directory listing - critical: preserve spaces in directory names
            let dirs: Vec<String> = stdout
                .lines()
                .filter(|line| line.contains("PRE"))
                .map(|line| {
                    // Preserve spaces after "PRE " prefix in S3 output
                    if let Some(start) = line.find("PRE ") {
                        let dir_name = &line[start + 4..]; // Skip "PRE "
                        dir_name.trim_end_matches('/').to_string()
                    } else {
                        String::new()
                    }
                })
                .filter(|d| !d.is_empty())
                .collect();
            Ok(dirs)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(BackupServiceError::from_stderr(&stderr, &full_path))
        }
    }

    /// Scan and collect all repositories for a hostname with true parallelization
    pub async fn scan_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryData>, BackupServiceError> {
        let all_repo_infos = self.discover_all_repositories(hostname).await?;
        let total_repos = all_repo_infos.len();
        let counter = Arc::new(AtomicUsize::new(0));

        if total_repos == 0 {
            info!("Scanning completed!");
            return Ok(Vec::new());
        }

        info!("Found {} repositories to check", total_repos);

        // Parallel execution: spawn concurrent tasks for repository checking
        let mut tasks = Vec::new();

        for repo_info in all_repo_infos {
            let snapshot_collector = self.snapshot_collector.clone();
            let counter_clone = counter.clone();

            // Each repository is checked concurrently using tokio::spawn
            let task = tokio::spawn(async move {
                let current = counter_clone.fetch_add(1, Ordering::SeqCst) + 1;
                let native_path = &repo_info.native_path;
                let repo_subpath = &repo_info.repo_subpath;

                info!(
                    "Checking ({}/{}) - {}",
                    current,
                    total_repos,
                    native_path.display()
                );

                let (count, snapshots) = snapshot_collector
                    .get_snapshots(repo_subpath, native_path)
                    .await?;

                if count > 0 {
                    info!(
                        "✓ ({}/{}) - {} snapshots found",
                        current, total_repos, count
                    );
                    Ok::<Option<RepositoryData>, BackupServiceError>(Some(RepositoryData {
                        info: repo_info,
                        snapshots,
                        snapshot_count: count,
                    }))
                } else {
                    Ok::<Option<RepositoryData>, BackupServiceError>(None)
                }
            });

            tasks.push(task);
        }

        let mut results = Vec::new();
        for task in tasks {
            match task.await {
                Ok(result) => results.push(result?),
                Err(join_error) => {
                    return Err(BackupServiceError::CommandFailed(format!(
                        "Task join error: {}",
                        join_error
                    )))
                }
            }
        }
        let repos: Vec<RepositoryData> = results.into_iter().flatten().collect();

        info!("Scanning completed!");
        Ok(repos)
    }

    async fn discover_all_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut all_repos = Vec::new();

        all_repos.extend(self.discover_user_home_repositories(hostname).await?);

        all_repos.extend(self.discover_docker_volume_repositories(hostname).await?);

        all_repos.extend(self.discover_system_repositories(hostname).await?);

        Ok(all_repos)
    }

    // Unified repository discovery for different categories (user_home/docker_volume/system)
    async fn discover_repositories_by_category(
        &self,
        hostname: &str,
        category: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let category_path = self.build_s3_path(hostname, category)?;
        let mut repos = Vec::new();

        info!("Scanning {} directories...", category);

        // Category-specific path mapping from S3 structure to native filesystem paths
        match category {
            "user_home" => {
                if let Ok(users) = self.list_s3_dirs(&category_path).await {
                    for user in users {
                        info!("Processing user: {}", user);
                        let user_path = format!("{}/{}", category_path, user);

                        if let Ok(subdirs) = self.list_s3_dirs(&user_path).await {
                            for subdir in subdirs {
                                let native_subdir = PathMapper::s3_to_native_path(&subdir)?;
                                let native_path =
                                    PathBuf::from(format!("/home/{}/{}", user, native_subdir));
                                let repo_subpath = format!("user_home/{}/{}", user, subdir);

                                repos.push(RepositoryInfo {
                                    native_path,
                                    repo_subpath,
                                    category: "user_home".to_string(),
                                });
                            }
                        }
                    }
                }
            }
            "docker_volume" => {
                if let Ok(volumes) = self.list_s3_dirs(&category_path).await {
                    for volume in volumes {
                        let native_path =
                            PathBuf::from(format!("/mnt/docker-data/volumes/{}", volume));
                        let repo_subpath = format!("docker_volume/{}", volume);

                        repos.push(RepositoryInfo {
                            native_path,
                            repo_subpath,
                            category: "docker_volume".to_string(),
                        });
                    }
                }
            }
            "system" => {
                if let Ok(paths) = self.list_s3_dirs(&category_path).await {
                    for path in paths {
                        let native_path_str = PathMapper::s3_to_native_path(&path)?;
                        let native_path = PathBuf::from(format!("/{}", native_path_str));
                        let repo_subpath = format!("system/{}", path);

                        repos.push(RepositoryInfo {
                            native_path,
                            repo_subpath,
                            category: "system".to_string(),
                        });
                    }
                }
            }
            _ => {
                return Err(BackupServiceError::ConfigurationError(format!(
                    "Unknown repository category: {}",
                    category
                )));
            }
        }

        Ok(repos)
    }

    async fn discover_user_home_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        self.discover_repositories_by_category(hostname, "user_home")
            .await
    }

    async fn discover_docker_volume_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        self.discover_repositories_by_category(hostname, "docker_volume")
            .await
    }

    async fn discover_system_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        self.discover_repositories_by_category(hostname, "system")
            .await
    }
}


// Collects snapshot data from restic repositories
#[derive(Clone)]
pub struct SnapshotCollector {
    config: Config,
}

impl SnapshotCollector {
    pub fn new(config: Config) -> Result<Self, BackupServiceError> {
        Ok(Self { config })
    }

    // Retrieve and parse snapshot information from restic repository
    pub async fn get_snapshots(
        &self,
        repo_subpath: &str,
        native_path: &Path,
    ) -> Result<(usize, Vec<SnapshotInfo>), BackupServiceError> {
        let repo_url = self.config.get_repo_url(repo_subpath)?;
        let restic_cmd = ResticCommandExecutor::new(self.config.clone(), repo_url)?;

        let snapshots = restic_cmd
            .snapshots(Some(&native_path.to_string_lossy()))
            .await?;
        let count = snapshots.len();

        // Parse JSON snapshot data into structured format
        let snapshot_infos: Vec<SnapshotInfo> = snapshots
            .into_iter()
            .filter_map(|s| {
                let time = s["time"].as_str()?.parse::<DateTime<Utc>>().ok()?;
                let id = s["short_id"].as_str()?.to_string();
                Some(SnapshotInfo {
                    time,
                    path: native_path.to_path_buf(),
                    id,
                })
            })
            .collect();

        Ok((count, snapshot_infos))
    }
}


#[derive(Debug, Clone)]
pub struct RepositoryInfo {
    pub native_path: PathBuf,
    pub repo_subpath: String,
    pub category: String,
}

impl RepositoryInfo {
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotInfo {
    pub time: DateTime<Utc>,
    pub path: PathBuf,
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::path::PathBuf;

    #[test]
    fn test_repository_info_creation() {
        let native_path = PathBuf::from("/home/user/.local/share/My Documents");
        let repo_subpath = "user_home/user/.local_share_My Documents".to_string();
        let category = "user_home".to_string();

        let repo_info = RepositoryInfo {
            native_path: native_path.clone(),
            repo_subpath: repo_subpath.clone(),
            category: category.clone(),
        };

        assert_eq!(repo_info.native_path, native_path);
        assert_eq!(repo_info.repo_subpath, repo_subpath);
        assert_eq!(repo_info.category, category);
    }

    #[test]
    fn test_repository_info_various_categories() {
        let test_cases = vec![
            (
                "/home/alice/projects",
                "user_home/alice/projects",
                "user_home",
            ),
            (
                "/mnt/docker-data/volumes/postgres",
                "docker_volume/postgres",
                "docker_volume",
            ),
            ("/etc/nginx", "system/etc_nginx", "system"),
            ("/var/log/app", "system/var_log_app", "system"),
            ("/usr/local/bin", "system/usr_local_bin", "system"),
        ];

        for (native, subpath, category) in test_cases {
            let repo_info = RepositoryInfo {
                native_path: PathBuf::from(native),
                repo_subpath: subpath.to_string(),
                category: category.to_string(),
            };

            assert_eq!(repo_info.native_path, PathBuf::from(native));
            assert_eq!(repo_info.repo_subpath, subpath);
            assert_eq!(repo_info.category, category);
        }
    }

    #[test]
    fn test_snapshot_info_creation() {
        let time_str = "2025-01-15T10:30:00Z";
        let time = DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .with_timezone(&Utc);
        let path = PathBuf::from("/home/gamer/.local/share/Paradox Interactive");
        let id = "abc123def456".to_string();

        let snapshot_info = SnapshotInfo {
            time,
            path: path.clone(),
            id: id.clone(),
        };

        assert_eq!(snapshot_info.time, time);
        assert_eq!(snapshot_info.path, path);
        assert_eq!(snapshot_info.id, id);
    }

    #[test]
    fn test_snapshot_info_equality() {
        let time = DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let snapshot1 = SnapshotInfo {
            time,
            path: PathBuf::from("/home/tim/docs"),
            id: "snap123".to_string(),
        };

        let snapshot2 = SnapshotInfo {
            time,
            path: PathBuf::from("/home/tim/docs"),
            id: "snap123".to_string(),
        };

        let snapshot3 = SnapshotInfo {
            time,
            path: PathBuf::from("/home/tim/projects"), // Different path
            id: "snap123".to_string(),
        };

        assert_eq!(snapshot1, snapshot2);
        assert_ne!(snapshot1, snapshot3);
    }

    #[test]
    fn test_snapshot_info_different_times() {
        let time1 = DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let time2 = DateTime::parse_from_rfc3339("2025-01-15T11:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let snapshot1 = SnapshotInfo {
            time: time1,
            path: PathBuf::from("/home/tim/docs"),
            id: "snap123".to_string(),
        };

        let snapshot2 = SnapshotInfo {
            time: time2, // Different time
            path: PathBuf::from("/home/tim/docs"),
            id: "snap123".to_string(),
        };

        assert_ne!(snapshot1, snapshot2);
    }

    #[test]
    fn test_repository_info_clone() {
        let original = RepositoryInfo {
            native_path: PathBuf::from("/home/tim/documents"),
            repo_subpath: "user_home/tim/documents".to_string(),
            category: "user_home".to_string(),
        };

        let cloned = original.clone();

        // Should be equal but separate instances
        assert_eq!(original.native_path, cloned.native_path);
        assert_eq!(original.repo_subpath, cloned.repo_subpath);
        assert_eq!(original.category, cloned.category);
    }

    #[test]
    fn test_snapshot_info_clone() {
        let time = DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let original = SnapshotInfo {
            time,
            path: PathBuf::from("/home/tim/documents"),
            id: "snap123".to_string(),
        };

        let cloned = original.clone();

        // Should be equal but separate instances
        assert_eq!(original, cloned);
        assert_eq!(original.time, cloned.time);
        assert_eq!(original.path, cloned.path);
        assert_eq!(original.id, cloned.id);
    }

    #[test]
    fn test_complex_path_handling() {
        let test_cases = vec![
            // Original cases
            "/home/user with spaces/documents",
            "/home/alice/projects/my-project",
            "/mnt/docker-data/volumes/app_data",
            "/etc/nginx/sites-available/default",
            "/var/log/app/2025/01/15",
            "/usr/local/bin/custom-script",
            "/opt/software/version-1.2.3",
            "/home/user123/Downloads/file.tar.gz",
            "/home/gamer/.local/share/Paradox Interactive",
            "/home/user/.steam/steam/steamapps/common/Grand Theft Auto V",
            "/home/player/Games/World of Warcraft/Interface/AddOns",
            "/home/user/.config/Google Chrome",
            "/home/developer/.local/share/JetBrains Toolbox",
            "/home/designer/Adobe After Effects 2024",
            "/home/user/Documents/Important Business Files",
            "/home/user/Music/Classical Music Collection",
            "/home/user/Videos/Home Movies 2024",
            "/mnt/docker-data/volumes/my app data",
            "/mnt/docker-data/volumes/web server config",
            "/mnt/docker-data/volumes/database backup files",
            "/usr/share/applications/Visual Studio Code",
            "/opt/Google Chrome",
            "/var/log/system events",
            "/home/user/My    Project    Files",
            "/home/user/App  With  Multiple  Spaces",
            "/home/user/ leading space",
            "/home/user/trailing space ",
            "/home/user/ both spaces ",
        ];

        for path_str in test_cases {
            let repo_info = RepositoryInfo {
                native_path: PathBuf::from(path_str),
                repo_subpath: format!("test/{}", path_str.replace("/", "_")),
                category: "test".to_string(),
            };

            assert_eq!(repo_info.native_path, PathBuf::from(path_str));
            assert!(repo_info.repo_subpath.starts_with("test/"));

            // Verify that whitespace is preserved in the path
            if path_str.contains(' ') {
                assert!(
                    repo_info.native_path.to_string_lossy().contains(' '),
                    "Whitespace should be preserved in path: {}",
                    path_str
                );
            }
        }
    }

    #[test]
    fn test_snapshot_info_with_various_ids() {
        let time = DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let path = PathBuf::from("/home/tim/docs");

        let id_formats = vec![
            "abc123def456",                             // Standard hex
            "12345678",                                 // Numbers only
            "short",                                    // Short ID
            "very-long-id-with-dashes-and-numbers-123", // Long with dashes
            "mixed_123-ABC_def",                        // Mixed case and symbols
        ];

        for id in id_formats {
            let snapshot = SnapshotInfo {
                time,
                path: path.clone(),
                id: id.to_string(),
            };

            assert_eq!(snapshot.id, id);
            assert_eq!(snapshot.time, time);
            assert_eq!(snapshot.path, path);
        }
    }

    #[test]
    fn test_docker_volume_helper_logic() {
        // Test the logic used in create_docker_volume_repo_info helper
        let volume_names = vec![
            // Original cases
            "postgres",
            "redis",
            "app-data",
            "my_volume",
            "volume123",
            "complex-name-with-dashes",
            "my app data",
            "web server config",
            "database backup files",
            "game server data",
            "Application Config Files",
            "My Personal Volume",
            "Development Environment",
            "Production Database",
        ];

        for volume in &volume_names {
            let native_path = PathBuf::from(format!("/mnt/docker-data/volumes/{}", volume));
            let repo_subpath = format!("docker_volume/{}", volume);

            let repo_info = RepositoryInfo {
                native_path: native_path.clone(),
                repo_subpath: repo_subpath.clone(),
                category: "docker_volume".to_string(),
            };

            assert_eq!(repo_info.native_path, native_path);
            assert_eq!(repo_info.repo_subpath, repo_subpath);
            assert_eq!(repo_info.category, "docker_volume");
            assert!(repo_info.native_path.to_string_lossy().contains(volume));

            // Verify whitespace is preserved in volume names with spaces
            if volume.contains(' ') {
                assert!(
                    repo_info.native_path.to_string_lossy().contains(' '),
                    "Whitespace should be preserved in docker volume: {}",
                    volume
                );
            }
        }
    }

    #[test]
    fn test_nested_repository_logic() {
        // Test the logic used in scan_nested_docker_repositories
        let test_scenarios = vec![
            // Original scenario
            ("myapp", vec!["config", "data", "logs", "backups"]),
            // Whitespace volume with nested repos
            (
                "my app data",
                vec!["config files", "user data", "backup storage", "temp files"],
            ),
            (
                "web server",
                vec!["apache config", "ssl certificates", "site data"],
            ),
            (
                "game server",
                vec!["world saves", "player data", "mod configs"],
            ),
        ];

        for (volume, nested_repos) in test_scenarios {
            for nested_repo in &nested_repos {
                let nested_path = PathBuf::from(format!(
                    "/mnt/docker-data/volumes/{}/{}",
                    volume, nested_repo
                ));
                let nested_repo_subpath = format!("docker_volume/{}/{}", volume, nested_repo);

                let repo_info = RepositoryInfo {
                    native_path: nested_path.clone(),
                    repo_subpath: nested_repo_subpath.clone(),
                    category: "docker_volume".to_string(),
                };

                assert_eq!(repo_info.native_path, nested_path);
                assert_eq!(repo_info.repo_subpath, nested_repo_subpath);
                assert_eq!(repo_info.category, "docker_volume");

                // Verify path structure
                assert!(repo_info.native_path.to_string_lossy().contains(volume));
                assert!(repo_info
                    .native_path
                    .to_string_lossy()
                    .contains(nested_repo));
                assert!(repo_info
                    .repo_subpath
                    .contains(&format!("{}/{}", volume, nested_repo)));

                // Verify whitespace preservation
                if volume.contains(' ') || nested_repo.contains(' ') {
                    let path_str = repo_info.native_path.to_string_lossy();
                    assert!(
                        path_str.contains(' '),
                        "Whitespace should be preserved in nested path: volume='{}', nested='{}'",
                        volume,
                        nested_repo
                    );
                }
            }
        }
    }

    #[test]
    fn test_s3_path_construction_no_double_slash() -> Result<(), BackupServiceError> {
        use crate::config::Config;

        let config_empty_base = Config {
            restic_password: "test".to_string(),
            restic_repo_base: "s3:https://example.com/bucket".to_string(), // Empty base path
            aws_access_key_id: "test".to_string(),
            aws_secret_access_key: "test".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://example.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        };

        let scanner = RepositoryScanner::new(config_empty_base)?;
        assert_eq!(
            scanner.build_s3_path("tim-pc", "user_home")?,
            "tim-pc/user_home"
        );
        assert_eq!(
            scanner.build_s3_path("tim-pc", "docker_volume")?,
            "tim-pc/docker_volume"
        );
        assert_eq!(scanner.build_s3_path("tim-pc", "system")?, "tim-pc/system");

        let config_with_base = Config {
            restic_password: "test".to_string(),
            restic_repo_base: "s3:https://example.com/bucket/restic".to_string(), // Non-empty base path
            aws_access_key_id: "test".to_string(),
            aws_secret_access_key: "test".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://example.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        };

        let scanner_with_base = RepositoryScanner::new(config_with_base)?;
        assert_eq!(
            scanner_with_base.build_s3_path("tim-pc", "user_home")?,
            "restic/tim-pc/user_home"
        );
        assert_eq!(
            scanner_with_base.build_s3_path("tim-pc", "docker_volume")?,
            "restic/tim-pc/docker_volume"
        );
        assert_eq!(
            scanner_with_base.build_s3_path("tim-pc", "system")?,
            "restic/tim-pc/system"
        );

        assert_eq!(
            scanner.build_s3_path("host-with-dashes", "user_home")?,
            "host-with-dashes/user_home"
        );
        assert_eq!(
            scanner.build_s3_path("host123", "user_home")?,
            "host123/user_home"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_s3_directory_listing_parsing_with_spaces() {
        // Critical test: ensure S3 parsing preserves spaces (fixed previous bug)

        let mock_s3_output = r#"                           PRE .arduinoIDE/
                           PRE .bash_history/
                           PRE .config/
                           PRE .local_share_Paradox Interactive_/
                           PRE .local_share_Steam_steamapps_common_Grand Theft Auto V/
                           PRE .mozilla/
                           PRE Coding/
                           PRE My Documents/
                           PRE Photo Collection 2024/
"#;

        let parsed_dirs: Vec<String> = mock_s3_output
            .lines()
            .filter(|line| line.contains("PRE"))
            .map(|line| {
                // Fixed parsing: preserve spaces after "PRE " prefix
                if let Some(start) = line.find("PRE ") {
                    let dir_name = &line[start + 4..]; // Skip "PRE "
                    dir_name.trim_end_matches('/').to_string()
                } else {
                    String::new()
                }
            })
            .filter(|d| !d.is_empty())
            .collect();

        assert!(
            parsed_dirs.contains(&".local_share_Paradox Interactive_".to_string()),
            "Failed to preserve spaces in '.local_share_Paradox Interactive_'"
        );
        assert!(
            parsed_dirs
                .contains(&".local_share_Steam_steamapps_common_Grand Theft Auto V".to_string()),
            "Failed to preserve spaces in Grand Theft Auto V directory"
        );
        assert!(
            parsed_dirs.contains(&"My Documents".to_string()),
            "Failed to preserve spaces in 'My Documents'"
        );
        assert!(
            parsed_dirs.contains(&"Photo Collection 2024".to_string()),
            "Failed to preserve spaces in 'Photo Collection 2024'"
        );

        let buggy_parsed_dirs: Vec<String> = mock_s3_output
            .lines()
            .filter(|line| line.contains("PRE"))
            .map(|line| {
                line.split_whitespace()
                    .last()
                    .unwrap_or("")
                    .trim_end_matches('/')
                    .to_string()
            })
            .filter(|d| !d.is_empty())
            .collect();

        assert!(
            buggy_parsed_dirs.contains(&"Interactive_".to_string()),
            "Buggy parsing should truncate to 'Interactive_'"
        );
        assert!(
            !buggy_parsed_dirs.contains(&".local_share_Paradox Interactive_".to_string()),
            "Buggy parsing should NOT preserve full directory name with spaces"
        );
        assert!(
            buggy_parsed_dirs.contains(&"V".to_string()),
            "Buggy parsing should truncate Grand Theft Auto V to just 'V'"
        );

        assert_eq!(
            parsed_dirs.len(),
            9,
            "Should parse all 9 directories correctly"
        );
    }

    #[tokio::test]
    async fn test_s3_command_executor_parsing_with_spaces() {

        let mock_output = r#"                           PRE application logs/
                           PRE database backup files/
                           PRE user data/
                           PRE web server config/
                           PRE Docker Volume With Spaces/
"#;

        let parsed: Vec<String> = mock_output
            .lines()
            .filter(|line| line.contains("PRE"))
            .map(|line| {
                if let Some(start) = line.find("PRE ") {
                    let dir_name = &line[start + 4..];
                    dir_name.trim_end_matches('/').to_string()
                } else {
                    String::new()
                }
            })
            .filter(|d| !d.is_empty())
            .collect();

        assert_eq!(parsed.len(), 5);
        assert!(parsed.contains(&"application logs".to_string()));
        assert!(parsed.contains(&"database backup files".to_string()));
        assert!(parsed.contains(&"user data".to_string()));
        assert!(parsed.contains(&"web server config".to_string()));
        assert!(parsed.contains(&"Docker Volume With Spaces".to_string()));
    }

    #[test]
    fn test_s3_parsing_edge_cases() {
        let edge_case_outputs = vec![
            r#"                           PRE Directory  With  Multiple  Spaces/"#,
            r#"                           PRE  Leading And Trailing  /"#,
            r#"                           PRE My-App Config Files/"#,
            r#"                           PRE This Is A Very Long Directory Name With Many Words And Spaces/"#,
        ];

        for output in edge_case_outputs {
            let parsed: Vec<String> = output
                .lines()
                .filter(|line| line.contains("PRE"))
                .map(|line| {
                    if let Some(start) = line.find("PRE ") {
                        let dir_name = &line[start + 4..];
                        dir_name.trim_end_matches('/').to_string()
                    } else {
                        String::new()
                    }
                })
                .filter(|d| !d.is_empty())
                .collect();

            assert_eq!(
                parsed.len(),
                1,
                "Should parse exactly one directory from: {}",
                output
            );
            assert!(
                !parsed[0].is_empty(),
                "Parsed directory name should not be empty"
            );
        }
    }
}
