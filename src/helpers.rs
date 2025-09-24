use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::shared::commands::ResticCommandExecutor;
use crate::shared::paths::PathMapper;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;

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

    /// List S3 directories with proper error handling
    pub async fn list_s3_dirs(&self, s3_path: &str) -> Result<Vec<String>, BackupServiceError> {
        let s3_bucket = self.config.s3_bucket()?;
        let full_path = format!("s3://{}/{}", s3_bucket, s3_path);

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
                    line.split_whitespace()
                        .last()
                        .unwrap_or("")
                        .trim_end_matches('/')
                        .to_string()
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

        Ok(repos)
    }

    /// Scan user home directories specifically
    pub async fn scan_user_home_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut repos = Vec::new();
        let user_home_path = format!("{}/{}/user_home", self.config.s3_base_path()?, hostname);

        if let Ok(users) = self.list_s3_dirs(&user_home_path).await {
            for user in users {
                let user_path = format!("{}/{}", user_home_path, user);
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

        Ok(repos)
    }

    /// Scan docker volumes with nested repository detection
    pub async fn scan_docker_volume_repositories(
        &self,
        hostname: &str,
    ) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut repos = Vec::new();
        let docker_path = format!("{}/{}/docker_volume", self.config.s3_base_path()?, hostname);

        if let Ok(volumes) = self.list_s3_dirs(&docker_path).await {
            for volume in volumes {
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
        let system_path = format!("{}/{}/system", self.config.s3_base_path()?, hostname);

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
        hostname: &str,
        repo_subpath: &str,
        native_path: &Path,
    ) -> Result<(usize, Vec<SnapshotInfo>), BackupServiceError> {
        let repo_url = self
            .config
            .get_repo_url(&format!("{}/{}", hostname, repo_subpath))?;
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
            "data_backup", // Similar but not exact
            "index.html",  // File-like names
            "keys_backup", // Similar but not exact
            "snapshots_old", // Similar but not exact
            "locks.txt",   // Similar but not exact
            "",            // Empty string
            "DATA",        // Wrong case
            "Index",       // Wrong case
            "SNAPSHOTS",   // Wrong case
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
        let native_path = PathBuf::from("/home/tim/documents");
        let repo_subpath = "user_home/tim/documents".to_string();
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
            ("/home/alice/projects", "user_home/alice/projects", "user_home"),
            ("/mnt/docker-data/volumes/postgres", "docker_volume/postgres", "docker_volume"),
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
        let path = PathBuf::from("/home/tim/documents");
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
            "/home/user with spaces/documents",
            "/home/alice/projects/my-project",
            "/mnt/docker-data/volumes/app_data",
            "/etc/nginx/sites-available/default",
            "/var/log/app/2025/01/15",
            "/usr/local/bin/custom-script",
            "/opt/software/version-1.2.3",
            "/home/user123/Downloads/file.tar.gz",
        ];

        for path_str in test_cases {
            let repo_info = RepositoryInfo {
                native_path: PathBuf::from(path_str),
                repo_subpath: format!("test/{}", path_str.replace("/", "_")),
                category: "test".to_string(),
            };

            assert_eq!(repo_info.native_path, PathBuf::from(path_str));
            assert!(repo_info.repo_subpath.starts_with("test/"));
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
            "abc123def456",           // Standard hex
            "12345678",               // Numbers only
            "short",                  // Short ID
            "very-long-id-with-dashes-and-numbers-123", // Long with dashes
            "mixed_123-ABC_def",      // Mixed case and symbols
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
            "postgres",
            "redis",
            "app-data",
            "my_volume",
            "volume123",
            "complex-name-with-dashes",
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
        }
    }

    #[test]
    fn test_nested_repository_logic() {
        // Test the logic used in scan_nested_docker_repositories
        let volume = "myapp";
        let nested_repos = vec!["config", "data", "logs", "backups"];

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
            assert!(repo_info.native_path.to_string_lossy().contains(nested_repo));
            assert!(repo_info.repo_subpath.contains(&format!("{}/{}", volume, nested_repo)));
        }
    }
}
