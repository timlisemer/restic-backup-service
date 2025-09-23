use anyhow::Result;
use crate::config::Config;
use crate::repository::{BackupRepo, s3_to_native_path};
use crate::utils::{echo_info, echo_warning, echo_error, list_s3_dirs, run_command_with_env, is_restic_internal_dir, validate_credentials, BackupServiceError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde_json::json;
use colored::Colorize;
use chrono::{DateTime, Utc};

pub async fn list_hosts(config: Config) -> Result<()> {
    echo_info("Getting available hosts...");
    config.set_aws_env();

    // Validate credentials before trying to list hosts
    if let Err(e) = validate_credentials(&config).await {
        echo_error("FAILED TO LIST HOSTS: Cannot access repository");
        return Err(anyhow::anyhow!("Credential validation failed: {}", e));
    }

    let base_path = config.s3_base_path();

    match list_s3_dirs(&config, &base_path).await {
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

                    match get_snapshots(&config, &hostname, &repo_subpath, &native_path).await {
                        Ok((count, snapshots)) => {
                            if count > 0 {
                                repos.push(BackupRepo::new(native_path).with_count(count));
                                all_snapshots.extend(snapshots);
                            }
                        }
                        Err(BackupServiceError::AuthenticationFailed) => {
                            echo_error("CRITICAL: Authentication failed while scanning repositories!");
                            echo_error("This should not happen after credential validation.");
                            return Err(anyhow::anyhow!("Authentication failed during repository scan"));
                        }
                        Err(_) => {
                            // Skip this repository - might be corrupted or inaccessible
                            continue;
                        }
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
            match get_snapshots(&config, &hostname, &repo_subpath, &native_path).await {
                Ok((count, snapshots)) => {
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

                                match get_snapshots(&config, &hostname, &nested_repo_subpath, &nested_path).await {
                                    Ok((nested_count, nested_snapshots)) => {
                                        if nested_count > 0 {
                                            repos.push(BackupRepo::new(nested_path).with_count(nested_count));
                                            all_snapshots.extend(nested_snapshots);
                                        }
                                    }
                                    Err(BackupServiceError::AuthenticationFailed) => {
                                        echo_error("CRITICAL: Authentication failed while scanning nested repositories!");
                                        return Err(anyhow::anyhow!("Authentication failed during nested repository scan"));
                                    }
                                    Err(_) => {
                                        // Skip this nested repository
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(BackupServiceError::AuthenticationFailed) => {
                    echo_error("CRITICAL: Authentication failed while scanning docker repositories!");
                    return Err(anyhow::anyhow!("Authentication failed during docker repository scan"));
                }
                Err(_) => {
                    // Skip this docker volume
                    continue;
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

            match get_snapshots(&config, &hostname, &repo_subpath, &native_path).await {
                Ok((count, snapshots)) => {
                    if count > 0 {
                        repos.push(BackupRepo::new(native_path).with_count(count));
                        all_snapshots.extend(snapshots);
                    }
                }
                Err(BackupServiceError::AuthenticationFailed) => {
                    echo_error("CRITICAL: Authentication failed while scanning system repositories!");
                    return Err(anyhow::anyhow!("Authentication failed during system repository scan"));
                }
                Err(_) => {
                    // Skip this system repository
                    continue;
                }
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

async fn get_snapshots(config: &Config, hostname: &str, repo_subpath: &str, native_path: &Path) -> Result<(usize, Vec<SnapshotInfo>), BackupServiceError> {
    let repo_url = format!("{}/{}/{}", config.restic_repo_base, hostname, repo_subpath);

    let output = run_command_with_env(
        "restic",
        &["--repo", &repo_url, "snapshots", "--json"],
        config,
    )?;

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
                    path: native_path.to_path_buf(),
                    id,
                })
            })
            .collect();
        Ok((count, snapshot_infos))
    } else {
        // JSON parsing failed - probably not a real repository or corrupted data
        Ok((0, Vec::new()))
    }
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