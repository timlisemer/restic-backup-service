use crate::errors::BackupServiceError;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub restic_password: String,
    pub restic_repo_base: String,
    pub aws_access_key_id: String,
    pub aws_secret_access_key: String,
    pub aws_default_region: String,
    pub aws_s3_endpoint: String,
    pub backup_paths: Vec<PathBuf>,
    pub hostname: String,
}

impl Config {
    pub fn load() -> Result<Self, BackupServiceError> {
        dotenv::dotenv().ok();

        let restic_password = env::var("RESTIC_PASSWORD")?;
        let restic_repo_base = env::var("RESTIC_REPO_BASE")?;
        let aws_access_key_id = env::var("AWS_ACCESS_KEY_ID")?;
        let aws_secret_access_key = env::var("AWS_SECRET_ACCESS_KEY")?;

        let aws_default_region =
            env::var("AWS_DEFAULT_REGION").unwrap_or_else(|_| "auto".to_string());

        let aws_s3_endpoint = env::var("AWS_S3_ENDPOINT")?;

        // Parse backup paths from comma-separated list
        let backup_paths = env::var("BACKUP_PATHS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(s.trim()))
            .collect();

        // Get hostname from env or system
        let hostname = env::var("BACKUP_HOSTNAME").unwrap_or_else(|_| {
            hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        });

        Ok(Config {
            restic_password,
            restic_repo_base,
            aws_access_key_id,
            aws_secret_access_key,
            aws_default_region,
            aws_s3_endpoint,
            backup_paths,
            hostname,
        })
    }

    /// Get S3 endpoint URL from repo base
    pub fn s3_endpoint(&self) -> Result<String, BackupServiceError> {
        if let Some(endpoint) = self.restic_repo_base.strip_prefix("s3:") {
            if let Some(pos) = endpoint.find('/') {
                return Ok(endpoint[..pos].to_string());
            }
        }
        Ok(self.aws_s3_endpoint.clone())
    }

    /// Get S3 bucket name from repo base
    pub fn s3_bucket(&self) -> Result<String, BackupServiceError> {
        if let Some(s3_path) = self.restic_repo_base.strip_prefix("s3:") {
            // Remove protocol and extract bucket
            if let Some(path_start) = s3_path.find("//") {
                let path = &s3_path[path_start + 2..];
                if let Some(slash_pos) = path.find('/') {
                    let after_domain = &path[slash_pos + 1..];
                    if let Some(next_slash) = after_domain.find('/') {
                        return Ok(after_domain[..next_slash].to_string());
                    }
                    return Ok(after_domain.to_string());
                }
            }
        }
        Err(BackupServiceError::ConfigurationError(format!(
            "Could not extract bucket name from repo base: {}",
            self.restic_repo_base
        )))
    }

    /// Get the base path within the bucket (after bucket name)
    pub fn s3_base_path(&self) -> Result<String, BackupServiceError> {
        if let Some(s3_path) = self.restic_repo_base.strip_prefix("s3:") {
            if let Some(path_start) = s3_path.find("//") {
                let path = &s3_path[path_start + 2..];
                if let Some(slash_pos) = path.find('/') {
                    let after_domain = &path[slash_pos + 1..];
                    if let Some(next_slash) = after_domain.find('/') {
                        return Ok(after_domain[next_slash + 1..].to_string());
                    }
                }
            }
        }
        Ok(String::new())
    }

    /// Set AWS environment variables for restic
    pub fn set_aws_env(&self) -> Result<(), BackupServiceError> {
        env::set_var("AWS_ACCESS_KEY_ID", &self.aws_access_key_id);
        env::set_var("AWS_SECRET_ACCESS_KEY", &self.aws_secret_access_key);
        env::set_var("AWS_DEFAULT_REGION", &self.aws_default_region);
        env::set_var("RESTIC_PASSWORD", &self.restic_password);
        Ok(())
    }

    /// Get full repository URL for a specific path
    pub fn get_repo_url(&self, subpath: &str) -> Result<String, BackupServiceError> {
        Ok(format!(
            "{}/{}/{}",
            self.restic_repo_base, self.hostname, subpath
        ))
    }
}
