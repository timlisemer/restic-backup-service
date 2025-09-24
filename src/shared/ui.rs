use crate::errors::BackupServiceError;
use chrono::{DateTime, Duration, Utc};
use dialoguer::{Confirm, MultiSelect, Select};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

/// Host selection data
#[derive(Debug, Clone)]
pub struct HostSelection {
    pub selected_host: String,
}

/// Repository selection data
#[derive(Debug, Clone)]
pub struct RepositorySelection {
    pub selected_repos: Vec<RepositorySelectionItem>,
}

#[derive(Debug, Clone)]
pub struct RepositorySelectionItem {
    pub path: PathBuf,
    pub repo_subpath: String,
    pub category: String,
    pub snapshots: Vec<SnapshotItem>,
}

#[derive(Debug, Clone)]
pub struct SnapshotItem {
    pub id: String,
    pub time: DateTime<Utc>,
}

/// Timestamp selection data
#[derive(Debug, Clone)]
pub struct TimestampSelection {
    pub selected_timestamp: DateTime<Utc>,
}

/// Interactive host selection UI
pub async fn select_host(
    available_hosts: Vec<String>,
    current_host: String,
    host_opt: Option<String>,
) -> Result<HostSelection, BackupServiceError> {
    let selected_host = if let Some(host) = host_opt {
        host
    } else {
        if available_hosts.is_empty() {
            return Err(BackupServiceError::ConfigurationError(
                "No hosts found in backup repository".to_string(),
            ));
        }

        let default = available_hosts
            .iter()
            .position(|h| h == &current_host)
            .unwrap_or(0);

        let selection = Select::new()
            .with_prompt("Select hostname")
            .items(&available_hosts)
            .default(default)
            .interact()?;

        available_hosts[selection].clone()
    };

    Ok(HostSelection { selected_host })
}

/// Interactive repository selection UI
pub async fn select_repositories(
    backup_data: Vec<RepositorySelectionItem>,
    path_opt: Option<String>,
) -> Result<RepositorySelection, BackupServiceError> {
    let selected_repos = if let Some(path) = path_opt {
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
            0 => backup_data.clone(),
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
        return Err(BackupServiceError::ConfigurationError(
            "No repositories selected".to_string(),
        ));
    }

    Ok(RepositorySelection { selected_repos })
}

/// Interactive timestamp selection UI
pub async fn select_timestamp(
    selected_repos: &[RepositorySelectionItem],
    timestamp_opt: Option<String>,
) -> Result<TimestampSelection, BackupServiceError> {
    let selected_timestamp = if let Some(ts) = timestamp_opt {
        ts.parse::<DateTime<Utc>>()?
    } else {
        let mut all_timestamps: Vec<DateTime<Utc>> = selected_repos
            .iter()
            .flat_map(|r| &r.snapshots)
            .map(|s| s.time)
            .collect();
        all_timestamps.sort();
        all_timestamps.reverse();
        all_timestamps.dedup();

        if all_timestamps.is_empty() {
            return Err(BackupServiceError::ConfigurationError(
                "No snapshots found for selected repositories".to_string(),
            ));
        }

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

    Ok(TimestampSelection { selected_timestamp })
}

/// Create and configure progress bar for backup operations
pub fn create_backup_progress_bar(total: usize) -> Result<ProgressBar, BackupServiceError> {
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")?
            .progress_chars("#>-"),
    );
    Ok(pb)
}

/// Simple confirmation dialog
pub async fn confirm_action(prompt: &str, default: bool) -> Result<bool, BackupServiceError> {
    let result = Confirm::new()
        .with_prompt(prompt)
        .default(default)
        .interact()?;
    Ok(result)
}
