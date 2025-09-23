use anyhow::{Result, Context};
use colored::*;
use std::process::Command;
use crate::config::Config;
use crate::repository;
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


/// Run a command with environment variables
pub fn run_command_with_env(cmd: &str, args: &[&str], config: &Config) -> Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .env("AWS_ACCESS_KEY_ID", &config.aws_access_key_id)
        .env("AWS_SECRET_ACCESS_KEY", &config.aws_secret_access_key)
        .env("AWS_DEFAULT_REGION", &config.aws_default_region)
        .env("RESTIC_PASSWORD", &config.restic_password)
        .output()
        .context(format!("Failed to execute command: {}", cmd))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Don't bail on restic errors, just return stderr
        Ok(stderr.to_string())
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
pub async fn init_repo_if_needed(config: &Config, repo_url: &str) -> Result<()> {
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
pub async fn list_s3_dirs(config: &Config, s3_path: &str) -> Result<Vec<String>> {
    let full_path = format!("s3://{}/{}", config.s3_bucket()?, s3_path);

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