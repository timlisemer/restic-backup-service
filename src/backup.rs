use crate::config::Config;
use crate::errors::Result;
use crate::helpers::{PathMapper, ResticCommand};
use crate::utils::validate_credentials;
use std::path::{Path, PathBuf};
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{info, warn, error};

pub async fn run_backup(config: Config, additional_paths: Vec<String>) -> Result<()> {
    let hostname = &config.hostname.clone();
    info!(hostname = %hostname, "Starting backup process");

    config.set_aws_env();

    // Validate credentials before doing any backup work
    validate_credentials(&config).await?;

    // Collect all paths to backup
    let mut all_paths: Vec<PathBuf> = config.backup_paths.clone();

    // Add any additional paths from command line
    for path in additional_paths {
        all_paths.push(PathBuf::from(path));
    }

    // Add docker volumes if directory exists
    let docker_volumes_path = Path::new("/mnt/docker-data/volumes");
    if docker_volumes_path.exists() {
        info!("Detecting docker volumes...");
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
        warn!("No paths configured for backup. Use BACKUP_PATHS in .env or specify paths via command line.");
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
            warn!(path = %path.display(), "Path does not exist, skipping");
            skip_count += 1;
            continue;
        }

        let repo_subpath = PathMapper::path_to_repo_subpath(path)?;
        let repo_url = config.get_repo_url(&repo_subpath);
        let restic_cmd = ResticCommand::new(config.clone(), repo_url);

        // Initialize repository if needed
        restic_cmd.init_if_needed().await?;

        // Run backup using ResticCommand helper
        let output = restic_cmd.backup(path, hostname).await?;

        if output.contains("snapshot") && output.contains("saved") {
            // Extract snapshot ID
            let snapshot_id = output
                .lines()
                .find(|line| line.contains("snapshot") && line.contains("saved"))
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("unknown");

            if output.contains("at least one source file could not be read") {
                warn!(
                    path = %path.display(),
                    snapshot_id = %snapshot_id,
                    "Backed up with some files skipped due to I/O errors"
                );
            } else {
                info!(
                    path = %path.display(),
                    snapshot_id = %snapshot_id,
                    "Backup completed"
                );
            }
            success_count += 1;
        } else {
            warn!(path = %path.display(), "Failed to backup");
            skip_count += 1;
        }
    }

    pb.finish_and_clear();

    if success_count == 0 && skip_count > 0 {
        error!(
            success_count = %success_count,
            skip_count = %skip_count,
            "BACKUP FAILED: No data was backed up! Please check the errors above"
        );
    } else if skip_count > 0 {
        warn!(
            success_count = %success_count,
            skip_count = %skip_count,
            "Backup partially completed"
        );
    } else {
        info!(
            success_count = %success_count,
            "Backup completed successfully"
        );
    }

    Ok(())
}

