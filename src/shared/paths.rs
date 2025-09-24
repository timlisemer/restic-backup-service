use crate::errors::BackupServiceError;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Docker volume discovery and validation utilities
pub struct PathUtilities;

impl PathUtilities {
    /// Discover and validate docker volumes, extracted from backup.rs
    pub fn discover_docker_volumes() -> Result<Vec<PathBuf>, BackupServiceError> {
        let mut volumes = Vec::new();

        let docker_volumes_path = Path::new("/mnt/docker-data/volumes");
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

                        if name != "backingFsBlockDev" && name != "metadata.db" {
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
    /// Convert S3 directory name back to native path (preserve filename underscores)
    pub fn s3_to_native_path(s3_dir: &str) -> Result<String, BackupServiceError> {
        let result = if s3_dir.matches('_').count() > 1 {
            s3_dir.replace('_', "/")
        } else {
            s3_dir.to_string()
        };
        Ok(result)
    }

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
            PathMapper::path_to_repo_subpath(Path::new("/home/tim/documents"))?,
            "user_home/tim/documents"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/home/tim/my/deep/path"))?,
            "user_home/tim/my_deep_path"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/mnt/docker-data/volumes/myapp"))?,
            "docker_volume/myapp"
        );
        assert_eq!(
            PathMapper::path_to_repo_subpath(Path::new("/etc/nginx"))?,
            "system/etc_nginx"
        );
        Ok(())
    }

    #[test]
    fn test_s3_to_native_path() -> Result<(), BackupServiceError> {
        assert_eq!(
            PathMapper::s3_to_native_path("documents")?,
            "documents"
        );
        assert_eq!(
            PathMapper::s3_to_native_path("my_deep_path")?,
            "my/deep/path"
        );
        assert_eq!(
            PathMapper::s3_to_native_path("etc_nginx_conf")?,
            "etc/nginx/conf"
        );
        assert_eq!(
            PathMapper::s3_to_native_path("single")?,
            "single"
        );
        Ok(())
    }

    #[test]
    fn test_comprehensive_path_conversion() -> Result<(), BackupServiceError> {
        // Test cases matching original NixOS logic
        let test_cases = vec![
            // Docker volume paths with nested structure
            ("/mnt/docker-data/volumes/complex/nested/volume", "docker_volume/complex_nested_volume"),

            // System paths with deep nesting
            ("/var/log/nginx/access", "system/var_log_nginx_access"),
            ("/etc/systemd/system/my-service", "system/etc_systemd_system_my-service"),

            // Edge cases for boundary paths
            ("/mnt/docker-data/volumes/", "docker_volume"),
            ("/", "system"),

            // User home with various subdirectories
            ("/home/user/Projects/rust/my-project", "user_home/user/Projects_rust_my-project"),
        ];

        for (native_path, expected_repo_path) in test_cases {
            let result = PathMapper::path_to_repo_subpath(Path::new(native_path))?;
            assert_eq!(result, expected_repo_path, "Failed for path: {}", native_path);
        }
        Ok(())
    }

    #[test]
    fn test_s3_to_native_smart_conversion() -> Result<(), BackupServiceError> {
        // Test smart conversion logic that preserves filename underscores vs path separators
        let test_cases = vec![
            // Single underscore (preserve as filename)
            ("my_file", "my_file"),
            ("docker_volume", "docker_volume"),
            ("backup_data", "backup_data"),

            // Multiple underscores (convert to path)
            ("var_log_nginx_access", "var/log/nginx/access"),
            ("home_user_documents_important", "home/user/documents/important"),

            // No underscores (unchanged)
            ("documents", "documents"),
            ("projects", "projects"),
        ];

        for (s3_dir, expected_native) in test_cases {
            let result = PathMapper::s3_to_native_path(s3_dir)?;
            assert_eq!(result, expected_native, "Failed for S3 dir: {}", s3_dir);
        }
        Ok(())
    }
}
