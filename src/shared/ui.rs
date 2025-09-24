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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::path::PathBuf;

    fn create_test_snapshot_item(time_str: &str, id: &str) -> SnapshotItem {
        let time = DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .with_timezone(&Utc);
        SnapshotItem {
            id: id.to_string(),
            time,
        }
    }

    fn create_test_repository_item(
        path: &str,
        repo_subpath: &str,
        category: &str,
        snapshots: Vec<SnapshotItem>,
    ) -> RepositorySelectionItem {
        RepositorySelectionItem {
            path: PathBuf::from(path),
            repo_subpath: repo_subpath.to_string(),
            category: category.to_string(),
            snapshots,
        }
    }

    #[tokio::test]
    async fn test_select_host_with_host_opt() -> Result<(), BackupServiceError> {
        let available_hosts = vec!["host1".to_string(), "host2".to_string()];
        let current_host = "host1".to_string();
        let host_opt = Some("host2".to_string());

        let result = select_host(available_hosts, current_host, host_opt).await?;
        assert_eq!(result.selected_host, "host2");
        Ok(())
    }

    #[tokio::test]
    async fn test_select_host_empty_hosts_error() {
        let available_hosts = vec![];
        let current_host = "nonexistent".to_string();
        let host_opt = None;

        let result = select_host(available_hosts, current_host, host_opt).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No hosts found"));
    }

    #[tokio::test]
    async fn test_select_repositories_path_filtering() -> Result<(), BackupServiceError> {
        let backup_data = vec![
            create_test_repository_item(
                "/home/tim/docs",
                "user_home/tim/docs",
                "user_home",
                vec![create_test_snapshot_item("2025-01-15T10:30:00Z", "snap1")],
            ),
            create_test_repository_item(
                "/home/alice/projects",
                "user_home/alice/projects",
                "user_home",
                vec![create_test_snapshot_item("2025-01-15T11:00:00Z", "snap2")],
            ),
        ];

        let path_opt = Some("/home/tim/docs".to_string());
        let result = select_repositories(backup_data, path_opt).await?;

        assert_eq!(result.selected_repos.len(), 1);
        assert_eq!(result.selected_repos[0].path, PathBuf::from("/home/tim/docs"));
        Ok(())
    }

    #[tokio::test]
    async fn test_select_repositories_path_filtering_no_match() {
        let backup_data = vec![
            create_test_repository_item(
                "/home/tim/docs",
                "user_home/tim/docs",
                "user_home",
                vec![create_test_snapshot_item("2025-01-15T10:30:00Z", "snap1")],
            ),
        ];

        let path_opt = Some("/nonexistent/path".to_string());
        let result = select_repositories(backup_data, path_opt).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No repositories selected"));
    }

    #[tokio::test]
    async fn test_select_timestamp_with_timestamp_opt() -> Result<(), BackupServiceError> {
        let repos = vec![
            create_test_repository_item(
                "/home/tim/docs",
                "user_home/tim/docs",
                "user_home",
                vec![create_test_snapshot_item("2025-01-15T10:30:00Z", "snap1")],
            ),
        ];

        let timestamp_opt = Some("2025-01-15T12:00:00Z".to_string());
        let result = select_timestamp(&repos, timestamp_opt).await?;

        let expected_time = DateTime::parse_from_rfc3339("2025-01-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(result.selected_timestamp, expected_time);
        Ok(())
    }

    #[tokio::test]
    async fn test_select_timestamp_empty_snapshots_error() {
        let repos = vec![
            create_test_repository_item(
                "/home/tim/docs",
                "user_home/tim/docs",
                "user_home",
                vec![], // No snapshots
            ),
        ];

        let result = select_timestamp(&repos, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No snapshots found"));
    }

    #[test]
    fn test_time_window_calculation() -> Result<(), BackupServiceError> {
        // Test the core time window logic
        let snapshots = vec![
            create_test_snapshot_item("2025-01-15T10:30:00Z", "snap1"),
            create_test_snapshot_item("2025-01-15T10:32:30Z", "snap2"),
            create_test_snapshot_item("2025-01-15T10:35:15Z", "snap3"), // Different window
            create_test_snapshot_item("2025-01-15T10:37:45Z", "snap4"), // Same window as snap3
        ];

        let repos = vec![
            create_test_repository_item(
                "/home/tim/docs",
                "user_home/tim/docs",
                "user_home",
                snapshots,
            ),
        ];

        // Extract the time window calculation logic
        let mut all_timestamps: Vec<DateTime<Utc>> = repos
            .iter()
            .flat_map(|r| &r.snapshots)
            .map(|s| s.time)
            .collect();
        all_timestamps.sort();
        all_timestamps.reverse();
        all_timestamps.dedup();

        let mut window_times = Vec::new();
        for ts in &all_timestamps {
            let window_start = ts.timestamp() - (ts.timestamp() % 300);
            let window_time = DateTime::<Utc>::from_timestamp(window_start, 0).unwrap();

            if !window_times.contains(&window_time) {
                window_times.push(window_time);
            }
        }

        // Should have 2 different 5-minute windows
        assert_eq!(window_times.len(), 2);

        // First window should be for 10:30-10:35 (contains snap1, snap2)
        let first_window = window_times.iter().max().unwrap(); // Latest first due to reverse sort
        assert_eq!(first_window.format("%H:%M").to_string(), "10:35");

        // Second window should be for 10:30-10:35 (contains snap3, snap4)
        let second_window = window_times.iter().min().unwrap();
        assert_eq!(second_window.format("%H:%M").to_string(), "10:30");

        Ok(())
    }

    #[test]
    fn test_time_window_counting() -> Result<(), BackupServiceError> {
        // Test time window snapshot counting logic
        let snapshots = vec![
            create_test_snapshot_item("2025-01-15T10:30:00Z", "snap1"),
            create_test_snapshot_item("2025-01-15T10:31:00Z", "snap2"),
            create_test_snapshot_item("2025-01-15T10:32:30Z", "snap3"), // Same window
            create_test_snapshot_item("2025-01-15T10:35:00Z", "snap4"), // Next window
            create_test_snapshot_item("2025-01-15T10:36:00Z", "snap5"), // Same window as snap4
        ];

        let all_timestamps: Vec<DateTime<Utc>> = snapshots.iter().map(|s| s.time).collect();

        // Test counting for first window (10:30-10:35)
        let window_time = DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let window_end = window_time + Duration::minutes(5);

        let count = all_timestamps
            .iter()
            .filter(|t| **t >= window_time && **t < window_end)
            .count();

        assert_eq!(count, 3); // snap1, snap2, snap3

        // Test counting for second window (10:35-10:40)
        let window_time2 = DateTime::parse_from_rfc3339("2025-01-15T10:35:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let window_end2 = window_time2 + Duration::minutes(5);

        let count2 = all_timestamps
            .iter()
            .filter(|t| **t >= window_time2 && **t < window_end2)
            .count();

        assert_eq!(count2, 2); // snap4, snap5

        Ok(())
    }

    #[test]
    fn test_time_window_deduplication() -> Result<(), BackupServiceError> {
        // Test that identical timestamps don't create duplicate windows
        let snapshots = vec![
            create_test_snapshot_item("2025-01-15T10:30:00Z", "snap1"),
            create_test_snapshot_item("2025-01-15T10:30:00Z", "snap2"), // Exact duplicate time
            create_test_snapshot_item("2025-01-15T10:30:30Z", "snap3"), // Same window
        ];

        let mut all_timestamps: Vec<DateTime<Utc>> = snapshots.iter().map(|s| s.time).collect();
        all_timestamps.sort();
        all_timestamps.reverse();
        all_timestamps.dedup();

        // Should have 2 unique timestamps after deduplication
        assert_eq!(all_timestamps.len(), 2);

        let mut window_times = Vec::new();
        for ts in &all_timestamps {
            let window_start = ts.timestamp() - (ts.timestamp() % 300);
            let window_time = DateTime::<Utc>::from_timestamp(window_start, 0).unwrap();

            if !window_times.contains(&window_time) {
                window_times.push(window_time);
            }
        }

        // Should have only 1 window since all timestamps are in same 5-minute window
        assert_eq!(window_times.len(), 1);

        Ok(())
    }

    #[test]
    fn test_time_window_edge_cases() -> Result<(), BackupServiceError> {
        // Test edge cases around 5-minute boundaries
        let snapshots = vec![
            create_test_snapshot_item("2025-01-15T10:29:59Z", "snap1"), // Just before window
            create_test_snapshot_item("2025-01-15T10:30:00Z", "snap2"), // Exact window start
            create_test_snapshot_item("2025-01-15T10:34:59Z", "snap3"), // Just before window end
            create_test_snapshot_item("2025-01-15T10:35:00Z", "snap4"), // Next window start
        ];

        let all_timestamps: Vec<DateTime<Utc>> = snapshots.iter().map(|s| s.time).collect();

        // Test first window (10:25-10:30)
        let window1 = DateTime::parse_from_rfc3339("2025-01-15T10:25:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let window1_end = window1 + Duration::minutes(5);
        let count1 = all_timestamps
            .iter()
            .filter(|t| **t >= window1 && **t < window1_end)
            .count();
        assert_eq!(count1, 1); // Only snap1

        // Test second window (10:30-10:35)
        let window2 = DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let window2_end = window2 + Duration::minutes(5);
        let count2 = all_timestamps
            .iter()
            .filter(|t| **t >= window2 && **t < window2_end)
            .count();
        assert_eq!(count2, 2); // snap2, snap3

        // Test third window (10:35-10:40)
        let window3 = DateTime::parse_from_rfc3339("2025-01-15T10:35:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let window3_end = window3 + Duration::minutes(5);
        let count3 = all_timestamps
            .iter()
            .filter(|t| **t >= window3 && **t < window3_end)
            .count();
        assert_eq!(count3, 1); // snap4

        Ok(())
    }

    #[test]
    fn test_create_backup_progress_bar() -> Result<(), BackupServiceError> {
        // Test progress bar creation
        let pb = create_backup_progress_bar(100)?;
        assert_eq!(pb.length(), Some(100));

        // Test with zero items
        let pb_zero = create_backup_progress_bar(0)?;
        assert_eq!(pb_zero.length(), Some(0));

        // Test with large number
        let pb_large = create_backup_progress_bar(999999)?;
        assert_eq!(pb_large.length(), Some(999999));

        Ok(())
    }

    #[test]
    fn test_repository_category_filtering() -> Result<(), BackupServiceError> {
        // Test the category filtering logic used in select_repositories
        let backup_data = vec![
            create_test_repository_item(
                "/home/tim/docs",
                "user_home/tim/docs",
                "user_home",
                vec![create_test_snapshot_item("2025-01-15T10:30:00Z", "snap1")],
            ),
            create_test_repository_item(
                "/home/alice/projects",
                "user_home/alice/projects",
                "user_home",
                vec![create_test_snapshot_item("2025-01-15T11:00:00Z", "snap2")],
            ),
            create_test_repository_item(
                "/mnt/docker-data/volumes/postgres",
                "docker_volume/postgres",
                "docker_volume",
                vec![create_test_snapshot_item("2025-01-15T09:00:00Z", "snap3")],
            ),
            create_test_repository_item(
                "/etc/nginx",
                "system/etc_nginx",
                "system",
                vec![create_test_snapshot_item("2025-01-15T08:00:00Z", "snap4")],
            ),
        ];

        // Test user_home filtering
        let user_home_repos: Vec<RepositorySelectionItem> = backup_data
            .iter()
            .filter(|r| r.category == "user_home")
            .cloned()
            .collect();
        assert_eq!(user_home_repos.len(), 2);
        assert!(user_home_repos.iter().all(|r| r.category == "user_home"));

        // Test docker_volume filtering
        let docker_repos: Vec<RepositorySelectionItem> = backup_data
            .iter()
            .filter(|r| r.category == "docker_volume")
            .cloned()
            .collect();
        assert_eq!(docker_repos.len(), 1);
        assert_eq!(docker_repos[0].path, PathBuf::from("/mnt/docker-data/volumes/postgres"));

        // Test system filtering
        let system_repos: Vec<RepositorySelectionItem> = backup_data
            .iter()
            .filter(|r| r.category == "system")
            .cloned()
            .collect();
        assert_eq!(system_repos.len(), 1);
        assert_eq!(system_repos[0].path, PathBuf::from("/etc/nginx"));

        Ok(())
    }

    #[test]
    fn test_host_default_selection_logic() -> Result<(), BackupServiceError> {
        // Test the host default selection logic
        let available_hosts = vec![
            "host1".to_string(),
            "host2".to_string(),
            "current-host".to_string(),
            "host4".to_string(),
        ];
        let current_host = "current-host".to_string();

        // Test finding correct default position
        let default = available_hosts
            .iter()
            .position(|h| h == &current_host)
            .unwrap_or(0);
        assert_eq!(default, 2);

        // Test fallback when current host not found
        let current_host_missing = "missing-host".to_string();
        let default_fallback = available_hosts
            .iter()
            .position(|h| h == &current_host_missing)
            .unwrap_or(0);
        assert_eq!(default_fallback, 0);

        Ok(())
    }
}
