use colored::*;
use std::process::Command;
use crate::config::Config;
use crate::errors::{BackupServiceError, Result};
use std::path::Path;

// Color coded output helpers
pub fn echo_info(msg: &str) {
    println!("{} {}", "[INFO]".blue().bold(), msg);
}

pub fn echo_success(msg: &str) {
    println!("{} {}", "[SUCCESS]".green().bold(), msg);
}

pub fn echo_error(msg: &str) {
    eprintln!("{} {}", "[ERROR]".red().bold(), msg);
}

pub fn echo_warning(msg: &str) {
    println!("{} {}", "[WARNING]".yellow().bold(), msg);
}



/// Validate credentials by testing basic S3 and restic connectivity
pub async fn validate_credentials(config: &Config) -> Result<()> {
    echo_info("Validating credentials...");

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
        echo_success("Credentials validated successfully");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = BackupServiceError::from_stderr(&stderr, "credential validation");

        // Display detailed error messages based on error type
        match &error {
            BackupServiceError::AuthenticationFailed => {
                echo_error("CREDENTIAL VALIDATION FAILED!");
                echo_error("Your AWS credentials are invalid or access is denied.");
                echo_error("Please check your .env file and verify:");
                echo_error("  - AWS_ACCESS_KEY_ID is correct");
                echo_error("  - AWS_SECRET_ACCESS_KEY is correct");
                echo_error("  - AWS_S3_ENDPOINT is correct");
                echo_error("  - Your credentials have access to the S3 bucket");
            }
            BackupServiceError::NetworkError => {
                echo_error("NETWORK CONNECTION FAILED!");
                echo_error("Cannot connect to your S3 endpoint.");
                echo_error("Please check:");
                echo_error("  - Your internet connection");
                echo_error("  - AWS_S3_ENDPOINT URL is correct and reachable");
            }
            _ => {
                echo_error("REPOSITORY ACCESS FAILED!");
                echo_error(&format!("Error: {}", stderr));
            }
        }

        Err(error.with_validation_context())
    }
}

/// Show the size of a path in the repository
pub async fn show_size(config: Config, path: String) -> Result<()> {
    use crate::helpers::{PathMapper, ResticCommand};

    let native_path = Path::new(&path);
    let repo_subpath = PathMapper::path_to_repo_subpath(native_path);
    let repo_url = config.get_repo_url(&repo_subpath);
    let restic_cmd = ResticCommand::new(config, repo_url);

    echo_info(&format!("Checking size for path: {}", path.bold()));

    // Check if path exists in snapshots
    let snapshots = restic_cmd.snapshots(Some(&path)).await?;

    if snapshots.is_empty() {
        echo_warning(&format!("No snapshots found for path: {}", path));
        return Ok(());
    }

    // Get stats for the path
    let total_size = restic_cmd.stats(&path).await?;
    let size_str = format_bytes(total_size);
    echo_success(&format!("{}: {}", path, size_str.bold()));

    Ok(())
}

/// Format bytes to human readable format
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

