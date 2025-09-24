use crate::errors::BackupServiceError;
use std::path::PathBuf;

/// Information about a backup repository
#[derive(Debug, Clone)]
pub struct BackupRepo {
    pub native_path: PathBuf,
    pub snapshot_count: usize,
}

impl BackupRepo {
    pub fn new(native_path: PathBuf) -> Result<Self, BackupServiceError> {
        Ok(Self {
            native_path,
            snapshot_count: 0,
        })
    }

    pub fn with_count(mut self, count: usize) -> Result<Self, BackupServiceError> {
        self.snapshot_count = count;
        Ok(self)
    }

    pub fn category(&self) -> Result<&'static str, BackupServiceError> {
        let path_str = self.native_path.to_string_lossy();

        let result = if path_str.starts_with("/home/") && path_str != "/home/" {
            // Ensure it's actually a user path, not just /home or /home/
            "user_home"
        } else if path_str.starts_with("/mnt/docker-data/volumes/")
            && path_str != "/mnt/docker-data/volumes/"
        {
            // Ensure it's actually a volume, not just the volumes directory
            "docker_volume"
        } else {
            "system"
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_backup_repo_creation() -> Result<(), BackupServiceError> {
        // Test basic creation
        let path = PathBuf::from("/home/tim/documents");
        let repo = BackupRepo::new(path.clone())?;

        assert_eq!(repo.native_path, path);
        assert_eq!(repo.snapshot_count, 0);

        Ok(())
    }

    #[test]
    fn test_backup_repo_with_count() -> Result<(), BackupServiceError> {
        // Test builder pattern with count
        let path = PathBuf::from("/home/tim/projects");
        let repo = BackupRepo::new(path.clone())?.with_count(42)?;

        assert_eq!(repo.native_path, path);
        assert_eq!(repo.snapshot_count, 42);

        // Test chaining multiple operations
        let repo2 = BackupRepo::new(PathBuf::from("/tmp/test"))?.with_count(0)?;
        assert_eq!(repo2.snapshot_count, 0);

        let repo3 = BackupRepo::new(PathBuf::from("/var/log"))?.with_count(999)?;
        assert_eq!(repo3.snapshot_count, 999);

        Ok(())
    }

    #[test]
    fn test_category_detection_user_home() -> Result<(), BackupServiceError> {
        // Test various user home paths
        let test_cases = vec![
            "/home/tim",
            "/home/tim/",
            "/home/tim/documents",
            "/home/tim/my/deep/path",
            "/home/alice/projects",
            "/home/user123/data",
            "/home/user-name/files",

            // Whitespace path scenarios
            "/home/user/.local/share/Paradox Interactive",
            "/home/gamer/Documents/My Games",
            "/home/user/Software Installation Files",
            "/home/alice/.steam/steam/steamapps/common/Game Name",
            "/home/tim/.config/My Application",
            "/home/user/Downloads/My Project Files",
            "/home/developer/Projects/Cool App Name",
            "/home/user/Music/My Favorite Songs",
            "/home/gamer/.local/share/Steam Games",
            "/home/user/Videos/Home Movies Collection",
        ];

        for path_str in test_cases {
            let repo = BackupRepo::new(PathBuf::from(path_str))?;
            assert_eq!(
                repo.category()?,
                "user_home",
                "Failed for path: {}",
                path_str
            );
        }

        Ok(())
    }

    #[test]
    fn test_category_detection_docker_volume() -> Result<(), BackupServiceError> {
        // Test various docker volume paths
        let test_cases = vec![
            "/mnt/docker-data/volumes/myapp",
            "/mnt/docker-data/volumes/myapp/",
            "/mnt/docker-data/volumes/myapp/data",
            "/mnt/docker-data/volumes/postgres-data",
            "/mnt/docker-data/volumes/redis_cache",
            "/mnt/docker-data/volumes/app-volume/nested/path",
            "/mnt/docker-data/volumes/complex-name-123",

            // Whitespace docker volume scenarios
            "/mnt/docker-data/volumes/my app data",
            "/mnt/docker-data/volumes/game server config",
            "/mnt/docker-data/volumes/web app storage",
            "/mnt/docker-data/volumes/My Database Backup",
            "/mnt/docker-data/volumes/Application Config Files",
            "/mnt/docker-data/volumes/media server content",
            "/mnt/docker-data/volumes/backup storage volume",
            "/mnt/docker-data/volumes/My Personal Files",
            "/mnt/docker-data/volumes/Development Environment",
            "/mnt/docker-data/volumes/shared app resources",
        ];

        for path_str in test_cases {
            let repo = BackupRepo::new(PathBuf::from(path_str))?;
            assert_eq!(
                repo.category()?,
                "docker_volume",
                "Failed for path: {}",
                path_str
            );
        }

        Ok(())
    }

    #[test]
    fn test_category_detection_system() -> Result<(), BackupServiceError> {
        // Test various system paths
        let test_cases = vec![
            "/",
            "/etc",
            "/etc/",
            "/etc/nginx",
            "/var/log",
            "/var/lib/database",
            "/usr/local/bin",
            "/opt/software",
            "/root/.config",
            "/tmp/backup",
            "/srv/www",

            // Whitespace system path scenarios
            "/usr/share/applications/My Application",
            "/opt/Google Chrome",
            "/var/log/system events",
            "/etc/systemd/system/my service.service",
            "/usr/local/share/Application Data",
            "/opt/Software Installation",
            "/var/cache/package manager",
            "/etc/Network Manager",
            "/usr/share/icons/My Theme",
            "/srv/web application files",
            "/tmp/temporary backup files",
            "/root/.local/share/My Settings",
        ];

        for path_str in test_cases {
            let repo = BackupRepo::new(PathBuf::from(path_str))?;
            assert_eq!(repo.category()?, "system", "Failed for path: {}", path_str);
        }

        Ok(())
    }

    #[test]
    fn test_category_edge_cases() -> Result<(), BackupServiceError> {
        // Test edge cases and boundary conditions

        // Test edge cases with corrected logic
        let repo1 = BackupRepo::new(PathBuf::from("/home"))?; // Just /home, not a user directory
        assert_eq!(repo1.category()?, "system"); // Should be system, not user_home

        let repo2 = BackupRepo::new(PathBuf::from("/home/"))?; // /home/ directory itself
        assert_eq!(repo2.category()?, "system"); // Should be system, not user_home

        let repo3 = BackupRepo::new(PathBuf::from("/homestead"))?; // Similar but different
        assert_eq!(repo3.category()?, "system");

        let repo4 = BackupRepo::new(PathBuf::from("/my/home/dir"))?; // home in middle
        assert_eq!(repo4.category()?, "system");

        // Paths that look like docker volumes but aren't
        let repo5 = BackupRepo::new(PathBuf::from("/mnt/docker-data"))?; // Too short
        assert_eq!(repo5.category()?, "system");

        let repo6 = BackupRepo::new(PathBuf::from("/mnt/docker-data/volumes"))?; // Just volumes directory
        assert_eq!(repo6.category()?, "system"); // Should be system, not docker_volume

        let repo7 = BackupRepo::new(PathBuf::from("/mnt/docker-data/volumes/"))?; // Volumes directory with trailing slash
        assert_eq!(repo7.category()?, "system"); // Should be system, not docker_volume

        let repo8 = BackupRepo::new(PathBuf::from("/mnt/docker-data-volumes/app"))?; // Wrong format
        assert_eq!(repo8.category()?, "system");

        Ok(())
    }

    #[test]
    fn test_category_with_relative_and_special_paths() -> Result<(), BackupServiceError> {
        // Test relative and special paths (all should be system)
        let test_cases = vec![
            "relative/path",
            "./relative/path",
            "../relative/path",
            "home/notreallyhome",           // relative, not absolute /home/
            "mnt/docker-data/volumes/fake", // relative, not absolute
        ];

        for path_str in test_cases {
            let repo = BackupRepo::new(PathBuf::from(path_str))?;
            assert_eq!(repo.category()?, "system", "Failed for path: {}", path_str);
        }

        Ok(())
    }

    #[test]
    fn test_category_detection_whitespace_edge_cases() -> Result<(), BackupServiceError> {
        // Test edge cases with whitespace paths
        let edge_cases = vec![
            // Paths with multiple spaces
            ("/home/user/My    Project    Files", "user_home"),
            ("/mnt/docker-data/volumes/app  with  spaces", "docker_volume"),
            ("/usr/share/My  Application  Data", "system"),

            // Paths with leading/trailing spaces in components
            ("/home/user/ leading space", "user_home"),
            ("/home/user/trailing space ", "user_home"),
            ("/mnt/docker-data/volumes/ docker volume ", "docker_volume"),

            // Paths with special whitespace characters (tabs, newlines would be unusual but test robustness)
            ("/home/user/tab\tseparated", "user_home"),

            // Mixed special characters and spaces
            ("/home/user/.local/share/App-Name With Spaces", "user_home"),
            ("/mnt/docker-data/volumes/my-app data_v2", "docker_volume"),
            ("/etc/systemd/system/my-service with spaces.service", "system"),

            // Real-world gaming and application paths
            ("/home/user/.steam/steam/steamapps/common/Counter Strike", "user_home"),
            ("/home/gamer/.local/share/Paradox Interactive/Europa Universalis IV", "user_home"),
            ("/home/user/.config/Google Chrome", "user_home"),
            ("/home/developer/Projects/My Awesome App", "user_home"),
        ];

        for (path, expected_category) in edge_cases {
            let repo = BackupRepo::new(PathBuf::from(path))?;
            assert_eq!(repo.category()?, expected_category,
                "Failed for whitespace edge case: {}", path);
        }

        Ok(())
    }

    #[test]
    fn test_backup_repo_comprehensive_workflow() -> Result<(), BackupServiceError> {
        // Test complete workflow with different path types including whitespace

        // User home workflow with whitespace
        let user_repo = BackupRepo::new(PathBuf::from("/home/tim/.local/share/My Documents"))?.with_count(15)?;
        assert_eq!(user_repo.category()?, "user_home");
        assert_eq!(user_repo.snapshot_count, 15);

        // Docker volume workflow with whitespace
        let docker_repo =
            BackupRepo::new(PathBuf::from("/mnt/docker-data/volumes/postgres backup data"))?.with_count(8)?;
        assert_eq!(docker_repo.category()?, "docker_volume");
        assert_eq!(docker_repo.snapshot_count, 8);

        // System path workflow with whitespace
        let system_repo = BackupRepo::new(PathBuf::from("/usr/share/applications/My App"))?.with_count(3)?;
        assert_eq!(system_repo.category()?, "system");
        assert_eq!(system_repo.snapshot_count, 3);

        Ok(())
    }
}
