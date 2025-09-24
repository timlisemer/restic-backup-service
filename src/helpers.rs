use crate::config::Config;
use crate::errors::BackupServiceError;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

// ============================================================================
// PathMapper - Centralized path conversion logic
// ============================================================================

pub struct PathMapper;

impl PathMapper {
    /// Convert native filesystem path to repository subpath (exact NixOS getLocation logic)
    pub fn path_to_repo_subpath(path: &Path) -> Result<String, BackupServiceError> {
        let path_str = path.to_string_lossy();

        let result = if let Some(stripped) = path_str.strip_prefix("/home/") {
            let parts: Vec<&str> = stripped.split('/').collect();
            if parts.is_empty() {
                "user_home".to_string()
            } else {
                let username = parts[0];
                if parts.len() == 1 {
                    format!("user_home/{}", username)
                } else {
                    let subdir = parts[1..].join("_");
                    format!("user_home/{}/{}", username, subdir)
                }
            }
        } else if let Some(stripped) = path_str.strip_prefix("/mnt/docker-data/volumes/") {
            let volume_path = stripped;
            if volume_path.is_empty() {
                "docker_volume".to_string()
            } else {
                format!("docker_volume/{}", volume_path.replace('/', "_"))
            }
        } else {
            let system_path = path_str.trim_start_matches('/');
            if system_path.is_empty() {
                "system".to_string()
            } else {
                format!("system/{}", system_path.replace('/', "_"))
            }
        };

        Ok(result)
    }

    /// Convert S3 directory name back to native path (preserve filename underscores)
    pub fn s3_to_native_path(s3_dir: &str) -> Result<String, BackupServiceError> {
        let result = if s3_dir.matches('_').count() > 1 {
            s3_dir.replace('_', "/")
        } else {
            s3_dir.to_string()
        };
        Ok(result)
    }

    /// Determine backup tag based on path (exact NixOS logic)
    pub fn determine_tag(path: &Path) -> Result<&'static str, BackupServiceError> {
        let path_str = path.to_string_lossy();
        let tag = if path_str.starts_with("/home/") {
            "user-path"
        } else if path_str.starts_with("/mnt/docker-data/volumes/") {
            "docker-volume"
        } else {
            "system-path"
        };
        Ok(tag)
    }
}

// ============================================================================
// ResticCommand - Unified interface for all restic operations
// ============================================================================

pub struct ResticCommand {
    repo_url: String,
    config: Config,
}

impl ResticCommand {
    pub fn new(config: Config, repo_url: String) -> Result<Self, BackupServiceError> {
        Ok(Self { repo_url, config })
    }

    /// Initialize repository if needed
    pub async fn init_if_needed(&self) -> Result<(), BackupServiceError> {
        if !self.repo_exists().await? {
            info!(repo_url = %self.repo_url, "Initializing repository");
            self.run_command(&["init"]).await?;
            info!("Repository initialized");
        }
        Ok(())
    }

    /// Check if repository exists
    pub async fn repo_exists(&self) -> Result<bool, BackupServiceError> {
        let result = self.run_command(&["snapshots", "--json"]).await.is_ok();
        Ok(result)
    }

    /// Run backup with exact NixOS parameters
    pub async fn backup(&self, path: &Path, hostname: &str) -> Result<String, BackupServiceError> {
        let path_str = path.to_string_lossy();
        let tag = PathMapper::determine_tag(path)?;

        let output = self
            .run_command(&["backup", &path_str, "--host", hostname, "--tag", tag])
            .await?;

        Ok(output)
    }

    /// Get snapshots as JSON
    pub async fn snapshots(&self, path: Option<&str>) -> Result<Vec<Value>, BackupServiceError> {
        let mut args = vec!["snapshots", "--json"];
        if let Some(p) = path {
            args.extend(&["--path", p]);
        }

        let output = self.run_command(&args).await?;
        let snapshots: Vec<Value> = serde_json::from_str(&output).unwrap_or_default();
        Ok(snapshots)
    }

    /// Restore snapshot
    pub async fn restore(&self, snapshot_id: &str, path: &str, target: &str) -> Result<String, BackupServiceError> {
        self.run_command(&["restore", snapshot_id, "--path", path, "--target", target])
            .await
    }

    /// Get repository stats
    pub async fn stats(&self, path: &str) -> Result<u64, BackupServiceError> {
        let output = self
            .run_command(&[
                "stats", "latest", "--mode", "raw-data", "--json", "--path", path,
            ])
            .await?;

        if let Ok(stats) = serde_json::from_str::<Value>(&output) {
            if let Some(total_size) = stats["total_size"].as_u64() {
                return Ok(total_size);
            }
        }
        Ok(0)
    }

    /// Core command execution with exact NixOS environment setup
    async fn run_command(&self, args: &[&str]) -> Result<String, BackupServiceError> {
        let output = Command::new("restic")
            .args(["--repo", &self.repo_url])
            .args(args)
            .env("AWS_ACCESS_KEY_ID", &self.config.aws_access_key_id)
            .env("AWS_SECRET_ACCESS_KEY", &self.config.aws_secret_access_key)
            .env("AWS_DEFAULT_REGION", &self.config.aws_default_region)
            .env("RESTIC_PASSWORD", &self.config.restic_password)
            .output()
            .map_err(|_| {
                BackupServiceError::CommandNotFound("Failed to execute restic".to_string())
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(BackupServiceError::from_stderr(&stderr, &self.repo_url))
        }
    }
}

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
        let s3_bucket = self
            .config
            .s3_bucket()
            ?;
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
        Ok(matches!(dir_name, "data" | "index" | "keys" | "snapshots" | "locks"))
    }

    /// Get available hosts from S3 bucket
    pub async fn get_hosts(&self) -> Result<Vec<String>, BackupServiceError> {
        let base_path = self.config.s3_base_path()?;
        self.list_s3_dirs(&base_path).await
    }

    /// Scan and collect all repositories for a hostname
    pub async fn scan_repositories(&self, hostname: &str) -> Result<Vec<RepositoryInfo>, BackupServiceError> {
        let mut repos = Vec::new();

        // Scan user home directories
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

        // Scan docker volumes with nested repository detection
        let docker_path = format!("{}/{}/docker_volume", self.config.s3_base_path()?, hostname);
        if let Ok(volumes) = self.list_s3_dirs(&docker_path).await {
            for volume in volumes {
                let native_path = PathBuf::from(format!("/mnt/docker-data/volumes/{}", volume));
                let repo_subpath = format!("docker_volume/{}", volume);

                repos.push(RepositoryInfo {
                    native_path: native_path.clone(),
                    repo_subpath: repo_subpath.clone(),
                    category: "docker_volume".to_string(),
                });

                // Check for nested repositories
                let volume_path = format!("{}/{}", docker_path, volume);
                if let Ok(nested) = self.list_s3_dirs(&volume_path).await {
                    for nested_repo in nested {
                        if !Self::is_restic_internal_dir(&nested_repo)? {
                            let nested_path = PathBuf::from(format!(
                                "/mnt/docker-data/volumes/{}/{}",
                                volume, nested_repo
                            ));
                            let nested_repo_subpath =
                                format!("docker_volume/{}/{}", volume, nested_repo);

                            repos.push(RepositoryInfo {
                                native_path: nested_path,
                                repo_subpath: nested_repo_subpath,
                                category: "docker_volume".to_string(),
                            });
                        }
                    }
                }
            }
        }

        // Scan system paths
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
        let restic_cmd = ResticCommand::new(self.config.clone(), repo_url)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_path_to_repo_subpath() -> Result<(), BackupServiceError> {
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/home/tim"))?,
            "user_home/tim"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/home/tim/documents"))?,
            "user_home/tim/documents"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/home/tim/my/deep/path"))?,
            "user_home/tim/my_deep_path"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/mnt/docker-data/volumes/myapp"))?,
            "docker_volume/myapp"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/etc/nginx"))?,
            "system/etc_nginx"
        );
        Ok(())
    }
}
