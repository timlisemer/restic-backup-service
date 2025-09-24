use crate::config::Config;
use crate::errors::BackupServiceError;
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tracing::{debug, info};

/// Unified command executor for AWS CLI and restic commands
pub struct CommandExecutor {
    config: Config,
}

/// Restic command wrapper using the unified executor
pub struct ResticCommandExecutor {
    executor: CommandExecutor,
    repo_url: String,
}

/// S3 command wrapper using the unified executor
pub struct S3CommandExecutor {
    executor: CommandExecutor,
}

impl CommandExecutor {
    pub fn new(config: Config) -> Result<Self, BackupServiceError> {
        Ok(Self { config })
    }

    /// Execute AWS S3 command with proper credentials and error handling
    pub async fn execute_aws_command(
        &self,
        args: &[&str],
        context: &str,
    ) -> Result<String, BackupServiceError> {
        debug!(args = ?args, context = %context, "Executing AWS command");

        let output = Command::new("aws")
            .args(args)
            .env("AWS_ACCESS_KEY_ID", &self.config.aws_access_key_id)
            .env("AWS_SECRET_ACCESS_KEY", &self.config.aws_secret_access_key)
            .env("AWS_DEFAULT_REGION", &self.config.aws_default_region)
            .output()
            .map_err(|_| {
                BackupServiceError::CommandNotFound("Failed to execute aws".to_string())
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(BackupServiceError::from_stderr(&stderr, context))
        }
    }

    /// Execute restic command with repository URL and proper environment
    pub async fn execute_restic_command(
        &self,
        repo_url: &str,
        args: &[&str],
        context: &str,
    ) -> Result<String, BackupServiceError> {
        debug!(repo_url = %repo_url, args = ?args, context = %context, "Executing restic command");

        let output = Command::new("restic")
            .args(["--repo", repo_url])
            .args(args)
            .env("AWS_ACCESS_KEY_ID", &self.config.aws_access_key_id)
            .env("AWS_SECRET_ACCESS_KEY", &self.config.aws_secret_access_key)
            .env("AWS_DEFAULT_REGION", &self.config.aws_default_region)
            .env("AWS_S3_ENDPOINT", &self.config.aws_s3_endpoint)
            .env("RESTIC_PASSWORD", &self.config.restic_password)
            .output()
            .map_err(|_| {
                BackupServiceError::CommandNotFound("Failed to execute restic".to_string())
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(BackupServiceError::from_stderr(&stderr, repo_url))
        }
    }

    /// Get S3 endpoint URL for AWS commands
    pub fn get_s3_endpoint_args(&self) -> Result<Vec<String>, BackupServiceError> {
        let endpoint = self.config.s3_endpoint()?;
        Ok(vec!["--endpoint-url".to_string(), endpoint])
    }
}

/// Helper function to check if restic repository exists
pub async fn check_restic_repository_exists(
    config: &Config,
    repo_url: &str,
) -> Result<bool, BackupServiceError> {
    let executor = CommandExecutor::new(config.clone())?;

    match executor
        .execute_restic_command(
            repo_url,
            &["snapshots", "--json"],
            "repository existence check",
        )
        .await
    {
        Ok(_) => Ok(true),
        Err(BackupServiceError::RepositoryNotFound(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

impl ResticCommandExecutor {
    pub fn new(config: Config, repo_url: String) -> Result<Self, BackupServiceError> {
        let executor = CommandExecutor::new(config)?;
        Ok(Self { executor, repo_url })
    }

    /// Initialize repository if needed
    pub async fn init_if_needed(&self) -> Result<(), BackupServiceError> {
        if !self.repo_exists().await? {
            info!(repo_url = %self.repo_url, "Initializing repository");
            self.executor
                .execute_restic_command(&self.repo_url, &["init"], "repository initialization")
                .await?;
            info!("Repository initialized");
        }
        Ok(())
    }

    /// Check if repository exists
    pub async fn repo_exists(&self) -> Result<bool, BackupServiceError> {
        check_restic_repository_exists(&self.executor.config, &self.repo_url).await
    }

    /// Run backup with exact parameters
    pub async fn backup(&self, path: &Path, hostname: &str) -> Result<String, BackupServiceError> {
        let path_str = path.to_string_lossy();
        let tag = determine_backup_tag(path)?;

        self.executor
            .execute_restic_command(
                &self.repo_url,
                &["backup", &path_str, "--host", hostname, "--tag", tag],
                &format!("backup {}", path_str),
            )
            .await
    }

    /// Get snapshots as JSON
    pub async fn snapshots(&self, path: Option<&str>) -> Result<Vec<Value>, BackupServiceError> {
        let mut args = vec!["snapshots", "--json"];
        if let Some(p) = path {
            args.extend(&["--path", p]);
        }

        let output = self
            .executor
            .execute_restic_command(&self.repo_url, &args, "snapshots listing")
            .await?;

        let snapshots: Vec<Value> = serde_json::from_str(&output).unwrap_or_default();
        Ok(snapshots)
    }

    /// Restore snapshot
    pub async fn restore(
        &self,
        snapshot_id: &str,
        path: &str,
        target: &str,
    ) -> Result<String, BackupServiceError> {
        self.executor
            .execute_restic_command(
                &self.repo_url,
                &["restore", snapshot_id, "--path", path, "--target", target],
                &format!("restore {} to {}", snapshot_id, target),
            )
            .await
    }

    /// Get repository stats
    pub async fn stats(&self, path: &str) -> Result<u64, BackupServiceError> {
        let output = self
            .executor
            .execute_restic_command(
                &self.repo_url,
                &[
                    "stats", "latest", "--mode", "raw-data", "--json", "--path", path,
                ],
                &format!("stats for {}", path),
            )
            .await?;

        if let Ok(stats) = serde_json::from_str::<Value>(&output) {
            if let Some(total_size) = stats["total_size"].as_u64() {
                return Ok(total_size);
            }
        }
        Ok(0)
    }
}

/// Determine backup tag based on path (extracted from PathMapper)
pub fn determine_backup_tag(path: &Path) -> Result<&'static str, BackupServiceError> {
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

impl S3CommandExecutor {
    pub fn new(config: Config) -> Result<Self, BackupServiceError> {
        let executor = CommandExecutor::new(config)?;
        Ok(Self { executor })
    }

    /// List S3 directories with proper error handling
    pub async fn list_directories(&self, s3_path: &str) -> Result<Vec<String>, BackupServiceError> {
        let s3_bucket = self.executor.config.s3_bucket()?;
        let full_path = format!("s3://{}/{}", s3_bucket, s3_path);

        let mut args = vec!["s3", "ls", &full_path];
        let endpoint_args = self.executor.get_s3_endpoint_args()?;
        args.extend(endpoint_args.iter().map(|s| s.as_str()));

        let output = self.executor.execute_aws_command(&args, &full_path).await?;

        let dirs: Vec<String> = output
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
    }

    /// Get available hosts from S3 bucket
    pub async fn get_hosts(&self) -> Result<Vec<String>, BackupServiceError> {
        let base_path = self.executor.config.s3_base_path()?;
        self.list_directories(&base_path).await
    }
}
