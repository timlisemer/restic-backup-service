use anyhow::Result;
use colored::*;
use std::process::Command;
use crate::config::Config;
use crate::repository;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BackupServiceError {
    #[error("Authentication failed: Invalid credentials or access denied")]
    AuthenticationFailed,
    #[error("Network error: Cannot connect to repository")]
    NetworkError,
    #[error("Repository not found: {0}")]
    RepositoryNotFound(String),
    #[error("Command execution failed: {0}")]
    CommandFailed(String),
    #[error("Invalid repository path or configuration")]
    InvalidRepository,
}

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


/// Run a command with environment variables - now with proper error handling
pub fn run_command_with_env(cmd: &str, args: &[&str], config: &Config) -> Result<String, BackupServiceError> {
    let output = Command::new(cmd)
        .args(args)
        .env("AWS_ACCESS_KEY_ID", &config.aws_access_key_id)
        .env("AWS_SECRET_ACCESS_KEY", &config.aws_secret_access_key)
        .env("AWS_DEFAULT_REGION", &config.aws_default_region)
        .env("RESTIC_PASSWORD", &config.restic_password)
        .output()
        .map_err(|_| BackupServiceError::CommandFailed(format!("Failed to execute {}", cmd)))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();

        // Parse stderr to determine error type
        if stderr.contains("access denied") || stderr.contains("invalid credentials") ||
           stderr.contains("authorization") || stderr.contains("forbidden") ||
           stderr.contains("access key") || stderr.contains("secret key") {
            Err(BackupServiceError::AuthenticationFailed)
        } else if stderr.contains("network") || stderr.contains("connection") ||
                  stderr.contains("timeout") || stderr.contains("unreachable") ||
                  stderr.contains("dns") {
            Err(BackupServiceError::NetworkError)
        } else if stderr.contains("repository") && stderr.contains("not found") {
            Err(BackupServiceError::RepositoryNotFound(args.iter().find(|s| s.contains("s3:")).unwrap_or(&"").to_string()))
        } else {
            let full_stderr = String::from_utf8_lossy(&output.stderr);
            Err(BackupServiceError::CommandFailed(full_stderr.to_string()))
        }
    }
}

/// Check if a restic repository exists
pub async fn repo_exists(config: &Config, repo_url: &str) -> bool {
    let result = run_command_with_env(
        "restic",
        &["--repo", repo_url, "snapshots", "--json"],
        config,
    );
    result.is_ok()
}

/// Initialize a restic repository if it doesn't exist
pub async fn init_repo_if_needed(config: &Config, repo_url: &str) -> Result<(), BackupServiceError> {
    if !repo_exists(config, repo_url).await {
        echo_info(&format!("Initializing repository: {}", repo_url));
        run_command_with_env(
            "restic",
            &["--repo", repo_url, "init"],
            config,
        )?;
        echo_success("Repository initialized");
    }
    Ok(())
}

/// Validate credentials by testing basic S3 and restic connectivity
pub async fn validate_credentials(config: &Config) -> Result<(), BackupServiceError> {
    echo_info("Validating credentials...");

    // Test S3 connectivity by listing bucket root
    let s3_bucket = config.s3_bucket().map_err(|_| BackupServiceError::InvalidRepository)?;
    let test_result = run_command_with_env(
        "aws",
        &[
            "s3",
            "ls",
            &format!("s3://{}/", s3_bucket),
            "--endpoint-url", &config.s3_endpoint(),
        ],
        config,
    );

    match test_result {
        Ok(_) => {
            echo_success("Credentials validated successfully");
            Ok(())
        }
        Err(BackupServiceError::AuthenticationFailed) => {
            echo_error("CREDENTIAL VALIDATION FAILED!");
            echo_error("Your AWS credentials are invalid or access is denied.");
            echo_error("Please check your .env file and verify:");
            echo_error("  - AWS_ACCESS_KEY_ID is correct");
            echo_error("  - AWS_SECRET_ACCESS_KEY is correct");
            echo_error("  - AWS_S3_ENDPOINT is correct");
            echo_error("  - Your credentials have access to the S3 bucket");
            Err(BackupServiceError::AuthenticationFailed)
        }
        Err(BackupServiceError::NetworkError) => {
            echo_error("NETWORK CONNECTION FAILED!");
            echo_error("Cannot connect to your S3 endpoint.");
            echo_error("Please check:");
            echo_error("  - Your internet connection");
            echo_error("  - AWS_S3_ENDPOINT URL is correct and reachable");
            Err(BackupServiceError::NetworkError)
        }
        Err(e) => {
            echo_error("REPOSITORY ACCESS FAILED!");
            echo_error(&format!("Error: {}", e));
            Err(e)
        }
    }
}

/// Show the size of a path in the repository
pub async fn show_size(config: Config, path: String) -> Result<()> {
    let native_path = Path::new(&path);
    let repo_subpath = repository::path_to_repo_subpath(native_path);
    let repo_url = config.get_repo_url(&repo_subpath);

    echo_info(&format!("Checking size for path: {}", path.bold()));

    // Check if path exists in snapshots
    let snapshots = run_command_with_env(
        "restic",
        &[
            "--repo", &repo_url,
            "snapshots",
            "--json",
            "--path", &path,
        ],
        &config,
    )?;

    let snapshots: Vec<serde_json::Value> = serde_json::from_str(&snapshots)
        .unwrap_or_default();

    if snapshots.is_empty() {
        echo_warning(&format!("No snapshots found for path: {}", path));
        return Ok(());
    }

    // Get stats for the path
    let stats = run_command_with_env(
        "restic",
        &[
            "--repo", &repo_url,
            "stats",
            "latest",
            "--mode", "raw-data",
            "--json",
            "--path", &path,
        ],
        &config,
    )?;

    if let Ok(stats_json) = serde_json::from_str::<serde_json::Value>(&stats) {
        if let Some(total_size) = stats_json["total_size"].as_u64() {
            let size_str = format_bytes(total_size);
            echo_success(&format!("{}: {}", path, size_str.bold()));
        }
    }

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

/// List S3 directories
pub async fn list_s3_dirs(config: &Config, s3_path: &str) -> Result<Vec<String>, BackupServiceError> {
    let full_path = format!("s3://{}/{}", config.s3_bucket().map_err(|_| BackupServiceError::InvalidRepository)?, s3_path);

    let output = run_command_with_env(
        "aws",
        &[
            "s3",
            "ls",
            &full_path,
            "--endpoint-url", &config.s3_endpoint(),
        ],
        config,
    )?;

    let dirs: Vec<String> = output
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
}

/// Check if S3 path contains a restic repository structure
pub fn is_restic_internal_dir(dir_name: &str) -> bool {
    matches!(dir_name, "data" | "index" | "keys" | "snapshots" | "locks")
}