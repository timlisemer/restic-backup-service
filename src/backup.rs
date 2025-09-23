use anyhow::Result;
use crate::config::Config;
use crate::helpers::{PathMapper, ResticCommand};
use crate::utils::{echo_info, echo_success, echo_warning, echo_error, validate_credentials, BackupServiceError};
use std::path::{Path, PathBuf};
use indicatif::{ProgressBar, ProgressStyle};
use colored::Colorize;

pub async fn run_backup(config: Config, additional_paths: Vec<String>) -> Result<()> {
    let hostname = &config.hostname.clone();
    echo_info(&format!("Starting backup process for host: {}", hostname.bold()));

    config.set_aws_env();

    // Validate credentials before doing any backup work
    if let Err(e) = validate_credentials(&config).await {
        echo_error("BACKUP ABORTED: Cannot proceed without valid credentials");
        return Err(anyhow::anyhow!("Credential validation failed: {}", e));
    }

    // Collect all paths to backup
    let mut all_paths: Vec<PathBuf> = config.backup_paths.clone();

    // Add any additional paths from command line
    for path in additional_paths {
        all_paths.push(PathBuf::from(path));
    }

    // Add docker volumes if directory exists
    let docker_volumes_path = Path::new("/mnt/docker-data/volumes");
    if docker_volumes_path.exists() {
        echo_info("Detecting docker volumes...");
        if let Ok(entries) = std::fs::read_dir(docker_volumes_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    // Skip non-volume entries
                    if name != "backingFsBlockDev" && name != "metadata.db" {
                        all_paths.push(path);
                    }
                }
            }
        }
    }

    if all_paths.is_empty() {
        echo_warning("No paths configured for backup. Use BACKUP_PATHS in .env or specify paths via command line.");
        return Ok(());
    }

    let total = all_paths.len();
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")?
            .progress_chars("#>-")
    );

    let mut success_count = 0;
    let mut skip_count = 0;

    for (idx, path) in all_paths.iter().enumerate() {
        pb.set_position(idx as u64);
        pb.set_message(format!("Backing up: {}", path.display()));

        // Check if path exists
        if !path.exists() {
            echo_warning(&format!("Path does not exist, skipping: {}", path.display()));
            skip_count += 1;
            continue;
        }

        let repo_subpath = PathMapper::path_to_repo_subpath(path);
        let repo_url = config.get_repo_url(&repo_subpath);
        let restic_cmd = ResticCommand::new(config.clone(), repo_url);

        // Initialize repository if needed
        if let Err(e) = restic_cmd.init_if_needed().await {
            match e {
                BackupServiceError::AuthenticationFailed => {
                    echo_error("CRITICAL ERROR: Authentication failed during repository initialization!");
                    echo_error("Your credentials are invalid or have insufficient permissions.");
                    echo_error("BACKUP ABORTED - Cannot continue without proper access.");
                    return Err(anyhow::anyhow!("Authentication failed"));
                }
                BackupServiceError::NetworkError => {
                    echo_error("CRITICAL ERROR: Network connection failed!");
                    echo_error("Cannot connect to repository endpoint.");
                    echo_error("BACKUP ABORTED - Check your network and endpoint configuration.");
                    return Err(anyhow::anyhow!("Network error"));
                }
                _ => {
                    echo_warning(&format!("Failed to initialize repository for {}: {}", path.display(), e));
                    skip_count += 1;
                    continue;
                }
            }
        }

        // Run backup using ResticCommand helper
        let backup_result = restic_cmd.backup(path, hostname).await;

        match backup_result {
            Ok(output) => {
                if output.contains("snapshot") && output.contains("saved") {
                    // Extract snapshot ID
                    let snapshot_id = output
                        .lines()
                        .find(|line| line.contains("snapshot") && line.contains("saved"))
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("unknown");

                    if output.contains("at least one source file could not be read") {
                        echo_warning(&format!(
                            "Backed up: {} (snapshot {}) - some files skipped due to I/O errors",
                            path.display(), snapshot_id
                        ));
                    } else {
                        echo_success(&format!(
                            "Backed up: {} (snapshot {})",
                            path.display(), snapshot_id
                        ));
                    }
                    success_count += 1;
                } else {
                    echo_warning(&format!("Failed to backup: {}", path.display()));
                    skip_count += 1;
                }
            }
            Err(BackupServiceError::AuthenticationFailed) => {
                echo_error("CRITICAL ERROR: Authentication failed during backup!");
                echo_error("Your credentials are invalid or access was denied.");
                echo_error("BACKUP ABORTED - Cannot continue without proper authentication.");
                return Err(anyhow::anyhow!("Authentication failed during backup"));
            }
            Err(BackupServiceError::NetworkError) => {
                echo_error("CRITICAL ERROR: Network connection failed during backup!");
                echo_error("Cannot connect to repository endpoint.");
                echo_error("BACKUP ABORTED - Check your network connection and endpoint configuration.");
                return Err(anyhow::anyhow!("Network error during backup"));
            }
            Err(e) => {
                echo_error(&format!("BACKUP FAILED for {}: {}", path.display(), e));
                echo_warning("Continuing with remaining paths...");
                skip_count += 1;
            }
        }
    }

    pb.finish_and_clear();

    if success_count == 0 && skip_count > 0 {
        echo_error(&format!(
            "BACKUP FAILED: 0 successful, {} failed/skipped",
            skip_count
        ));
        echo_error("No data was backed up! Please check the errors above.");
    } else if skip_count > 0 {
        echo_warning(&format!(
            "Backup partially completed: {} successful, {} failed/skipped",
            success_count, skip_count
        ));
    } else {
        echo_success(&format!(
            "Backup completed successfully: {} paths backed up",
            success_count
        ));
    }

    Ok(())
}

