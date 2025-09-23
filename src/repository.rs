use std::path::{Path, PathBuf};

/// Determines the repository location (subpath) for a given path
pub fn path_to_repo_subpath(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    if let Some(stripped) = path_str.strip_prefix("/home/") {
        // User home paths
        let parts: Vec<&str> = stripped.split('/').collect();
        if parts.is_empty() {
            return "user_home".to_string();
        }

        let username = parts[0];
        if parts.len() == 1 {
            format!("user_home/{}", username)
        } else {
            let subdir = parts[1..].join("_");
            format!("user_home/{}/{}", username, subdir)
        }
    } else if let Some(stripped) = path_str.strip_prefix("/mnt/docker-data/volumes/") {
        // Docker volume paths
        let volume_path = stripped;
        if volume_path.is_empty() {
            "docker_volume".to_string()
        } else {
            format!("docker_volume/{}", volume_path.replace('/', "_"))
        }
    } else {
        // System paths
        let system_path = path_str.trim_start_matches('/');
        if system_path.is_empty() {
            "system".to_string()
        } else {
            format!("system/{}", system_path.replace('/', "_"))
        }
    }
}

/// Converts S3 directory name back to native path format
/// Smart conversion: preserve filename underscores, convert path separators
pub fn s3_to_native_path(s3_dir: &str) -> String {
    // If there are multiple underscores, likely has path structure, do conversion
    if s3_dir.matches('_').count() > 1 {
        s3_dir.replace('_', "/")
    } else {
        // Single or no underscores - likely just filename, preserve it
        s3_dir.to_string()
    }
}


/// Information about a backup repository
#[derive(Debug, Clone)]
pub struct BackupRepo {
    pub native_path: PathBuf,
    pub snapshot_count: usize,
}

impl BackupRepo {
    pub fn new(native_path: PathBuf) -> Self {
        Self {
            native_path,
            snapshot_count: 0,
        }
    }

    pub fn with_count(mut self, count: usize) -> Self {
        self.snapshot_count = count;
        self
    }

    pub fn category(&self) -> &'static str {
        if self.native_path.starts_with("/home/") {
            "user_home"
        } else if self.native_path.starts_with("/mnt/docker-data/volumes/") {
            "docker_volume"
        } else {
            "system"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_repo_subpath() {
        assert_eq!(
            path_to_repo_subpath(Path::new("/home/tim")),
            "user_home/tim"
        );
        assert_eq!(
            path_to_repo_subpath(Path::new("/home/tim/documents")),
            "user_home/tim/documents"
        );
        assert_eq!(
            path_to_repo_subpath(Path::new("/home/tim/my/deep/path")),
            "user_home/tim/my_deep_path"
        );
        assert_eq!(
            path_to_repo_subpath(Path::new("/mnt/docker-data/volumes/myapp")),
            "docker_volume/myapp"
        );
        assert_eq!(
            path_to_repo_subpath(Path::new("/etc/nginx")),
            "system/etc_nginx"
        );
    }

}