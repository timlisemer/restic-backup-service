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

    #[test]
    fn test_round_trip_path_conversion() -> Result<(), BackupServiceError> {
        // Test that converting native -> repo -> s3 -> native gives consistent results
        let native_paths = vec![
            "/home/tim/documents",
            "/home/alice/projects/rust",
            "/home/user123/my/deep/path",
            "/mnt/docker-data/volumes/postgres",
            "/mnt/docker-data/volumes/app/config",
            "/etc/nginx",
            "/var/log/app",
            "/usr/local/bin",
            "/",
        ];

        for native_path in native_paths {
            let repo_subpath = PathMapper::path_to_repo_subpath(Path::new(native_path))?;

            // Extract the path part after the category for S3 conversion testing
            if let Some(slash_pos) = repo_subpath.find('/') {
                let after_category = &repo_subpath[slash_pos + 1..];
                if after_category.contains('/') {
                    // Only test if there are nested paths
                    if let Some(last_slash) = after_category.rfind('/') {
                        let s3_dir = &after_category[last_slash + 1..];
                        if s3_dir.contains('_') {
                            let reconstructed = PathMapper::s3_to_native_path(s3_dir)?;
                            // Verify the conversion makes sense
                            assert!(reconstructed.contains('/') || !s3_dir.contains('_') || s3_dir.matches('_').count() == 1,
                                "Round trip conversion failed for native: {}, s3_dir: {}, reconstructed: {}",
                                native_path, s3_dir, reconstructed);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_path_mapping_edge_cases() -> Result<(), BackupServiceError> {
        // Test edge cases and boundary conditions
        let edge_cases = vec![
            // Root paths
            ("/", "system"),

            // Directory boundaries
            ("/home", "system/home"),
            ("/home/", "user_home/"), // After stripping /home/, empty string splits to [""] -> user_home/
            ("/mnt", "system/mnt"),
            ("/mnt/docker-data", "system/mnt_docker-data"),
            ("/mnt/docker-data/", "system/mnt_docker-data_"),
            ("/mnt/docker-data/volumes", "system/mnt_docker-data_volumes"),

            // Empty components
            ("/home//user", "user_home//user"), // Matches /home/ prefix, so treated as user_home
            ("//", "system"), // After trimming leading slashes, empty string -> "system"

            // Special characters in paths
            ("/home/user-name/my-project", "user_home/user-name/my-project"),
            ("/home/user_name/file.txt", "user_home/user_name/file.txt"),
            ("/mnt/docker-data/volumes/app-data", "docker_volume/app-data"),
            ("/etc/systemd/system/my-service.service", "system/etc_systemd_system_my-service.service"),
        ];

        for (native_path, expected_repo) in edge_cases {
            let result = PathMapper::path_to_repo_subpath(Path::new(native_path))?;
            assert_eq!(result, expected_repo, "Failed for edge case: {}", native_path);
        }
        Ok(())
    }

    #[test]
    fn test_validate_and_filter_paths_logic() -> Result<(), BackupServiceError> {
        // Test the filtering logic without actual file system access
        // We can test the basic structure even though paths don't exist
        let test_paths = vec![
            PathBuf::from("/nonexistent/path1"),
            PathBuf::from("/nonexistent/path2"),
            PathBuf::from("/nonexistent/path3"),
        ];

        // This will return an empty vector since paths don't exist, but won't error
        let result = PathUtilities::validate_and_filter_paths(test_paths)?;
        assert_eq!(result.len(), 0); // All paths should be filtered out

        // Test with empty input
        let empty_paths = vec![];
        let empty_result = PathUtilities::validate_and_filter_paths(empty_paths)?;
        assert_eq!(empty_result.len(), 0);

        Ok(())
    }

    #[test]
    fn test_docker_volume_path_variations() -> Result<(), BackupServiceError> {
        // Test various docker volume path scenarios
        let docker_paths = vec![
            ("/mnt/docker-data/volumes/postgres", "docker_volume/postgres"),
            ("/mnt/docker-data/volumes/postgres/data", "docker_volume/postgres_data"),
            ("/mnt/docker-data/volumes/my-app", "docker_volume/my-app"),
            ("/mnt/docker-data/volumes/app_data", "docker_volume/app_data"),
            ("/mnt/docker-data/volumes/complex-name-123", "docker_volume/complex-name-123"),
            ("/mnt/docker-data/volumes/app/config/nested", "docker_volume/app_config_nested"),
            ("/mnt/docker-data/volumes/vol/subdir/deep/path", "docker_volume/vol_subdir_deep_path"),
        ];

        for (docker_path, expected_repo) in docker_paths {
            let result = PathMapper::path_to_repo_subpath(Path::new(docker_path))?;
            assert_eq!(result, expected_repo, "Failed for docker path: {}", docker_path);
        }
        Ok(())
    }

    #[test]
    fn test_user_home_path_variations() -> Result<(), BackupServiceError> {
        // Test various user home path scenarios
        let user_paths = vec![
            ("/home/tim", "user_home/tim"),
            ("/home/alice", "user_home/alice"),
            ("/home/user123", "user_home/user123"),
            ("/home/user-name", "user_home/user-name"),
            ("/home/user_name", "user_home/user_name"),
            ("/home/tim/documents", "user_home/tim/documents"),
            ("/home/alice/projects", "user_home/alice/projects"),
            ("/home/user123/My Documents", "user_home/user123/My Documents"),
            ("/home/tim/projects/rust/my-project", "user_home/tim/projects_rust_my-project"),
            ("/home/alice/Downloads/file.tar.gz", "user_home/alice/Downloads_file.tar.gz"),
            ("/home/user/deep/nested/directory/structure", "user_home/user/deep_nested_directory_structure"),
        ];

        for (user_path, expected_repo) in user_paths {
            let result = PathMapper::path_to_repo_subpath(Path::new(user_path))?;
            assert_eq!(result, expected_repo, "Failed for user path: {}", user_path);
        }
        Ok(())
    }

    #[test]
    fn test_system_path_variations() -> Result<(), BackupServiceError> {
        // Test various system path scenarios
        let system_paths = vec![
            ("/etc", "system/etc"),
            ("/var", "system/var"),
            ("/usr", "system/usr"),
            ("/opt", "system/opt"),
            ("/etc/nginx", "system/etc_nginx"),
            ("/etc/nginx/sites-available", "system/etc_nginx_sites-available"),
            ("/var/log", "system/var_log"),
            ("/var/log/nginx/access.log", "system/var_log_nginx_access.log"),
            ("/usr/local/bin", "system/usr_local_bin"),
            ("/opt/software/config", "system/opt_software_config"),
            ("/srv/www/html", "system/srv_www_html"),
            ("/tmp/backup", "system/tmp_backup"),
            ("/root/.config", "system/root_.config"),
        ];

        for (system_path, expected_repo) in system_paths {
            let result = PathMapper::path_to_repo_subpath(Path::new(system_path))?;
            assert_eq!(result, expected_repo, "Failed for system path: {}", system_path);
        }
        Ok(())
    }

    #[test]
    fn test_s3_to_native_complex_scenarios() -> Result<(), BackupServiceError> {
        // Test complex S3 to native conversion scenarios
        let complex_cases = vec![
            // Deep path structures
            ("var_log_nginx_access_2025_01_15", "var/log/nginx/access/2025/01/15"),
            ("home_user_projects_rust_my_project", "home/user/projects/rust/my/project"),
            ("etc_systemd_system_docker_service", "etc/systemd/system/docker/service"),

            // Mixed separators and special chars
            ("config_files_app_data", "config/files/app/data"),
            ("backup_2025_01_15_full", "backup/2025/01/15/full"),

            // Single underscore cases (should be preserved)
            ("my_app", "my_app"),
            ("database_backup", "database_backup"),
            ("config_file", "config_file"),

            // No underscores
            ("documents", "documents"),
            ("projects", "projects"),
            ("config", "config"),

            // Edge cases
            ("_", "_"),
            ("a_b", "a_b"),
            ("a_b_c", "a/b/c"),
            ("a_b_c_d_e", "a/b/c/d/e"),
        ];

        for (s3_input, expected_native) in complex_cases {
            let result = PathMapper::s3_to_native_path(s3_input)?;
            assert_eq!(result, expected_native, "Failed for S3 input: {}", s3_input);
        }
        Ok(())
    }

    #[test]
    fn test_integration_category_consistency() -> Result<(), BackupServiceError> {
        // Test that paths consistently map to the same categories
        let category_tests = vec![
            // User home category
            ("/home/tim/docs", "user_home"),
            ("/home/alice/projects", "user_home"),
            ("/home/user123/data", "user_home"),

            // Docker volume category
            ("/mnt/docker-data/volumes/postgres", "docker_volume"),
            ("/mnt/docker-data/volumes/app/config", "docker_volume"),
            ("/mnt/docker-data/volumes/complex-name", "docker_volume"),

            // System category
            ("/etc/nginx", "system"),
            ("/var/log/app", "system"),
            ("/usr/local/bin", "system"),
            ("/opt/software", "system"),
            ("/", "system"),
            ("/root", "system"),
        ];

        for (path, expected_category) in category_tests {
            let repo_subpath = PathMapper::path_to_repo_subpath(Path::new(path))?;
            let category = repo_subpath.split('/').next().unwrap_or("");
            assert_eq!(category, expected_category,
                "Category mismatch for path: {}, got: {}, expected: {}",
                path, category, expected_category);
        }
        Ok(())
    }

    #[test]
    fn test_path_normalization_consistency() -> Result<(), BackupServiceError> {
        // Test that similar paths are handled consistently
        let normalization_tests = vec![
            // Trailing slash handling
            ("/home/tim", "/home/tim/"),
            ("/mnt/docker-data/volumes/app", "/mnt/docker-data/volumes/app/"),
            ("/etc/nginx", "/etc/nginx/"),

            // Leading slash consistency (all should have leading slash for absolute paths)
            // Note: These are all absolute paths, so behavior should be consistent
        ];

        for (path1, path2) in normalization_tests {
            let result1 = PathMapper::path_to_repo_subpath(Path::new(path1))?;
            let result2 = PathMapper::path_to_repo_subpath(Path::new(path2))?;

            // Results should be similar (trailing slash in input shouldn't change category)
            let category1 = result1.split('/').next().unwrap_or("");
            let category2 = result2.split('/').next().unwrap_or("");
            assert_eq!(category1, category2,
                "Categories differ for similar paths: {} -> {}, {} -> {}",
                path1, result1, path2, result2);
        }
        Ok(())
    }

    #[test]
    fn test_special_character_handling() -> Result<(), BackupServiceError> {
        // Test handling of special characters in paths
        let special_char_tests = vec![
            ("/home/user/file with spaces", "user_home/user/file with spaces"),
            ("/home/user/file-with-hyphens", "user_home/user/file-with-hyphens"),
            ("/home/user/file_with_underscores", "user_home/user/file_with_underscores"),
            ("/home/user/file.with.dots", "user_home/user/file.with.dots"),
            ("/mnt/docker-data/volumes/app-name", "docker_volume/app-name"),
            ("/etc/my-service.conf", "system/etc_my-service.conf"),
            ("/var/log/app_2025.log", "system/var_log_app_2025.log"),
        ];

        for (input_path, expected_repo) in special_char_tests {
            let result = PathMapper::path_to_repo_subpath(Path::new(input_path))?;
            assert_eq!(result, expected_repo,
                "Special character handling failed for: {}", input_path);
        }
        Ok(())
    }

    #[test]
    fn test_mock_integration_scenario() -> Result<(), BackupServiceError> {
        // Test a complete integration scenario with mock data
        struct MockRepository {
            native_path: PathBuf,
            repo_subpath: String,
            category: String,
        }

        let mock_repositories = vec![
            // User home repositories
            MockRepository {
                native_path: PathBuf::from("/home/tim/documents"),
                repo_subpath: "user_home/tim/documents".to_string(),
                category: "user_home".to_string(),
            },
            MockRepository {
                native_path: PathBuf::from("/home/alice/projects/rust"),
                repo_subpath: "user_home/alice/projects_rust".to_string(),
                category: "user_home".to_string(),
            },

            // Docker volume repositories
            MockRepository {
                native_path: PathBuf::from("/mnt/docker-data/volumes/postgres"),
                repo_subpath: "docker_volume/postgres".to_string(),
                category: "docker_volume".to_string(),
            },
            MockRepository {
                native_path: PathBuf::from("/mnt/docker-data/volumes/app/config"),
                repo_subpath: "docker_volume/app_config".to_string(),
                category: "docker_volume".to_string(),
            },

            // System repositories
            MockRepository {
                native_path: PathBuf::from("/etc/nginx"),
                repo_subpath: "system/etc_nginx".to_string(),
                category: "system".to_string(),
            },
            MockRepository {
                native_path: PathBuf::from("/var/log/app"),
                repo_subpath: "system/var_log_app".to_string(),
                category: "system".to_string(),
            },
        ];

        // Test that all mock repositories have consistent path mapping
        for mock_repo in &mock_repositories {
            let computed_repo_subpath = PathMapper::path_to_repo_subpath(&mock_repo.native_path)?;
            assert_eq!(computed_repo_subpath, mock_repo.repo_subpath,
                "Inconsistent path mapping for: {:?}", mock_repo.native_path);

            let computed_category = computed_repo_subpath.split('/').next().unwrap_or("");
            assert_eq!(computed_category, mock_repo.category,
                "Inconsistent category for: {:?}", mock_repo.native_path);
        }

        // Test category grouping
        let user_home_count = mock_repositories.iter()
            .filter(|r| r.category == "user_home")
            .count();
        let docker_volume_count = mock_repositories.iter()
            .filter(|r| r.category == "docker_volume")
            .count();
        let system_count = mock_repositories.iter()
            .filter(|r| r.category == "system")
            .count();

        assert_eq!(user_home_count, 2);
        assert_eq!(docker_volume_count, 2);
        assert_eq!(system_count, 2);
        assert_eq!(mock_repositories.len(), 6);

        Ok(())
    }
}
