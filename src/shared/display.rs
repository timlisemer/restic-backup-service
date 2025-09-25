use crate::errors::BackupServiceError;
use crate::shared::operations::SnapshotInfo;
use crate::repository::BackupRepo;
use std::collections::HashMap;
use tracing::info;

/// Display formatter for backup summaries and listings
pub struct DisplayFormatter;

impl DisplayFormatter {
    /// Display complete backup summary (main entry point)
    pub fn display_backup_summary(
        repos: &[BackupRepo],
        snapshots: &[SnapshotInfo],
    ) -> Result<(), BackupServiceError> {
        Self::display_backup_paths_summary(repos)?;
        Self::display_snapshot_timeline(snapshots)?;
        info!("");
        Ok(())
    }

    /// Display backup paths summary section
    pub fn display_backup_paths_summary(repos: &[BackupRepo]) -> Result<(), BackupServiceError> {
        info!("");
        info!("BACKUP PATHS SUMMARY:");
        info!("====================");

        // Group by category
        let categories = Self::group_repos_by_category(repos)?;

        // Display each category
        Self::display_user_home_repos(&categories)?;
        Self::display_docker_volume_repos(&categories)?;
        Self::display_system_repos(&categories)?;

        Ok(())
    }

    /// Display snapshot timeline section
    pub fn display_snapshot_timeline(snapshots: &[SnapshotInfo]) -> Result<(), BackupServiceError> {
        info!("");
        info!("SNAPSHOT TIMELINE:");
        info!("==================");

        if snapshots.is_empty() {
            info!("No snapshots found");
            return Ok(());
        }

        let timeline = Self::group_snapshots_by_time(snapshots)?;
        Self::display_timeline_entries(&timeline)?;

        Ok(())
    }

    /// Group repositories by category
    fn group_repos_by_category(
        repos: &[BackupRepo],
    ) -> Result<HashMap<&str, Vec<&BackupRepo>>, BackupServiceError> {
        let mut categories: HashMap<&str, Vec<&BackupRepo>> = HashMap::new();
        for repo in repos {
            categories.entry(repo.category()?).or_default().push(repo);
        }
        Ok(categories)
    }

    /// Display user home repositories
    fn display_user_home_repos(
        categories: &HashMap<&str, Vec<&BackupRepo>>,
    ) -> Result<(), BackupServiceError> {
        let empty_vec = Vec::new();
        let user_repos = categories.get("user_home").unwrap_or(&empty_vec);

        info!("");
        info!("User Home ({} paths):", user_repos.len());
        if user_repos.is_empty() {
            info!("  None");
        } else {
            for repo in user_repos {
                Self::display_repo_entry(repo)?;
            }
        }

        Ok(())
    }

    /// Display docker volume repositories
    fn display_docker_volume_repos(
        categories: &HashMap<&str, Vec<&BackupRepo>>,
    ) -> Result<(), BackupServiceError> {
        let empty_vec = Vec::new();
        let docker_repos = categories.get("docker_volume").unwrap_or(&empty_vec);

        info!("");
        info!("Docker Volumes ({} paths):", docker_repos.len());
        if docker_repos.is_empty() {
            info!("  None");
        } else {
            for repo in docker_repos {
                Self::display_repo_entry(repo)?;
            }
        }

        Ok(())
    }

    /// Display system repositories
    fn display_system_repos(
        categories: &HashMap<&str, Vec<&BackupRepo>>,
    ) -> Result<(), BackupServiceError> {
        let empty_vec = Vec::new();
        let system_repos = categories.get("system").unwrap_or(&empty_vec);

        info!("");
        info!("System ({} paths):", system_repos.len());
        if system_repos.is_empty() {
            info!("  None");
        } else {
            for repo in system_repos {
                Self::display_repo_entry(repo)?;
            }
        }

        Ok(())
    }

    /// Display a single repository entry
    fn display_repo_entry(repo: &BackupRepo) -> Result<(), BackupServiceError> {
        info!(
            "  {:<50} - {} snapshots",
            repo.native_path.display(),
            repo.snapshot_count
        );
        Ok(())
    }

    /// Group snapshots by time for timeline display
    fn group_snapshots_by_time(
        snapshots: &[SnapshotInfo],
    ) -> Result<HashMap<String, Vec<&SnapshotInfo>>, BackupServiceError> {
        let mut timeline: HashMap<String, Vec<&SnapshotInfo>> = HashMap::new();
        for snapshot in snapshots {
            let time_key = snapshot.time.format("%Y-%m-%d %H:%M").to_string();
            timeline.entry(time_key).or_default().push(snapshot);
        }
        Ok(timeline)
    }

    /// Display timeline entries
    fn display_timeline_entries(
        timeline: &HashMap<String, Vec<&SnapshotInfo>>,
    ) -> Result<(), BackupServiceError> {
        // Sort and display
        let mut times: Vec<_> = timeline.keys().cloned().collect();
        times.sort();
        times.reverse();

        for time in times.iter().take(20) {
            if let Some(snaps) = timeline.get(time) {
                info!("");
                info!("{}:", time);
                for snap in snaps {
                    Self::display_snapshot_entry(snap)?;
                }
            }
        }

        if times.len() > 20 {
            info!("");
            info!("... and {} more time points", times.len() - 20);
        }

        Ok(())
    }

    /// Display a single snapshot entry
    fn display_snapshot_entry(snapshot: &SnapshotInfo) -> Result<(), BackupServiceError> {
        info!("  - {:<50} (id: {})", snapshot.path.display(), snapshot.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::operations::SnapshotInfo;
    use crate::repository::BackupRepo;
    use chrono::{DateTime, Utc};
    use std::path::PathBuf;

    fn create_test_snapshot(time_str: &str, path: &str, id: &str) -> SnapshotInfo {
        let time = DateTime::parse_from_rfc3339(time_str)
            .unwrap()
            .with_timezone(&Utc);
        SnapshotInfo {
            time,
            path: PathBuf::from(path),
            id: id.to_string(),
        }
    }

    fn create_test_repo(path: &str, count: usize) -> Result<BackupRepo, BackupServiceError> {
        BackupRepo::new(PathBuf::from(path))?.with_count(count)
    }

    #[test]
    fn test_group_snapshots_by_time() -> Result<(), BackupServiceError> {
        let snapshots = vec![
            create_test_snapshot("2025-01-15T10:30:00Z", "/home/tim/docs", "abc123"),
            create_test_snapshot("2025-01-15T10:30:30Z", "/home/tim/projects", "def456"), // same minute
            create_test_snapshot("2025-01-15T11:45:00Z", "/var/log", "ghi789"), // different hour
            create_test_snapshot("2025-01-16T10:30:00Z", "/etc/nginx", "jkl012"), // different day
        ];

        let timeline = DisplayFormatter::group_snapshots_by_time(&snapshots)?;

        // Check that snapshots are grouped correctly by "YYYY-MM-DD HH:MM"
        assert!(timeline.contains_key("2025-01-15 10:30"));
        assert!(timeline.contains_key("2025-01-15 11:45"));
        assert!(timeline.contains_key("2025-01-16 10:30"));

        // Check that the first group has 2 snapshots (same minute)
        assert_eq!(timeline.get("2025-01-15 10:30").unwrap().len(), 2);

        // Check that other groups have 1 snapshot each
        assert_eq!(timeline.get("2025-01-15 11:45").unwrap().len(), 1);
        assert_eq!(timeline.get("2025-01-16 10:30").unwrap().len(), 1);

        Ok(())
    }

    #[test]
    fn test_group_repos_by_category() -> Result<(), BackupServiceError> {
        let repos = vec![
            // Original paths
            create_test_repo("/home/tim/documents", 5)?,
            create_test_repo("/home/alice/projects", 3)?,
            create_test_repo("/mnt/docker-data/volumes/postgres", 8)?,
            create_test_repo("/mnt/docker-data/volumes/redis", 2)?,
            create_test_repo("/etc/nginx", 1)?,
            create_test_repo("/var/log", 4)?,
            // Whitespace paths
            create_test_repo("/home/gamer/.local/share/Paradox Interactive", 7)?,
            create_test_repo("/home/user/.config/Google Chrome", 12)?,
            create_test_repo("/mnt/docker-data/volumes/my app data", 6)?,
            create_test_repo("/mnt/docker-data/volumes/web server config", 3)?,
            create_test_repo("/usr/share/applications/My Application", 2)?,
            create_test_repo("/opt/Google Chrome", 1)?,
        ];

        let categories = DisplayFormatter::group_repos_by_category(&repos)?;

        // Check that we have all expected categories
        assert!(categories.contains_key("user_home"));
        assert!(categories.contains_key("docker_volume"));
        assert!(categories.contains_key("system"));

        // Check counts (should include whitespace paths)
        assert_eq!(categories.get("user_home").unwrap().len(), 4); // 2 original + 2 whitespace
        assert_eq!(categories.get("docker_volume").unwrap().len(), 4); // 2 original + 2 whitespace
        assert_eq!(categories.get("system").unwrap().len(), 4); // 2 original + 2 whitespace

        Ok(())
    }

    #[test]
    fn test_group_snapshots_by_time_edge_cases() -> Result<(), BackupServiceError> {
        // Test empty snapshots
        let empty_snapshots: Vec<SnapshotInfo> = vec![];
        let timeline = DisplayFormatter::group_snapshots_by_time(&empty_snapshots)?;
        assert!(timeline.is_empty());

        // Test snapshots at exact minute boundaries
        let boundary_snapshots = vec![
            create_test_snapshot("2025-01-15T10:29:59Z", "/path1", "id1"),
            create_test_snapshot("2025-01-15T10:30:00Z", "/path2", "id2"),
            create_test_snapshot("2025-01-15T10:30:01Z", "/path3", "id3"),
            create_test_snapshot("2025-01-15T10:31:00Z", "/path4", "id4"),
        ];

        let timeline = DisplayFormatter::group_snapshots_by_time(&boundary_snapshots)?;

        // Should have 3 different minute groups
        assert_eq!(timeline.len(), 3);
        assert!(timeline.contains_key("2025-01-15 10:29"));
        assert!(timeline.contains_key("2025-01-15 10:30"));
        assert!(timeline.contains_key("2025-01-15 10:31"));

        // Check grouping is correct
        assert_eq!(timeline.get("2025-01-15 10:29").unwrap().len(), 1);
        assert_eq!(timeline.get("2025-01-15 10:30").unwrap().len(), 2);
        assert_eq!(timeline.get("2025-01-15 10:31").unwrap().len(), 1);

        Ok(())
    }

    #[test]
    fn test_group_repos_by_category_edge_cases() -> Result<(), BackupServiceError> {
        // Test empty repos
        let empty_repos: Vec<BackupRepo> = vec![];
        let categories = DisplayFormatter::group_repos_by_category(&empty_repos)?;
        assert!(categories.is_empty());

        // Test single category
        let single_category_repos = vec![
            create_test_repo("/home/user1/docs", 1)?,
            create_test_repo("/home/user2/projects", 2)?,
            create_test_repo("/home/user3/files", 3)?,
        ];

        let categories = DisplayFormatter::group_repos_by_category(&single_category_repos)?;
        assert_eq!(categories.len(), 1);
        assert!(categories.contains_key("user_home"));
        assert_eq!(categories.get("user_home").unwrap().len(), 3);

        Ok(())
    }

    #[test]
    fn test_timeline_display_logic() -> Result<(), BackupServiceError> {
        // Test the main display functions without actually printing (integration test)
        let repos = vec![
            create_test_repo("/home/tim/documents", 5)?,
            create_test_repo("/mnt/docker-data/volumes/app", 3)?,
            create_test_repo("/etc/config", 2)?,
        ];

        let snapshots = vec![
            create_test_snapshot("2025-01-15T10:30:00Z", "/home/tim/documents", "snap1"),
            create_test_snapshot("2025-01-15T11:00:00Z", "/etc/config", "snap2"),
        ];

        // These functions print output, but should not error
        DisplayFormatter::display_backup_paths_summary(&repos)?;
        DisplayFormatter::display_snapshot_timeline(&snapshots)?;
        DisplayFormatter::display_backup_summary(&repos, &snapshots)?;

        Ok(())
    }

    #[test]
    fn test_snapshot_time_formatting_precision() -> Result<(), BackupServiceError> {
        // Test that different times within the same minute are grouped correctly
        let snapshots = vec![
            create_test_snapshot("2025-01-15T10:30:00.000Z", "/path1", "id1"),
            create_test_snapshot("2025-01-15T10:30:15.500Z", "/path2", "id2"),
            create_test_snapshot("2025-01-15T10:30:45.999Z", "/path3", "id3"),
            create_test_snapshot("2025-01-15T10:31:00.001Z", "/path4", "id4"), // different minute
        ];

        let timeline = DisplayFormatter::group_snapshots_by_time(&snapshots)?;

        // All first 3 should be in same minute group
        assert_eq!(timeline.get("2025-01-15 10:30").unwrap().len(), 3);
        assert_eq!(timeline.get("2025-01-15 10:31").unwrap().len(), 1);

        Ok(())
    }

    #[test]
    fn test_mixed_timezone_handling() -> Result<(), BackupServiceError> {
        // All snapshots are converted to UTC in the struct, so timezone differences
        // should be handled correctly
        let snapshots = vec![
            create_test_snapshot("2025-01-15T10:30:00Z", "/path1", "id1"), // UTC
            create_test_snapshot("2025-01-15T05:30:00-05:00", "/path2", "id2"), // EST (same as UTC time)
            create_test_snapshot("2025-01-15T15:30:00+05:00", "/path3", "id3"), // Different timezone (same UTC)
        ];

        let timeline = DisplayFormatter::group_snapshots_by_time(&snapshots)?;

        // All should be grouped together as they represent the same UTC time
        assert_eq!(timeline.len(), 1);
        assert!(timeline.contains_key("2025-01-15 10:30"));
        assert_eq!(timeline.get("2025-01-15 10:30").unwrap().len(), 3);

        Ok(())
    }

    #[test]
    fn test_repository_categorization_comprehensive() -> Result<(), BackupServiceError> {
        // Test comprehensive repository categorization with various path types
        let repos = vec![
            // User home variations
            create_test_repo("/home/tim", 1)?,
            create_test_repo("/home/alice/documents", 2)?,
            create_test_repo("/home/user123/projects/rust", 3)?,
            // Docker volume variations
            create_test_repo("/mnt/docker-data/volumes/postgres", 4)?,
            create_test_repo("/mnt/docker-data/volumes/app-data/config", 5)?,
            // System variations
            create_test_repo("/etc", 6)?,
            create_test_repo("/var/log/nginx", 7)?,
            create_test_repo("/usr/local/bin", 8)?,
            create_test_repo("/opt/software", 9)?,
            create_test_repo("/", 10)?,
        ];

        let categories = DisplayFormatter::group_repos_by_category(&repos)?;

        // Verify all categories exist
        assert!(categories.contains_key("user_home"));
        assert!(categories.contains_key("docker_volume"));
        assert!(categories.contains_key("system"));

        // Verify counts
        assert_eq!(categories.get("user_home").unwrap().len(), 3);
        assert_eq!(categories.get("docker_volume").unwrap().len(), 2);
        assert_eq!(categories.get("system").unwrap().len(), 5);

        Ok(())
    }

    #[test]
    fn test_display_whitespace_path_formatting() -> Result<(), BackupServiceError> {
        // Test display functionality with paths containing spaces
        let repos = vec![
            create_test_repo("/home/gamer/.local/share/Paradox Interactive", 15)?,
            create_test_repo("/home/user/.steam/steam/steamapps/common/Counter Strike", 8)?,
            create_test_repo("/mnt/docker-data/volumes/my app data", 12)?,
            create_test_repo("/mnt/docker-data/volumes/web server config", 4)?,
            create_test_repo("/usr/share/applications/Visual Studio Code", 3)?,
            create_test_repo("/opt/Google Chrome", 6)?,
        ];

        let snapshots = vec![
            create_test_snapshot(
                "2025-01-15T10:30:00Z",
                "/home/gamer/.local/share/Paradox Interactive",
                "snap1",
            ),
            create_test_snapshot(
                "2025-01-15T10:31:00Z",
                "/mnt/docker-data/volumes/my app data",
                "snap2",
            ),
            create_test_snapshot(
                "2025-01-15T10:32:00Z",
                "/usr/share/applications/Visual Studio Code",
                "snap3",
            ),
        ];

        // Test grouping with whitespace paths
        let categories = DisplayFormatter::group_repos_by_category(&repos)?;
        assert_eq!(categories.get("user_home").unwrap().len(), 2);
        assert_eq!(categories.get("docker_volume").unwrap().len(), 2);
        assert_eq!(categories.get("system").unwrap().len(), 2);

        // Test timeline grouping with whitespace paths
        let timeline = DisplayFormatter::group_snapshots_by_time(&snapshots)?;
        assert_eq!(timeline.len(), 3); // Different minutes

        // Test that display functions don't error with whitespace paths
        DisplayFormatter::display_backup_paths_summary(&repos)?;
        DisplayFormatter::display_snapshot_timeline(&snapshots)?;
        DisplayFormatter::display_backup_summary(&repos, &snapshots)?;

        Ok(())
    }
}
