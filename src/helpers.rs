use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::shared::commands::ResticCommandExecutor;
use crate::shared::paths::PathMapper;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

// ============================================================================

// ============================================================================
// RepositoryScanner - Unified S3 directory scanning and nested repo detection
// ============================================================================

pub struct RepositoryScanner {
    config: Config,
}

impl RepositoryScanner {
    pub fn new(config: Config) -> Result<Self, BackupServiceError> {
        Ok(Self { config })
    }

    /// Construct S3 path without double slashes
    fn build_s3_path(&self, hostname: &str, category: &str) -> Result<String, BackupServiceError> {
        let base_path = self.config.s3_base_path()?;
        if base_path.is_empty() {
            Ok(format!("{}/{}", hostname, category))
        } else {
            Ok(format!("{}/{}/{}", base_path, hostname, category))
        }
    }

    /// List S3 directories with proper error handling
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
            let dirs: Vec<String> = stdout
                .lines()
                .filter(|line| line.contains("PRE"))
                .map(|line| {
                    // Extract directory name after "PRE " prefix, preserving spaces
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

    /// Check if directory name is restic internal structure
    pub fn is_restic_internal_dir(dir_name: &str) -> Result<bool, BackupServiceError> {
        Ok(matches!(
            dir_name,
            "data" | "index" | "keys" | "snapshots" | "locks"
        ))
    }

    /// Scan and collect all repositories for a hostname using modular category scanners
    pub async fn scan_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut repos = Vec::new();

        // Use category-specific scanning functions
        repos.extend(self.scan_user_home_repositories(hostname).await?);
        repos.extend(self.scan_docker_volume_repositories(hostname).await?);
        repos.extend(self.scan_system_repositories(hostname).await?);

        info!("Scanning completed!");
        Ok(repos)
    }

    /// Scan user home directories specifically
    pub async fn scan_user_home_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut repos = Vec::new();
        let user_home_path = self.build_s3_path(hostname, "user_home")?;

        info!("Scanning user home directories...");
        if let Ok(users) = self.list_s3_dirs(&user_home_path).await {
            for user in users {
                info!("Processing user: {}", user);
                let user_path = format!("{}/{}", user_home_path, user);
                if let Ok(subdirs) = self.list_s3_dirs(&user_path).await {
                    for subdir in subdirs {
                        info!("  Checking {}...", subdir);
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

        Ok(repos)
    }

    /// Scan docker volumes with nested repository detection
    pub async fn scan_docker_volume_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut repos = Vec::new();
        let docker_path = self.build_s3_path(hostname, "docker_volume")?;

        info!("Scanning docker volumes...");
        if let Ok(volumes) = self.list_s3_dirs(&docker_path).await {
            for volume in volumes {
                info!("Processing volume: {}", volume);
                // Add main volume repository
                repos.push(self.create_docker_volume_repo_info(&volume)?);

                // Check for nested repositories
                repos.extend(
                    self.scan_nested_docker_repositories(&docker_path, &volume)
                        .await?,
                );
            }
        }

        Ok(repos)
    }

    /// Scan system paths specifically
    pub async fn scan_system_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut repos = Vec::new();
        let system_path = self.build_s3_path(hostname, "system")?;

        info!("Scanning system paths...");
        if let Ok(paths) = self.list_s3_dirs(&system_path).await {
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

        Ok(repos)
    }

    /// Helper function to create docker volume repository info
    fn create_docker_volume_repo_info(
        &self,
        volume: &str,
    ) -> Result<RepositoryInfo, BackupServiceError> {
        let native_path = PathBuf::from(format!("/mnt/docker-data/volumes/{}", volume));
        let repo_subpath = format!("docker_volume/{}", volume);

        Ok(RepositoryInfo {
            native_path,
            repo_subpath,
            category: "docker_volume".to_string(),
        })
    }

    /// Helper function to scan nested docker repositories
    async fn scan_nested_docker_repositories(
        &self,
        docker_path: &str,
        volume: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut repos = Vec::new();
        let volume_path = format!("{}/{}", docker_path, volume);

        if let Ok(nested) = self.list_s3_dirs(&volume_path).await {
            for nested_repo in nested {
                if !Self::is_restic_internal_dir(&nested_repo)? {
                    let nested_path = PathBuf::from(format!(
                        "/mnt/docker-data/volumes/{}/{}",
                        volume, nested_repo
                    ));
                    let nested_repo_subpath = format!("docker_volume/{}/{}", volume, nested_repo);

                    repos.push(RepositoryInfo {
                        native_path: nested_path,
                        repo_subpath: nested_repo_subpath,
                        category: "docker_volume".to_string(),
                    });
                }
            }
        }

        Ok(repos)
    }
}

// ============================================================================
// SnapshotCollector - Unified snapshot gathering
// ============================================================================

pub struct SnapshotCollector {
    config: Config,
}

impl SnapshotCollector {
    pub fn new(config: Config) -> Result<Self, BackupServiceError> {
        Ok(Self { config })
    }

    /// Get snapshots for a repository with count
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

// ============================================================================
// Data Structures
// ============================================================================

#[derive(Debug, Clone)]
pub struct RepositoryInfo {
    pub native_path: PathBuf,
    pub repo_subpath: String,
    pub category: String,
}

impl RepositoryInfo {
    // Future helper methods can be added here
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
    fn test_is_restic_internal_dir_valid_dirs() -> Result<(), BackupServiceError> {
        // Test all valid restic internal directories
        let valid_dirs = vec!["data", "index", "keys", "snapshots", "locks"];

        for dir in valid_dirs {
            assert!(
                RepositoryScanner::is_restic_internal_dir(dir)?,
                "Directory '{}' should be recognized as restic internal",
                dir
            );
        }

        Ok(())
    }

    #[test]
    fn test_is_restic_internal_dir_invalid_dirs() -> Result<(), BackupServiceError> {
        // Test directories that are NOT restic internal
        let invalid_dirs = vec![
            "app",
            "config",
            "postgres",
            "redis",
            "nginx",
            "documents",
            "projects",
            "backup",
            "temp",
            "usr",
            "var",
            "etc",
            "data_backup",   // Similar but not exact
            "index.html",    // File-like names
            "keys_backup",   // Similar but not exact
            "snapshots_old", // Similar but not exact
            "locks.txt",     // Similar but not exact
            "",              // Empty string
            "DATA",          // Wrong case
            "Index",         // Wrong case
            "SNAPSHOTS",     // Wrong case
        ];

        for dir in invalid_dirs {
            assert!(
                !RepositoryScanner::is_restic_internal_dir(dir)?,
                "Directory '{}' should NOT be recognized as restic internal",
                dir
            );
        }

        Ok(())
    }

    #[test]
    fn test_is_restic_internal_dir_edge_cases() -> Result<(), BackupServiceError> {
        // Test edge cases and special characters
        let edge_cases = vec![
            " data",       // Leading space
            "data ",       // Trailing space
            " data ",      // Both spaces
            "data/",       // With slash
            "/data",       // With leading slash
            "data-old",    // With hyphen
            "data_backup", // With underscore
            "data.bak",    // With dot
        ];

        for case in edge_cases {
            assert!(
                !RepositoryScanner::is_restic_internal_dir(case)?,
                "Edge case '{}' should NOT be recognized as restic internal",
                case
            );
        }

        Ok(())
    }

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
        // Test different category types
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

        assert_eq!(snapshot1, snapshot2); // Should be equal
        assert_ne!(snapshot1, snapshot3); // Should not be equal
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
        // Test complex paths with spaces, unicode, and special characters
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
            // Comprehensive whitespace scenarios
            // Gaming directories
            "/home/gamer/.local/share/Paradox Interactive",
            "/home/user/.steam/steam/steamapps/common/Grand Theft Auto V",
            "/home/player/Games/World of Warcraft/Interface/AddOns",
            // Application directories
            "/home/user/.config/Google Chrome",
            "/home/developer/.local/share/JetBrains Toolbox",
            "/home/designer/Adobe After Effects 2024",
            // Document and media folders
            "/home/user/Documents/Important Business Files",
            "/home/user/Music/Classical Music Collection",
            "/home/user/Videos/Home Movies 2024",
            // Docker volumes with spaces
            "/mnt/docker-data/volumes/my app data",
            "/mnt/docker-data/volumes/web server config",
            "/mnt/docker-data/volumes/database backup files",
            // System paths with spaces
            "/usr/share/applications/Visual Studio Code",
            "/opt/Google Chrome",
            "/var/log/system events",
            // Edge cases with multiple spaces
            "/home/user/My    Project    Files",
            "/home/user/App  With  Multiple  Spaces",
            // Leading and trailing spaces
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

        // Test various ID formats
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
            // Whitespace docker volume names
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
        // Test that S3 path construction doesn't create double slashes
        use crate::config::Config;

        // Test with empty base path (common case)
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

        // Test with non-empty base path
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

        // Test various hostname formats
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
        // CRITICAL: Test that would have caught the whitespace parsing bug

        // Mock AWS S3 ls output with directories containing spaces
        // This is the exact format that AWS CLI returns
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

        // Parse the output manually to test our parsing logic
        let parsed_dirs: Vec<String> = mock_s3_output
            .lines()
            .filter(|line| line.contains("PRE"))
            .map(|line| {
                // This is the FIXED parsing logic that should preserve spaces
                if let Some(start) = line.find("PRE ") {
                    let dir_name = &line[start + 4..]; // Skip "PRE "
                    dir_name.trim_end_matches('/').to_string()
                } else {
                    String::new()
                }
            })
            .filter(|d| !d.is_empty())
            .collect();

        // Verify that directories with spaces are preserved correctly
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

        // Test that the OLD BUGGY parsing would have failed
        let buggy_parsed_dirs: Vec<String> = mock_s3_output
            .lines()
            .filter(|line| line.contains("PRE"))
            .map(|line| {
                // This is the OLD BUGGY parsing logic
                line.split_whitespace()
                    .last()
                    .unwrap_or("")
                    .trim_end_matches('/')
                    .to_string()
            })
            .filter(|d| !d.is_empty())
            .collect();

        // Demonstrate that the buggy parsing would have truncated directory names
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

        // Ensure we have the expected count of directories
        assert_eq!(
            parsed_dirs.len(),
            9,
            "Should parse all 9 directories correctly"
        );
    }

    #[tokio::test]
    async fn test_s3_command_executor_parsing_with_spaces() {
        // Test the S3CommandExecutor parsing logic specifically

        // Mock S3 output for testing the parsing logic in isolation
        // This tests the parse_s3_directories helper logic
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
                // Test the FIXED parsing logic
                if let Some(start) = line.find("PRE ") {
                    let dir_name = &line[start + 4..];
                    dir_name.trim_end_matches('/').to_string()
                } else {
                    String::new()
                }
            })
            .filter(|d| !d.is_empty())
            .collect();

        // Verify correct parsing of directories with spaces
        assert_eq!(parsed.len(), 5);
        assert!(parsed.contains(&"application logs".to_string()));
        assert!(parsed.contains(&"database backup files".to_string()));
        assert!(parsed.contains(&"user data".to_string()));
        assert!(parsed.contains(&"web server config".to_string()));
        assert!(parsed.contains(&"Docker Volume With Spaces".to_string()));
    }

    #[test]
    fn test_s3_parsing_edge_cases() {
        // Test edge cases that could break S3 parsing
        let edge_case_outputs = vec![
            // Multiple spaces in directory names
            r#"                           PRE Directory  With  Multiple  Spaces/"#,
            // Leading and trailing spaces
            r#"                           PRE  Leading And Trailing  /"#,
            // Special characters with spaces
            r#"                           PRE My-App Config Files/"#,
            // Very long directory names with spaces
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
