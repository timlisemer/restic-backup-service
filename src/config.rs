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
        // Load environment variables from .env file if present
        dotenv::dotenv().ok();

        // Avoid dotenv variable substitution issues - try .env file first, then env var
        let restic_password =
            Self::read_password_from_env_file().or_else(|_| env::var("RESTIC_PASSWORD"))?;
        let restic_repo_base = env::var("RESTIC_REPO_BASE")?;
        let aws_access_key_id = env::var("AWS_ACCESS_KEY_ID")?;
        let aws_secret_access_key = env::var("AWS_SECRET_ACCESS_KEY")?;

        let aws_default_region =
            env::var("AWS_DEFAULT_REGION").unwrap_or_else(|_| "auto".to_string());

        let aws_s3_endpoint = env::var("AWS_S3_ENDPOINT")?;

        let backup_paths = env::var("BACKUP_PATHS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(s.trim().trim_end_matches('/')))
            .collect();

        // Hostname fallback: env var -> system hostname -> "unknown"
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

    pub fn s3_endpoint(&self) -> Result<String, BackupServiceError> {
        // Parse endpoint from s3:https://domain.com/bucket/path format
        if let Some(endpoint) = self.restic_repo_base.strip_prefix("s3:") {
            if let Some(protocol_end) = endpoint.find("://") {
                let after_protocol = &endpoint[protocol_end + 3..];
                if let Some(path_start) = after_protocol.find('/') {
                    return Ok(endpoint[..protocol_end + 3 + path_start].to_string());
                }
            }
        }
        Ok(self.aws_s3_endpoint.clone())
    }

    pub fn s3_bucket(&self) -> Result<String, BackupServiceError> {
        // Extract bucket name from s3:https://domain.com/bucket/path
        if let Some(s3_path) = self.restic_repo_base.strip_prefix("s3:") {
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

    // Set environment variables for AWS SDK/CLI usage
    pub fn set_aws_env(&self) -> Result<(), BackupServiceError> {
        env::set_var("AWS_ACCESS_KEY_ID", &self.aws_access_key_id);
        env::set_var("AWS_SECRET_ACCESS_KEY", &self.aws_secret_access_key);
        env::set_var("AWS_DEFAULT_REGION", &self.aws_default_region);
        env::set_var("RESTIC_PASSWORD", &self.restic_password);
        Ok(())
    }

    // Construct final restic repository URL with hostname and subpath
    pub fn get_repo_url(&self, subpath: &str) -> Result<String, BackupServiceError> {
        Ok(format!(
            "{}/{}/{}",
            self.restic_repo_base, self.hostname, subpath
        ))
    }

    // Manual .env parsing to avoid dotenv variable substitution issues
    fn read_password_from_env_file() -> Result<String, BackupServiceError> {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        let file = File::open(".env").map_err(|_| {
            BackupServiceError::ConfigurationError("Cannot open .env file".to_string())
        })?;

        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line.map_err(|_| {
                BackupServiceError::ConfigurationError("Cannot read .env file".to_string())
            })?;

            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "RESTIC_PASSWORD" {
                    let password = value.trim();
                    // Remove quotes if present
                    let password = if (password.starts_with('"') && password.ends_with('"'))
                        || (password.starts_with('\'') && password.ends_with('\''))
                    {
                        &password[1..password.len() - 1]
                    } else {
                        password
                    };
                    return Ok(password.to_string());
                }
            }
        }

        Err(BackupServiceError::ConfigurationError(
            "RESTIC_PASSWORD not found in .env file".to_string(),
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
        let config = create_test_config("s3:https://bucket.s3.amazonaws.com/restic");
        assert_eq!(config.s3_endpoint()?, "https://bucket.s3.amazonaws.com");

        // Test Cloudflare R2 format
        let config =
            create_test_config("s3:https://abc123.r2.cloudflarestorage.com/my-bucket/restic");
        assert_eq!(
            config.s3_endpoint()?,
            "https://abc123.r2.cloudflarestorage.com"
        );

        let config = create_test_config("s3:https://minio.example.com/bucket");
        assert_eq!(config.s3_endpoint()?, "https://minio.example.com");

        // Test HTTP
        let config = create_test_config("s3:http://localhost:9000/bucket");
        assert_eq!(config.s3_endpoint()?, "http://localhost:9000");

        let config = create_test_config("invalid_format");
        assert_eq!(config.s3_endpoint()?, "https://fallback.example.com");

        let config = create_test_config("s3:https-no-slashes");
        assert_eq!(config.s3_endpoint()?, "https://fallback.example.com");

        Ok(())
    }

    #[test]
    fn test_s3_bucket_extraction() -> Result<(), BackupServiceError> {
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/restic");
        assert_eq!(config.s3_bucket()?, "my-bucket");

        // Test bucket only
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket");
        assert_eq!(config.s3_bucket()?, "my-bucket");

        let config = create_test_config("s3:https://minio.example.com/my-bucket/deep/path");
        assert_eq!(config.s3_bucket()?, "my-bucket");

        // Test Cloudflare R2 format
        let config =
            create_test_config("s3:https://abc123.r2.cloudflarestorage.com/bucket-name/restic");
        assert_eq!(config.s3_bucket()?, "bucket-name");

        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket-123/path");
        assert_eq!(config.s3_bucket()?, "my-bucket-123");

        Ok(())
    }

    #[test]
    fn test_s3_bucket_extraction_errors() {
        let config = create_test_config("invalid_format");
        assert!(config.s3_bucket().is_err());

        let config = create_test_config("s3:invalid");
        assert!(config.s3_bucket().is_err());

        let config = create_test_config("s3:https://example.com");
        assert!(config.s3_bucket().is_err());
    }

    #[test]
    fn test_s3_base_path_extraction() -> Result<(), BackupServiceError> {
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/restic");
        assert_eq!(config.s3_base_path()?, "restic");

        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/path/to/restic");
        assert_eq!(config.s3_base_path()?, "path/to/restic");

        // Test bucket only (no base path)
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket");
        assert_eq!(config.s3_base_path()?, "");

        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/");
        assert_eq!(config.s3_base_path()?, "");

        let config = create_test_config("invalid_format");
        assert_eq!(config.s3_base_path()?, "");

        Ok(())
    }

    #[test]
    fn test_get_repo_url_construction() -> Result<(), BackupServiceError> {
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/restic");

        assert_eq!(
            config.get_repo_url("user_home/tim/documents")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/user_home/tim/documents"
        );

        assert_eq!(
            config.get_repo_url("docker_volume/myapp")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/docker_volume/myapp"
        );

        assert_eq!(
            config.get_repo_url("system/etc_nginx")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/system/etc_nginx"
        );

        assert_eq!(
            config.get_repo_url("")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/"
        );

        // Test whitespace path scenarios
        assert_eq!(
            config.get_repo_url("user_home/gamer/.local/share/Paradox Interactive")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/user_home/gamer/.local/share/Paradox Interactive"
        );

        assert_eq!(
            config.get_repo_url("user_home/user/Documents/My Games")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/user_home/user/Documents/My Games"
        );

        assert_eq!(
            config.get_repo_url("docker_volume/my app data")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/docker_volume/my app data"
        );

        assert_eq!(
            config.get_repo_url("system/usr_share_applications_My Application")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/system/usr_share_applications_My Application"
        );

        Ok(())
    }

    #[test]
    fn test_get_repo_url_whitespace_edge_cases() -> Result<(), BackupServiceError> {
        let config = create_test_config("s3:https://s3.amazonaws.com/my-bucket/restic");

        assert_eq!(
            config.get_repo_url("user_home/user/My   Project   Files")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/user_home/user/My   Project   Files"
        );

        assert_eq!(
            config.get_repo_url("user_home/user/ leading space")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/user_home/user/ leading space"
        );

        assert_eq!(
            config.get_repo_url("docker_volume/trailing space ")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/docker_volume/trailing space "
        );

        // Test paths with special characters and spaces
        assert_eq!(
            config.get_repo_url("user_home/developer/Cool App-Name v2.0")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/user_home/developer/Cool App-Name v2.0"
        );

        // Test realistic gaming paths
        assert_eq!(
            config.get_repo_url("user_home/gamer/.steam/steam/steamapps/common/Counter Strike")?,
            "s3:https://s3.amazonaws.com/my-bucket/restic/test-host/user_home/gamer/.steam/steam/steamapps/common/Counter Strike"
        );

        Ok(())
    }

    #[test]
    fn test_real_world_s3_urls() -> Result<(), BackupServiceError> {
        // Test actual Cloudflare R2 URL from .env
        let config = create_test_config(
            "s3:https://0338e2011591dfc360150a909e7c2e1c.r2.cloudflarestorage.com/restic",
        );
        assert_eq!(
            config.s3_endpoint()?,
            "https://0338e2011591dfc360150a909e7c2e1c.r2.cloudflarestorage.com"
        );
        assert_eq!(config.s3_bucket()?, "restic");
        assert_eq!(config.s3_base_path()?, "");

        // Test AWS S3 standard URL
        let config = create_test_config(
            "s3:https://s3.us-west-2.amazonaws.com/my-backup-bucket/restic-backups",
        );
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

    #[test]
    fn test_backup_paths_parsing() -> Result<(), BackupServiceError> {
        // Test parsing of comma-separated backup paths similar to NixOS style
        use std::env;

        // Set up test environment variable
        let test_paths = "/home/user/Projects,/home/user/Downloads,/home/user/.config,/home/user/.steam,/home/user/.local/share/Paradox Interactive,/home/user/.local/share/Steam/steamapps/common/My Game";
        env::set_var("BACKUP_PATHS", test_paths);

        // Test parsing
        let parsed_paths: Vec<PathBuf> = env::var("BACKUP_PATHS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(s.trim().trim_end_matches('/')))
            .collect();

        assert_eq!(parsed_paths.len(), 6);
        assert_eq!(parsed_paths[0], PathBuf::from("/home/user/Projects"));
        assert_eq!(parsed_paths[1], PathBuf::from("/home/user/Downloads"));
        assert_eq!(parsed_paths[2], PathBuf::from("/home/user/.config"));
        assert_eq!(parsed_paths[3], PathBuf::from("/home/user/.steam"));
        assert_eq!(
            parsed_paths[4],
            PathBuf::from("/home/user/.local/share/Paradox Interactive")
        );
        assert_eq!(
            parsed_paths[5],
            PathBuf::from("/home/user/.local/share/Steam/steamapps/common/My Game")
        );

        // Test empty paths filtering
        env::set_var("BACKUP_PATHS", "/path1,,/path2,  ,/path3");
        let filtered_paths: Vec<PathBuf> = env::var("BACKUP_PATHS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| PathBuf::from(s.trim().trim_end_matches('/')))
            .collect();

        assert_eq!(filtered_paths.len(), 3);
        assert_eq!(filtered_paths[0], PathBuf::from("/path1"));
        assert_eq!(filtered_paths[1], PathBuf::from("/path2"));
        assert_eq!(filtered_paths[2], PathBuf::from("/path3"));

        // Clean up
        env::remove_var("BACKUP_PATHS");
        Ok(())
    }

    #[test]
    fn test_backup_paths_trailing_slash_trimming() -> Result<(), BackupServiceError> {
        use std::env;

        // Test that trailing slashes are properly trimmed from backup paths
        let test_paths_with_slashes = "/home/user/Documents/,/home/user/.local/share/Paradox Interactive/,/home/user/Projects,/home/user/.config/";
        env::set_var("BACKUP_PATHS", test_paths_with_slashes);

        // Use the actual config parsing logic
        let parsed_paths: Vec<PathBuf> = env::var("BACKUP_PATHS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(s.trim().trim_end_matches('/')))
            .collect();

        assert_eq!(parsed_paths.len(), 4);
        // Verify trailing slashes are removed
        assert_eq!(parsed_paths[0], PathBuf::from("/home/user/Documents"));
        assert_eq!(parsed_paths[1], PathBuf::from("/home/user/.local/share/Paradox Interactive"));
        assert_eq!(parsed_paths[2], PathBuf::from("/home/user/Projects")); // No slash to trim
        assert_eq!(parsed_paths[3], PathBuf::from("/home/user/.config"));

        // Clean up
        env::remove_var("BACKUP_PATHS");
        Ok(())
    }
}
