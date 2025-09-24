use crate::config::Config;
use crate::errors::BackupServiceError;
use std::path::Path;
use std::process::Command;
use tracing::{error, info, warn};

/// Validate credentials by testing basic S3 and restic connectivity
pub async fn validate_credentials(config: &Config) -> Result<(), BackupServiceError> {
    info!("🔑 Validating credentials...");

    // Test S3 connectivity by listing bucket root
    let s3_bucket = config.s3_bucket()?;

    let output = Command::new("aws")
        .args([
            "s3",
            "ls",
            &format!("s3://{}/", s3_bucket),
            "--endpoint-url",
            &config.s3_endpoint()?,
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
pub async fn show_size(config: Config, path: String) -> Result<(), BackupServiceError> {
    use crate::shared::commands::ResticCommandExecutor;
    use crate::shared::paths::PathMapper;

    let native_path = Path::new(&path);
    let repo_subpath = PathMapper::path_to_repo_subpath(native_path)?;
    let repo_url = config.get_repo_url(&repo_subpath)?;
    let restic_cmd = ResticCommandExecutor::new(config, repo_url)?;

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
pub fn format_bytes(bytes: u64) -> Result<String, BackupServiceError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes_basic_units() -> Result<(), BackupServiceError> {
        // Test basic byte values
        assert_eq!(format_bytes(0)?, "0 B");
        assert_eq!(format_bytes(1)?, "1 B");
        assert_eq!(format_bytes(512)?, "512 B");
        assert_eq!(format_bytes(1023)?, "1023 B");
        Ok(())
    }

    #[test]
    fn test_format_bytes_kilobytes() -> Result<(), BackupServiceError> {
        // Test KB values
        assert_eq!(format_bytes(1024)?, "1.00 KB");
        assert_eq!(format_bytes(1536)?, "1.50 KB");
        assert_eq!(format_bytes(2048)?, "2.00 KB");
        assert_eq!(format_bytes(1048575)?, "1024.00 KB");  // 1MB - 1 byte = 1024 KB - 1/1024 KB ≈ 1024 KB
        Ok(())
    }

    #[test]
    fn test_format_bytes_megabytes() -> Result<(), BackupServiceError> {
        // Test MB values
        assert_eq!(format_bytes(1048576)?, "1.00 MB");
        assert_eq!(format_bytes(1572864)?, "1.50 MB");
        assert_eq!(format_bytes(10485760)?, "10.00 MB");
        assert_eq!(format_bytes(1073741823)?, "1024.00 MB");  // 1GB - 1 byte ≈ 1024 MB
        Ok(())
    }

    #[test]
    fn test_format_bytes_gigabytes() -> Result<(), BackupServiceError> {
        // Test GB values
        assert_eq!(format_bytes(1073741824)?, "1.00 GB");
        assert_eq!(format_bytes(2147483648)?, "2.00 GB");
        assert_eq!(format_bytes(5368709120)?, "5.00 GB");
        Ok(())
    }

    #[test]
    fn test_format_bytes_terabytes() -> Result<(), BackupServiceError> {
        // Test TB values (largest unit)
        assert_eq!(format_bytes(1099511627776)?, "1.00 TB");
        assert_eq!(format_bytes(2199023255552)?, "2.00 TB");

        // Test very large values that exceed TB scale
        assert_eq!(format_bytes(10995116277760)?, "10.00 TB");
        assert_eq!(format_bytes(u64::MAX)?, "16777216.00 TB");
        Ok(())
    }

    #[test]
    fn test_format_bytes_precision() -> Result<(), BackupServiceError> {
        // Test decimal precision
        assert_eq!(format_bytes(1024 + 102)?, "1.10 KB");
        assert_eq!(format_bytes(1024 + 205)?, "1.20 KB");
        assert_eq!(format_bytes(1048576 + 52428)?, "1.05 MB");
        assert_eq!(format_bytes(1073741824 + 107374182)?, "1.10 GB");
        Ok(())
    }

    #[test]
    fn test_format_bytes_edge_cases() -> Result<(), BackupServiceError> {
        // Test boundary conditions
        assert_eq!(format_bytes(1024 - 1)?, "1023 B");  // Just under KB
        assert_eq!(format_bytes(1024)?, "1.00 KB");     // Exactly 1 KB

        assert_eq!(format_bytes(1048576 - 1)?, "1024.00 KB");  // Just under MB (1024 KB - 1/1024 KB)
        assert_eq!(format_bytes(1048576)?, "1.00 MB");         // Exactly 1 MB

        assert_eq!(format_bytes(1073741824 - 1)?, "1024.00 MB");  // Just under GB (1024 MB - 1/1024 MB)
        assert_eq!(format_bytes(1073741824)?, "1.00 GB");         // Exactly 1 GB

        Ok(())
    }

    #[test]
    fn test_format_bytes_realistic_sizes() -> Result<(), BackupServiceError> {
        // Test common file/directory sizes
        assert_eq!(format_bytes(4096)?, "4.00 KB");        // Common page size
        assert_eq!(format_bytes(65536)?, "64.00 KB");      // Small file
        assert_eq!(format_bytes(1048576)?, "1.00 MB");     // Medium file
        assert_eq!(format_bytes(104857600)?, "100.00 MB"); // Large file
        assert_eq!(format_bytes(1073741824)?, "1.00 GB");  // Very large file/small disk
        assert_eq!(format_bytes(107374182400)?, "100.00 GB"); // Medium disk
        Ok(())
    }
}
