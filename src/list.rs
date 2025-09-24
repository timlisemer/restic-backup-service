use crate::config::Config;
use crate::errors::Result;
use crate::helpers::{RepositoryScanner, SnapshotCollector, SnapshotInfo};
use crate::repository::BackupRepo;
use crate::utils::validate_credentials;
use serde_json::json;
use std::collections::HashMap;
use tracing::{info, warn};

pub async fn list_hosts(config: Config) -> Result<()> {
    info!("Getting available hosts...");
    config.set_aws_env();

    // Validate credentials before trying to list hosts
    validate_credentials(&config).await?;

    let scanner = RepositoryScanner::new(config);
    let hosts = scanner.get_hosts().await?;

    if hosts.is_empty() {
        warn!("No hosts found in backup repository (repository is empty)");
    } else {
        println!("\nAvailable hosts:");
        for host in hosts {
            println!("  - {}", host);
        }
    }

    Ok(())
}

pub async fn list_backups(config: Config, host: Option<String>, json_output: bool) -> Result<()> {
    let hostname = host.unwrap_or_else(|| config.hostname.clone());
    config.set_aws_env();

    if !json_output {
        info!(hostname = %hostname, "Listing backups from S3 bucket");
    }

    // Validate credentials before trying to list backups
    validate_credentials(&config).await?;

    let scanner = RepositoryScanner::new(config.clone());
    let snapshot_collector = SnapshotCollector::new(config.clone());

    // Scan all repositories using the unified scanner
    let repo_infos = scanner.scan_repositories(&hostname).await?;

    let mut repos: Vec<BackupRepo> = Vec::new();
    let mut all_snapshots: Vec<SnapshotInfo> = Vec::new();

    // Collect snapshots for each repository
    for repo_info in repo_infos {
        let (count, snapshots) = snapshot_collector
            .get_snapshots(&hostname, &repo_info.repo_subpath, &repo_info.native_path)
            .await?;
        if count > 0 {
            repos.push(BackupRepo::new(repo_info.native_path).with_count(count));
            all_snapshots.extend(snapshots);
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
        display_backup_summary(&repos, &all_snapshots)?;
    }

    Ok(())
}

fn display_backup_summary(
    repos: &[BackupRepo],
    snapshots: &[SnapshotInfo],
) -> crate::errors::Result<()> {
    println!("\nBACKUP PATHS SUMMARY:");
    println!("====================");

    // Group by category
    let mut categories: HashMap<&str, Vec<&BackupRepo>> = HashMap::new();
    for repo in repos {
        categories.entry(repo.category()).or_default().push(repo);
    }

    // Display User Home
    let empty_vec = Vec::new();
    let user_repos = categories.get("user_home").unwrap_or(&empty_vec);
    println!("\nUser Home ({} paths):", user_repos.len());
    if user_repos.is_empty() {
        println!("  None");
    } else {
        for repo in user_repos {
            println!(
                "  {:<50} - {} snapshots",
                repo.native_path.display(),
                repo.snapshot_count
            );
        }
    }

    // Display Docker Volumes
    let docker_repos = categories.get("docker_volume").unwrap_or(&empty_vec);
    println!("\nDocker Volumes ({} paths):", docker_repos.len());
    if docker_repos.is_empty() {
        println!("  None");
    } else {
        for repo in docker_repos {
            println!(
                "  {:<50} - {} snapshots",
                repo.native_path.display(),
                repo.snapshot_count
            );
        }
    }

    // Display System
    let system_repos = categories.get("system").unwrap_or(&empty_vec);
    println!("\nSystem ({} paths):", system_repos.len());
    if system_repos.is_empty() {
        println!("  None");
    } else {
        for repo in system_repos {
            println!(
                "  {:<50} - {} snapshots",
                repo.native_path.display(),
                repo.snapshot_count
            );
        }
    }

    // Display timeline
    println!("\nSNAPSHOT TIMELINE:");
    println!("==================");

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
                println!("\n{}:", time);
                for snap in snaps {
                    println!("  - {:<50} (id: {})", snap.path.display(), snap.id);
                }
            }
        }

        if times.len() > 20 {
            println!("\n... and {} more time points", times.len() - 20);
        }
    }

    println!();
    Ok(())
}
