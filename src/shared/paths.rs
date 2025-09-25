use crate::errors::BackupServiceError;
use crate::shared::constants::{
    DOCKER_BACKING_FS_BLOCK_DEV, DOCKER_METADATA_DB, DOCKER_VOLUMES_DIR,
};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Docker volume discovery and validation utilities
pub struct PathUtilities;

impl PathUtilities {
    /// Discover and validate docker volumes, extracted from backup.rs
    pub fn discover_docker_volumes() -> Result<Vec<PathBuf>, BackupServiceError> {
        let mut volumes = Vec::new();

        let docker_volumes_path = Path::new(DOCKER_VOLUMES_DIR);
        if docker_volumes_path.exists() {
            info!("Detecting docker volumes...");
            if let Ok(entries) = std::fs::read_dir(docker_volumes_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default();

                        if name != DOCKER_BACKING_FS_BLOCK_DEV && name != DOCKER_METADATA_DB {
                            volumes.push(path);
                        }
                    }
                }
            }
        }

        Ok(volumes)
    }

    /// Validate that paths exist and are accessible
    pub fn validate_and_filter_paths(
        paths: Vec<PathBuf>,
    ) -> Result<Vec<PathBuf>, BackupServiceError> {
        let mut valid_paths = Vec::new();
        let mut skip_count = 0;

        for path in paths {
            if !path.exists() {
                warn!(path = %path.display(), "Path does not exist, skipping");
                skip_count += 1;
                continue;
            }

            valid_paths.push(path);
        }

        if skip_count > 0 {
            info!(skip_count = %skip_count, "Skipped non-existent paths");
        }

        Ok(valid_paths)
    }
}

/// Path mapping utilities (extracted from helpers.rs PathMapper)
pub struct PathMapper;

impl PathMapper {

    /// Convert native filesystem path to repository subpath
    pub fn path_to_repo_subpath(path: &Path) -> Result<String, BackupServiceError> {
        let path_str = path.to_string_lossy();

        let result = if let Some(stripped) = path_str.strip_prefix("/home/") {
            let parts: Vec<&str> = stripped.split('/').collect();
            if parts.is_empty() {
                "user_home".to_string()
            } else {
                let username = parts[0];
                if parts.len() == 1 {
                    format!("user_home/{}", username)
                } else {
                    let subdir = parts[1..].join("_");
                    format!("user_home/{}/{}", username, subdir)
                }
            }
        } else if let Some(stripped) = path_str.strip_prefix("/mnt/docker-data/volumes/") {
            let volume_path = stripped;
            if volume_path.is_empty() {
                "docker_volume".to_string()
            } else {
                format!("docker_volume/{}", volume_path.replace('/', "_"))
            }
        } else {
            let system_path = path_str.trim_start_matches('/');
            if system_path.is_empty() {
                "system".to_string()
            } else {
                format!("system/{}", system_path.replace('/', "_"))
            }
        };

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_repo_subpath() -> Result<(), BackupServiceError> {
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/home/tim"))?,
            "user_home/tim"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/home/user/.local/share/My Documents"))?,
            "user_home/user/.local_share_My Documents"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/home/tim/my/deep/path"))?,
            "user_home/tim/my_deep_path"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/mnt/docker-data/volumes/my app data"))?,
            "docker_volume/my app data"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/usr/share/applications/Google Chrome"))?,
            "system/usr_share_applications_Google Chrome"
        );
        Ok(())
    }


    // Additional core tests kept, but most bloat removed
    #[test]
    fn test_comprehensive_path_conversion() -> Result<(), BackupServiceError> {
        let test_cases = vec![
            ("/mnt/docker-data/volumes/complex/nested/volume", "docker_volume/complex_nested_volume"),
            ("/var/log/nginx/access", "system/var_log_nginx_access"),
            ("/home/user/Projects/rust/my-project", "user_home/user/Projects_rust_my-project"),
        ];

        for (native_path, expected_repo_path) in test_cases {
            let result = PathMapper::path_to_repo_subpath(Path::new(native_path))?;
            assert_eq!(result, expected_repo_path, "Failed for path: {}", native_path);
        }
        Ok(())
    }

    #[test]
    fn test_validate_and_filter_paths_logic() -> Result<(), BackupServiceError> {
        let test_paths = vec![
            PathBuf::from("/nonexistent/path1"),
            PathBuf::from("/nonexistent/path2"),
        ];

        let result = PathUtilities::validate_and_filter_paths(test_paths)?;
        assert_eq!(result.len(), 0); // All paths should be filtered out
        Ok(())
    }
}
