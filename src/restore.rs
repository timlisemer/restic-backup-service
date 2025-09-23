use crate::config::Config;
use crate::helpers::{RepositoryScanner, ResticCommand};
use crate::utils::{
    echo_error, echo_info, echo_success, echo_warning, validate_credentials, BackupServiceError,
};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use colored::Colorize;
use dialoguer::{Confirm, MultiSelect, Select};
use std::fs;
use std::path::{Path, PathBuf};

pub async fn restore_interactive(
    config: Config,
    host_opt: Option<String>,
    path_opt: Option<String>,
    timestamp_opt: Option<String>,
) -> Result<()> {
    config.set_aws_env();

    echo_info("Restic Interactive Restore Tool");
    println!("{}", "===============================".bold());
    println!();

    // Validate credentials before starting restore process
    if let Err(e) = validate_credentials(&config).await {
        echo_error("RESTORE ABORTED: Cannot access repository");
        return Err(anyhow::anyhow!("Credential validation failed: {}", e));
    }

    // Phase 1: Host selection
    let selected_host = if let Some(host) = host_opt {
        host
    } else {
        let hosts = get_available_hosts(&config).await?;
        if hosts.is_empty() {
            echo_error("No hosts found in backup repository");
            return Ok(());
        }

        let current_host = config.hostname.clone();
        let default = hosts.iter().position(|h| h == &current_host).unwrap_or(0);

        let selection = Select::new()
            .with_prompt("Select hostname")
            .items(&hosts)
            .default(default)
            .interact()?;

        hosts[selection].clone()
    };

    echo_info(&format!("Selected host: {}", selected_host.bold()));

    // Phase 2: Get backup data
    echo_info(&format!("Querying backups for {}...", selected_host));
    let backup_data = collect_backup_data(&config, &selected_host).await?;

    if backup_data.is_empty() {
        echo_error(&format!("No backups found for host {}", selected_host));
        return Ok(());
    }

    // Phase 3: Repository selection
    let selected_repos = if let Some(path) = path_opt {
        // Find matching repository
        backup_data
            .iter()
            .filter(|r| r.path.to_string_lossy() == path)
            .cloned()
            .collect()
    } else {
        let categories = vec![
            "All (everything)",
            "User Home (all user directories)",
            "Docker Volumes (all docker volumes)",
            "System (all system paths)",
            "Custom Selection (choose specific repositories)",
            "Individual Repository (single selection)",
        ];

        let selection = Select::new()
            .with_prompt("Select what to restore")
            .items(&categories)
            .interact()?;

        match selection {
            0 => backup_data.clone(), // All
            1 => backup_data
                .iter()
                .filter(|r| r.category == "user_home")
                .cloned()
                .collect(),
            2 => backup_data
                .iter()
                .filter(|r| r.category == "docker_volume")
                .cloned()
                .collect(),
            3 => backup_data
                .iter()
                .filter(|r| r.category == "system")
                .cloned()
                .collect(),
            4 => {
                // Custom multi-selection
                let items: Vec<String> = backup_data
                    .iter()
                    .map(|r| format!("{} ({} snapshots)", r.path.display(), r.snapshots.len()))
                    .collect();

                let selections = MultiSelect::new()
                    .with_prompt("Select repositories (space to toggle, enter to confirm)")
                    .items(&items)
                    .interact()?;

                selections
                    .into_iter()
                    .map(|i| backup_data[i].clone())
                    .collect()
            }
            5 => {
                // Single selection
                let items: Vec<String> = backup_data
                    .iter()
                    .map(|r| format!("{} ({} snapshots)", r.path.display(), r.snapshots.len()))
                    .collect();

                let selection = Select::new()
                    .with_prompt("Select repository")
                    .items(&items)
                    .interact()?;

                vec![backup_data[selection].clone()]
            }
            _ => vec![],
        }
    };

    if selected_repos.is_empty() {
        echo_error("No repositories selected");
        return Ok(());
    }

    echo_info(&format!(
        "Selected {} repositories for restoration",
        selected_repos.len()
    ));

    // Phase 4: Timestamp selection
    let selected_timestamp = if let Some(ts) = timestamp_opt {
        ts.parse::<DateTime<Utc>>()?
    } else {
        // Collect all unique timestamps from selected repos
        let mut all_timestamps: Vec<DateTime<Utc>> = selected_repos
            .iter()
            .flat_map(|r| &r.snapshots)
            .map(|s| s.time)
            .collect();
        all_timestamps.sort();
        all_timestamps.reverse();
        all_timestamps.dedup();

        if all_timestamps.is_empty() {
            echo_error("No snapshots found for selected repositories");
            return Ok(());
        }

        // Group into 5-minute windows
        let mut time_windows = Vec::new();
        let mut window_times = Vec::new();

        for ts in &all_timestamps {
            let window_start = ts.timestamp() - (ts.timestamp() % 300);
            let window_time = DateTime::<Utc>::from_timestamp(window_start, 0).unwrap();

            if !window_times.contains(&window_time) {
                let window_end = window_time + Duration::minutes(5);
                let count = all_timestamps
                    .iter()
                    .filter(|t| **t >= window_time && **t < window_end)
                    .count();

                let label = format!(
                    "{} to {} ({} snapshots)",
                    window_time.format("%Y-%m-%d %H:%M"),
                    window_end.format("%H:%M"),
                    count
                );

                time_windows.push(label);
                window_times.push(window_time);
            }
        }

        let selection = Select::new()
            .with_prompt("Select time window")
            .items(&time_windows)
            .default(0)
            .interact()?;

        window_times[selection]
    };

    echo_info(&format!(
        "Selected time window: {}",
        selected_timestamp
            .format("%Y-%m-%d %H:%M")
            .to_string()
            .bold()
    ));

    // Phase 5: Restoration
    let dest_dir = PathBuf::from("/tmp/restic/interactive");

    // Check if destination exists
    if dest_dir.exists() {
        if fs::read_dir(&dest_dir)?.next().is_some() {
            echo_warning(&format!(
                "Destination directory {} is not empty",
                dest_dir.display()
            ));

            if !Confirm::new()
                .with_prompt("Continue and clear the directory?")
                .default(false)
                .interact()?
            {
                echo_error("Operation cancelled by user");
                return Ok(());
            }
        }
        fs::remove_dir_all(&dest_dir)?;
    }
    fs::create_dir_all(&dest_dir)?;

    echo_info(&format!(
        "Restoring to: {}",
        dest_dir.display().to_string().bold()
    ));

    let mut restored_count = 0;
    let mut skipped_count = 0;

    for repo in &selected_repos {
        echo_info(&format!(
            "Restoring {} from {}",
            repo.path.display().to_string().bold(),
            repo.repo_subpath.bold()
        ));

        let repo_url = config.get_repo_url(&repo.repo_subpath);

        // Find best snapshot within time window
        let window_end = selected_timestamp + Duration::minutes(5);
        let best_snapshot = repo
            .snapshots
            .iter()
            .filter(|s| s.time >= selected_timestamp && s.time < window_end)
            .max_by_key(|s| s.time)
            .or_else(|| {
                // If none in window, find newest before window
                repo.snapshots
                    .iter()
                    .filter(|s| s.time < selected_timestamp)
                    .max_by_key(|s| s.time)
            });

        if let Some(snapshot) = best_snapshot {
            let restic_cmd = ResticCommand::new(config.clone(), repo_url);
            let result = restic_cmd
                .restore(
                    &snapshot.id,
                    &repo.path.to_string_lossy(),
                    &dest_dir.to_string_lossy(),
                )
                .await;

            match result {
                Ok(_) => {
                    echo_success(&format!(
                        "Restored {} from snapshot {} at {}",
                        repo.path.display(),
                        snapshot.id,
                        snapshot.time.format("%Y-%m-%d %H:%M:%S")
                    ));
                    restored_count += 1;
                }
                Err(BackupServiceError::AuthenticationFailed) => {
                    echo_error("CRITICAL ERROR: Authentication failed during restore!");
                    echo_error("Your credentials are invalid or access was denied.");
                    echo_error("RESTORE ABORTED - Cannot continue without proper authentication.");
                    return Err(anyhow::anyhow!("Authentication failed during restore"));
                }
                Err(BackupServiceError::NetworkError) => {
                    echo_error("CRITICAL ERROR: Network connection failed during restore!");
                    echo_error("Cannot connect to repository endpoint.");
                    echo_error("RESTORE ABORTED - Check your network connection and endpoint configuration.");
                    return Err(anyhow::anyhow!("Network error during restore"));
                }
                Err(e) => {
                    echo_error(&format!(
                        "RESTORE FAILED for {}: {}",
                        repo.path.display(),
                        e
                    ));
                    echo_warning("Continuing with remaining repositories...");
                    skipped_count += 1;
                }
            }
        } else {
            echo_warning(&format!(
                "No suitable snapshots found for {}",
                repo.path.display()
            ));
            skipped_count += 1;
        }
    }

    println!();
    echo_info("Restoration Summary:");
    echo_info(&format!(
        "  Successfully restored: {} repositories",
        restored_count
    ));
    echo_info(&format!("  Skipped: {} repositories", skipped_count));
    echo_info(&format!(
        "  Destination: {}",
        dest_dir.display().to_string().bold()
    ));

    if restored_count > 0 {
        echo_success("Restoration completed successfully!");
        echo_info(&format!(
            "You can now access your restored files at {}",
            dest_dir.display()
        ));

        // Offer to move/copy files
        println!();
        let actions = vec![
            "Copy to original location (replace existing files)",
            "Move to original location (replace existing files)",
            "Leave files in temporary location",
        ];

        let selection = Select::new()
            .with_prompt("What would you like to do with the restored files?")
            .items(&actions)
            .default(2)
            .interact()?;

        match selection {
            0 => {
                echo_info("Copying files to original locations...");
                // Implement copy logic
                for repo in &selected_repos {
                    let src = dest_dir.join(repo.path.strip_prefix("/").unwrap_or(&repo.path));
                    if src.exists() {
                        let parent = repo.path.parent().unwrap_or(Path::new("/"));
                        fs::create_dir_all(parent)?;

                        // Use system cp command for simplicity
                        let result = std::process::Command::new("cp")
                            .args(["-rf", &src.to_string_lossy(), &parent.to_string_lossy()])
                            .output()?;

                        if result.status.success() {
                            echo_success(&format!("Copied {}", repo.path.display()));
                        } else {
                            echo_error(&format!("Failed to copy {}", repo.path.display()));
                        }
                    }
                }
            }
            1 => {
                echo_info("Moving files to original locations...");
                // Similar to copy but with mv command
                for repo in &selected_repos {
                    let src = dest_dir.join(repo.path.strip_prefix("/").unwrap_or(&repo.path));
                    if src.exists() {
                        if repo.path.exists() {
                            fs::remove_dir_all(&repo.path)?;
                        }
                        let parent = repo.path.parent().unwrap_or(Path::new("/"));
                        fs::create_dir_all(parent)?;
                        fs::rename(&src, &repo.path)?;
                        echo_success(&format!("Moved {}", repo.path.display()));
                    }
                }
                fs::remove_dir_all(&dest_dir).ok();
            }
            2 => {
                echo_info(&format!(
                    "Files remain at temporary location: {}",
                    dest_dir.display()
                ));
            }
            _ => {
                echo_info(&format!(
                    "Files remain at temporary location: {}",
                    dest_dir.display()
                ));
            }
        }
    }

    Ok(())
}

async fn get_available_hosts(config: &Config) -> Result<Vec<String>> {
    let scanner = RepositoryScanner::new(config.clone());
    scanner
        .get_hosts()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get available hosts: {}", e))
}

#[derive(Debug, Clone)]
struct RestoreRepo {
    path: PathBuf,
    repo_subpath: String,
    category: String,
    snapshots: Vec<RestoreSnapshot>,
}

#[derive(Debug, Clone)]
struct RestoreSnapshot {
    id: String,
    time: DateTime<Utc>,
}

async fn collect_backup_data(config: &Config, hostname: &str) -> Result<Vec<RestoreRepo>> {
    let scanner = RepositoryScanner::new(config.clone());

    // Get all repositories using the unified scanner
    let repo_infos = match scanner.scan_repositories(hostname).await {
        Ok(repos) => repos,
        Err(BackupServiceError::AuthenticationFailed) => {
            echo_error("CRITICAL: Authentication failed while scanning repositories!");
            return Err(anyhow::anyhow!(
                "Authentication failed during backup data collection"
            ));
        }
        Err(e) => {
            echo_error(&format!("Failed to scan repositories: {}", e));
            return Err(anyhow::anyhow!("Repository scan failed: {}", e));
        }
    };

    let mut repos = Vec::new();

    // Get snapshots for each repository
    for repo_info in repo_infos {
        match get_repo_snapshots(config, hostname, &repo_info.repo_subpath).await {
            Ok(Some(snapshots)) => {
                repos.push(RestoreRepo {
                    path: repo_info.native_path,
                    repo_subpath: repo_info.repo_subpath,
                    category: repo_info.category,
                    snapshots,
                });
            }
            Ok(None) => {
                // No snapshots in this repository - skip it
            }
            Err(BackupServiceError::AuthenticationFailed) => {
                echo_error("CRITICAL: Authentication failed while collecting snapshots!");
                return Err(anyhow::anyhow!(
                    "Authentication failed during snapshot collection"
                ));
            }
            Err(_) => {
                // Skip repositories that can't be accessed
                continue;
            }
        }
    }

    Ok(repos)
}

async fn get_repo_snapshots(
    config: &Config,
    hostname: &str,
    repo_subpath: &str,
) -> Result<Option<Vec<RestoreSnapshot>>, BackupServiceError> {
    let repo_url = format!("{}/{}/{}", config.restic_repo_base, hostname, repo_subpath);
    let restic_cmd = ResticCommand::new(config.clone(), repo_url);

    let snapshots = restic_cmd.snapshots(None).await?;

    let snapshot_list: Vec<RestoreSnapshot> = snapshots
        .into_iter()
        .filter_map(|s| {
            let time = s["time"].as_str()?.parse::<DateTime<Utc>>().ok()?;
            let id = s["short_id"].as_str()?.to_string();
            Some(RestoreSnapshot { id, time })
        })
        .collect();

    if !snapshot_list.is_empty() {
        Ok(Some(snapshot_list))
    } else {
        Ok(None)
    }
}
