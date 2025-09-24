use std::path::PathBuf;
use crate::errors::BackupServiceError;



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
        let result = if self.native_path.starts_with("/home/") {
            "user_home"
        } else if self.native_path.starts_with("/mnt/docker-data/volumes/") {
            "docker_volume"
        } else {
            "system"
        };
        Ok(result)
    }
}

