use std::process::Command;
use crate::config::Config;
use crate::errors::{BackupServiceError, Result};
use std::path::Path;
use tracing::{info, warn, error};



/// Validate credentials by testing basic S3 and restic connectivity
pub async fn validate_credentials(config: &Config) -> Result<()> {
    info!("🔑 Validating credentials...");

    // Test S3 connectivity by listing bucket root
    let s3_bucket = config.s3_bucket().map_err(|_| BackupServiceError::InvalidRepository)?;

    let output = Command::new("aws")
        .args([
            "s3", "ls", &format!("s3://{}/", s3_bucket),
            "--endpoint-url", &config.s3_endpoint(),
        ])
        .env("AWS_ACCESS_KEY_ID", &config.aws_access_key_id)
        .env("AWS_SECRET_ACCESS_KEY", &config.aws_secret_access_key)
        .env("AWS_DEFAULT_REGION", &config.aws_default_region)
        .output()
        .map_err(|_| BackupServiceError::CommandNotFound("Failed to execute aws".to_string()))?;

    if output.status.success() {
        info!("Credentials validated successfully");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = BackupServiceError::from_stderr(&stderr, "credential validation");

        // Use the Display implementation for consistent error messages
        error!(error = %error, "Credential validation failed");

        Err(error.with_validation_context())
    }
}

/// Show the size of a path in the repository
pub async fn show_size(config: Config, path: String) -> Result<()> {
    use crate::helpers::{PathMapper, ResticCommand};

    let native_path = Path::new(&path);
    let repo_subpath = PathMapper::path_to_repo_subpath(native_path)?;
    let repo_url = config.get_repo_url(&repo_subpath);
    let restic_cmd = ResticCommand::new(config, repo_url);

    info!(path = %path, "Checking size for path");

    // Check if path exists in snapshots
    let snapshots = restic_cmd.snapshots(Some(&path)).await?;

    if snapshots.is_empty() {
        warn!(path = %path, "No snapshots found for path");
        return Ok(());
    }

    // Get stats for the path
    let total_size = restic_cmd.stats(&path).await?;
    let size_str = format_bytes(total_size)?;
    info!(path = %path, size = %size_str, "Path size calculated");

    Ok(())
}

/// Format bytes to human readable format
pub fn format_bytes(bytes: u64) -> Result<String> {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    let formatted = if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    };

    Ok(formatted)
}

