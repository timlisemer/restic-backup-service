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

#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub time: DateTime<Utc>,
    pub path: PathBuf,
    pub id: String,
}
