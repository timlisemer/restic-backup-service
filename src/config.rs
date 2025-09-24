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
            // Find the first '/' after the protocol (after "://")
            if let Some(protocol_end) = endpoint.find("://") {
                let after_protocol = &endpoint[protocol_end + 3..];
                if let Some(path_start) = after_protocol.find('/') {
                    return Ok(endpoint[..protocol_end + 3 + path_start].to_string());
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config(repo_base: &str) -> Config {
        Config {
            restic_password: "test_password".to_string(),
            restic_repo_base: repo_base.to_string(),
            aws_access_key_id: "test_key".to_string(),
            aws_secret_access_key: "test_secret".to_string(),
            aws_default_region: "auto".to_string(),
            aws_s3_endpoint: "https://fallback.example.com".to_string(),
            backup_paths: vec![],
            hostname: "test-host".to_string(),
        }
    }

    #[test]
    fn test_s3_endpoint_extraction() -> Result<(), BackupServiceError> {
        // Test standard S3 URL format
        let config = create_test_config("s3:https://bucket.s3.amazonaws.com/restic");
        assert_eq!(config.s3_endpoint()?, "https://bucket.s3.amazonaws.com");

        // Test Cloudflare R2 format
        let config = create_test_config("s3:https://abc123.r2.cloudflarestorage.com/my-bucket/restic");
        assert_eq!(config.s3_endpoint()?, "https://abc123.r2.cloudflarestorage.com");

        // Test custom endpoint
        let config = create_test_config("s3:https://minio.example.com/bucket");
        assert_eq!(config.s3_endpoint()?, "https://minio.example.com");

        // Test HTTP
        let config = create_test_config("s3:http://localhost:9000/bucket");
        assert_eq!(config.s3_endpoint()?, "http://localhost:9000");

        // Test fallback when no s3: prefix
        let config = create_test_config("invalid_format");
        assert_eq!(config.s3_endpoint()?, "https://fallback.example.com");

        // Test fallback when no protocol or slash
        let config = create_test_config("s3:https-no-slashes");
        assert_eq!(config.s3_endpoint()?, "https://fallback.example.com");

        Ok(())
    }

    #[test]
    fn test_s3_bucket_extraction() -> Result<(), BackupServiceError> {
        // Test standard format with path
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/restic");
        assert_eq!(config.s3_bucket()?, "my-bucket");

        // Test bucket only
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket");
        assert_eq!(config.s3_bucket()?, "my-bucket");

        // Test with nested path
        let config = create_test_config("s3:https://minio.example.com/my-bucket/deep/path");
        assert_eq!(config.s3_bucket()?, "my-bucket");

        // Test Cloudflare R2 format
        let config = create_test_config("s3:https://abc123.r2.cloudflarestorage.com/bucket-name/restic");
        assert_eq!(config.s3_bucket()?, "bucket-name");

        // Test bucket with hyphens and numbers
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket-123/path");
        assert_eq!(config.s3_bucket()?, "my-bucket-123");

        Ok(())
    }

    #[test]
    fn test_s3_bucket_extraction_errors() {
        // Test invalid formats that should return errors
        let config = create_test_config("invalid_format");
        assert!(config.s3_bucket().is_err());

        let config = create_test_config("s3:invalid");
        assert!(config.s3_bucket().is_err());

        let config = create_test_config("s3:https://example.com");
        assert!(config.s3_bucket().is_err());
    }

    #[test]
    fn test_s3_base_path_extraction() -> Result<(), BackupServiceError> {
        // Test with base path
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/restic");
        assert_eq!(config.s3_base_path()?, "restic");

        // Test with nested base path
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/path/to/restic");
        assert_eq!(config.s3_base_path()?, "path/to/restic");

        // Test bucket only (no base path)
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket");
        assert_eq!(config.s3_base_path()?, "");

        // Test empty base path
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/");
        assert_eq!(config.s3_base_path()?, "");

        // Test invalid format returns empty
        let config = create_test_config("invalid_format");
        assert_eq!(config.s3_base_path()?, "");

        Ok(())
    }

    #[test]
    fn test_get_repo_url_construction() -> Result<(), BackupServiceError> {
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/restic");

        // Test basic URL construction
        assert_eq!(
            config.get_repo_url("user_home/tim/documents")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/user_home/tim/documents"
        );

        // Test docker volume path
        assert_eq!(
            config.get_repo_url("docker_volume/myapp")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/docker_volume/myapp"
        );

        // Test system path
        assert_eq!(
            config.get_repo_url("system/etc_nginx")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/system/etc_nginx"
        );

        // Test empty subpath
        assert_eq!(
            config.get_repo_url("")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/"
        );

        Ok(())
    }

    #[test]
    fn test_real_world_s3_urls() -> Result<(), BackupServiceError> {
        // Test actual Cloudflare R2 URL from .env
        let config = create_test_config("s3:https://0338e2011591dfc360150a909e7c2e1c.r2.cloudflarestorage.com/restic");
        assert_eq!(config.s3_endpoint()?, "https://0338e2011591dfc360150a909e7c2e1c.r2.cloudflarestorage.com");
        assert_eq!(config.s3_bucket()?, "restic");
        assert_eq!(config.s3_base_path()?, "");

        // Test AWS S3 standard URL
        let config = create_test_config("s3:https://s3.us-west-2.amazonaws.com/my-backup-bucket/restic-backups");
        assert_eq!(config.s3_endpoint()?, "https://s3.us-west-2.amazonaws.com");
        assert_eq!(config.s3_bucket()?, "my-backup-bucket");
        assert_eq!(config.s3_base_path()?, "restic-backups");

        // Test MinIO self-hosted
        let config = create_test_config("s3:https://minio.company.com/backups/restic/prod");
        assert_eq!(config.s3_endpoint()?, "https://minio.company.com");
        assert_eq!(config.s3_bucket()?, "backups");
        assert_eq!(config.s3_base_path()?, "restic/prod");

        Ok(())
    }

    #[test]
    fn test_edge_cases_and_malformed_urls() -> Result<(), BackupServiceError> {
        // Test URL with port number
        let config = create_test_config("s3:https://minio.example.com:9000/bucket/path");
        assert_eq!(config.s3_endpoint()?, "https://minio.example.com:9000");
        assert_eq!(config.s3_bucket()?, "bucket");
        assert_eq!(config.s3_base_path()?, "path");

        // Test HTTP instead of HTTPS
        let config = create_test_config("s3:http://localhost:9000/test-bucket");
        assert_eq!(config.s3_endpoint()?, "http://localhost:9000");
        assert_eq!(config.s3_bucket()?, "test-bucket");

        // Test with query parameters (should be ignored)
        let config = create_test_config("s3:https://s3.amazonaws.com/bucket/path?region=us-east-1");
        assert_eq!(config.s3_endpoint()?, "https://s3.amazonaws.com");
        assert_eq!(config.s3_bucket()?, "bucket");
        assert_eq!(config.s3_base_path()?, "path?region=us-east-1");

        Ok(())
    }
}
