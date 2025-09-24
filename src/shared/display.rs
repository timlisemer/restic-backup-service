use crate::errors::BackupServiceError;
use crate::helpers::SnapshotInfo;
use crate::repository::BackupRepo;
use std::collections::HashMap;

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
        println!();
        Ok(())
    }

    /// Display backup paths summary section
    pub fn display_backup_paths_summary(repos: &[BackupRepo]) -> Result<(), BackupServiceError> {
        println!("\nBACKUP PATHS SUMMARY:");
        println!("====================");

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
        println!("\nSNAPSHOT TIMELINE:");
        println!("==================");

        if snapshots.is_empty() {
            println!("No snapshots found");
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

        println!("\nUser Home ({} paths):", user_repos.len());
        if user_repos.is_empty() {
            println!("  None");
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

        println!("\nDocker Volumes ({} paths):", docker_repos.len());
        if docker_repos.is_empty() {
            println!("  None");
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

        println!("\nSystem ({} paths):", system_repos.len());
        if system_repos.is_empty() {
            println!("  None");
        } else {
            for repo in system_repos {
                Self::display_repo_entry(repo)?;
            }
        }

        Ok(())
    }

    /// Display a single repository entry
    fn display_repo_entry(repo: &BackupRepo) -> Result<(), BackupServiceError> {
        println!(
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
                println!("\n{}:", time);
                for snap in snaps {
                    Self::display_snapshot_entry(snap)?;
                }
            }
        }

        if times.len() > 20 {
            println!("\n... and {} more time points", times.len() - 20);
        }

        Ok(())
    }

    /// Display a single snapshot entry
    fn display_snapshot_entry(snapshot: &SnapshotInfo) -> Result<(), BackupServiceError> {
        println!("  - {:<50} (id: {})", snapshot.path.display(), snapshot.id);
        Ok(())
    }
}
