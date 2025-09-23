use anyhow::Result;
use crate::config::Config;
use crate::repository::{BackupRepo, s3_to_native_path};
use crate::utils::{echo_info, echo_warning, list_s3_dirs, run_command_with_env, is_restic_internal_dir};
use std::collections::HashMap;
use std::path::PathBuf;
use serde_json::json;
use colored::Colorize;
use chrono::{DateTime, Utc};

pub async fn list_hosts(config: Config) -> Result<()> {
    echo_info("Getting available hosts...");
    config.set_aws_env();

    let base_path = config.s3_base_path();
    let hosts = list_s3_dirs(&config, &base_path).await?;

    if hosts.is_empty() {
        echo_warning("No hosts found in backup repository");
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
        echo_info(&format!("Listing backups for {} from S3 bucket...", hostname.bold()));
    }

    let mut repos: Vec<BackupRepo> = Vec::new();
    let mut all_snapshots: Vec<SnapshotInfo> = Vec::new();

    // Scan user home directories
    let user_home_path = format!("{}/{}/user_home", config.s3_base_path(), hostname);
    if let Ok(users) = list_s3_dirs(&config, &user_home_path).await {
        for user in users {
            let user_path = format!("{}/{}", user_home_path, user);
            if let Ok(subdirs) = list_s3_dirs(&config, &user_path).await {
                for subdir in subdirs {
                    let native_subdir = s3_to_native_path(&subdir);
                    let native_path = PathBuf::from(format!("/home/{}/{}", user, native_subdir));
                    let repo_subpath = format!("user_home/{}/{}", user, subdir);

                    let (count, snapshots) = get_snapshots(&config, &hostname, &repo_subpath, &native_path).await;
                    if count > 0 {
                        repos.push(BackupRepo::new(native_path).with_count(count));
                        all_snapshots.extend(snapshots);
                    }
                }
            }
        }
    }

    // Scan docker volumes
    let docker_path = format!("{}/{}/docker_volume", config.s3_base_path(), hostname);
    if let Ok(volumes) = list_s3_dirs(&config, &docker_path).await {
        for volume in volumes {
            let native_path = PathBuf::from(format!("/mnt/docker-data/volumes/{}", volume));
            let repo_subpath = format!("docker_volume/{}", volume);

            // Check for direct snapshots
            let (count, snapshots) = get_snapshots(&config, &hostname, &repo_subpath, &native_path).await;
            if count > 0 {
                repos.push(BackupRepo::new(native_path.clone()).with_count(count));
                all_snapshots.extend(snapshots);
            } else {
                // Check for nested repositories
                let volume_path = format!("{}/{}", docker_path, volume);
                if let Ok(nested) = list_s3_dirs(&config, &volume_path).await {
                    let real_nested: Vec<_> = nested.into_iter()
                        .filter(|d| !is_restic_internal_dir(d))
                        .collect();

                    for nested_repo in real_nested {
                        let nested_path = PathBuf::from(format!("/mnt/docker-data/volumes/{}/{}", volume, nested_repo));
                        let nested_repo_subpath = format!("docker_volume/{}/{}", volume, nested_repo);

                        let (nested_count, nested_snapshots) = get_snapshots(&config, &hostname, &nested_repo_subpath, &nested_path).await;
                        if nested_count > 0 {
                            repos.push(BackupRepo::new(nested_path).with_count(nested_count));
                            all_snapshots.extend(nested_snapshots);
                        }
                    }
                }
            }
        }
    }

    // Scan system paths
    let system_path = format!("{}/{}/system", config.s3_base_path(), hostname);
    if let Ok(paths) = list_s3_dirs(&config, &system_path).await {
        for path in paths {
            let native_path_str = s3_to_native_path(&path);
            let native_path = PathBuf::from(format!("/{}", native_path_str));
            let repo_subpath = format!("system/{}", path);

            let (count, snapshots) = get_snapshots(&config, &hostname, &repo_subpath, &native_path).await;
            if count > 0 {
                repos.push(BackupRepo::new(native_path).with_count(count));
                all_snapshots.extend(snapshots);
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

async fn get_snapshots(config: &Config, hostname: &str, repo_subpath: &str, native_path: &PathBuf) -> (usize, Vec<SnapshotInfo>) {
    let repo_url = format!("{}/{}/{}", config.restic_repo_base, hostname, repo_subpath);

    if let Ok(output) = run_command_with_env(
        "restic",
        &["--repo", &repo_url, "snapshots", "--json"],
        config,
    ) {
        if let Ok(snapshots) = serde_json::from_str::<Vec<serde_json::Value>>(&output) {
            let count = snapshots.len();
            let snapshot_infos: Vec<SnapshotInfo> = snapshots
                .into_iter()
                .filter_map(|s| {
                    let time = s["time"].as_str()?
                        .parse::<DateTime<Utc>>().ok()?;
                    let id = s["short_id"].as_str()?.to_string();
                    Some(SnapshotInfo {
                        time,
                        path: native_path.clone(),
                        id,
                    })
                })
                .collect();
            return (count, snapshot_infos);
        }
    }
    (0, Vec::new())
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

#[derive(Debug, Clone)]
struct SnapshotInfo {
    time: DateTime<Utc>,
    path: PathBuf,
    id: String,
}