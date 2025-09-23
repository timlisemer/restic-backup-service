use anyhow::Result;
use crate::config::Config;
use crate::repository::BackupRepo;
use crate::helpers::{RepositoryScanner, SnapshotCollector, SnapshotInfo};
use crate::utils::{echo_info, echo_warning, echo_error, validate_credentials, BackupServiceError};
use std::collections::HashMap;
use serde_json::json;
use colored::Colorize;

pub async fn list_hosts(config: Config) -> Result<()> {
    echo_info("Getting available hosts...");
    config.set_aws_env();

    // Validate credentials before trying to list hosts
    if let Err(e) = validate_credentials(&config).await {
        echo_error("FAILED TO LIST HOSTS: Cannot access repository");
        return Err(anyhow::anyhow!("Credential validation failed: {}", e));
    }

    let scanner = RepositoryScanner::new(config);
    match scanner.get_hosts().await {
        Ok(hosts) => {
            if hosts.is_empty() {
                echo_warning("No hosts found in backup repository (repository is empty)");
            } else {
                println!("\nAvailable hosts:");
                for host in hosts {
                    println!("  - {}", host);
                }
            }
        }
        Err(e) => {
            echo_error("FAILED TO LIST HOSTS: Repository access error");
            echo_error(&format!("Error: {}", e));
            return Err(anyhow::anyhow!("Failed to list hosts: {}", e));
        }
    }

    Ok(())
}

pub async fn list_backups(config: Config, host: Option<String>, json_output: bool) -> Result<()> {
    let hostname = host.unwrap_or_else(|| config.hostname.clone());
    config.set_aws_env();

    if !json_output {
        echo_info(&format!("Listing backups for {} from S3 bucket...", hostname.bold()));
    }

    // Validate credentials before trying to list backups
    if let Err(e) = validate_credentials(&config).await {
        if json_output {
            let error_output = json!({
                "error": "authentication_failed",
                "message": format!("Cannot access repository: {}", e)
            });
            println!("{}", serde_json::to_string_pretty(&error_output)?);
        } else {
            echo_error("FAILED TO LIST BACKUPS: Cannot access repository");
        }
        return Err(anyhow::anyhow!("Credential validation failed: {}", e));
    }

    let scanner = RepositoryScanner::new(config.clone());
    let snapshot_collector = SnapshotCollector::new(config.clone());

    // Scan all repositories using the unified scanner
    let repo_infos = match scanner.scan_repositories(&hostname).await {
        Ok(repos) => repos,
        Err(BackupServiceError::AuthenticationFailed) => {
            echo_error("CRITICAL: Authentication failed while scanning repositories!");
            return Err(anyhow::anyhow!("Authentication failed during repository scan"));
        }
        Err(e) => {
            echo_error(&format!("Failed to scan repositories: {}", e));
            return Err(anyhow::anyhow!("Repository scan failed: {}", e));
        }
    };

    let mut repos: Vec<BackupRepo> = Vec::new();
    let mut all_snapshots: Vec<SnapshotInfo> = Vec::new();

    // Collect snapshots for each repository
    for repo_info in repo_infos {
        match snapshot_collector.get_snapshots(&hostname, &repo_info.repo_subpath, &repo_info.native_path).await {
            Ok((count, snapshots)) => {
                if count > 0 {
                    repos.push(BackupRepo::new(repo_info.native_path).with_count(count));
                    all_snapshots.extend(snapshots);
                }
            }
            Err(BackupServiceError::AuthenticationFailed) => {
                echo_error("CRITICAL: Authentication failed while collecting snapshots!");
                return Err(anyhow::anyhow!("Authentication failed during snapshot collection"));
            }
            Err(_) => {
                // Skip repositories that can't be accessed
                continue;
            }
        }
    }

    if json_output {
        // Return JSON format for scripting
        let output = json!({
            "host": hostname,
            "repositories": repos.iter().map(|r| json!({
                "path": r.native_path.to_string_lossy(),
                "category": r.category(),
                "snapshot_count": r.snapshot_count
            })).collect::<Vec<_>>(),
            "snapshots": all_snapshots.iter().map(|s| json!({
                "time": s.time.to_rfc3339(),
                "path": s.path.to_string_lossy(),
                "id": s.id
            })).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        // Display formatted output
        display_backup_summary(&repos, &all_snapshots);
    }

    Ok(())
}


fn display_backup_summary(repos: &[BackupRepo], snapshots: &[SnapshotInfo]) {
    println!("\n{}", "BACKUP PATHS SUMMARY:".bold());
    println!("{}", "====================".bold());

    // Group by category
    let mut categories: HashMap<&str, Vec<&BackupRepo>> = HashMap::new();
    for repo in repos {
        categories.entry(repo.category())
            .or_default()
            .push(repo);
    }

    // Display User Home
    let empty_vec = Vec::new();
    let user_repos = categories.get("user_home").unwrap_or(&empty_vec);
    println!("\n{} ({} paths):", "User Home".cyan(), user_repos.len());
    if user_repos.is_empty() {
        println!("  None");
    } else {
        for repo in user_repos {
            println!("  {:<50} - {} snapshots",
                repo.native_path.display(),
                repo.snapshot_count
            );
        }
    }

    // Display Docker Volumes
    let docker_repos = categories.get("docker_volume").unwrap_or(&empty_vec);
    println!("\n{} ({} paths):", "Docker Volumes".cyan(), docker_repos.len());
    if docker_repos.is_empty() {
        println!("  None");
    } else {
        for repo in docker_repos {
            println!("  {:<50} - {} snapshots",
                repo.native_path.display(),
                repo.snapshot_count
            );
        }
    }

    // Display System
    let system_repos = categories.get("system").unwrap_or(&empty_vec);
    println!("\n{} ({} paths):", "System".cyan(), system_repos.len());
    if system_repos.is_empty() {
        println!("  None");
    } else {
        for repo in system_repos {
            println!("  {:<50} - {} snapshots",
                repo.native_path.display(),
                repo.snapshot_count
            );
        }
    }

    // Display timeline
    println!("\n{}", "SNAPSHOT TIMELINE:".bold());
    println!("{}", "==================".bold());

    if snapshots.is_empty() {
        println!("No snapshots found");
    } else {
        // Group snapshots by minute
        let mut timeline: HashMap<String, Vec<&SnapshotInfo>> = HashMap::new();
        for snapshot in snapshots {
            let time_key = snapshot.time.format("%Y-%m-%d %H:%M").to_string();
            timeline.entry(time_key).or_default().push(snapshot);
        }

        // Sort and display
        let mut times: Vec<_> = timeline.keys().cloned().collect();
        times.sort();
        times.reverse();

        for time in times.iter().take(20) {
            if let Some(snaps) = timeline.get(time) {
                println!("\n{}:", time.green());
                for snap in snaps {
                    println!("  - {:<50} (id: {})",
                        snap.path.display(),
                        snap.id
                    );
                }
            }
        }

        if times.len() > 20 {
            println!("\n... and {} more time points", times.len() - 20);
        }
    }

    println!();
}

