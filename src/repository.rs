use std::path::PathBuf;



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

