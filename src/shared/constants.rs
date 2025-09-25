// Shared constants used across the backup service

/// Common directory paths
pub const HOME_DIR_WITH_SLASH: &str = "/home/";
pub const DOCKER_VOLUMES_DIR: &str = "/mnt/docker-data/volumes";
pub const DOCKER_VOLUMES_DIR_WITH_SLASH: &str = "/mnt/docker-data/volumes/";

/// Repository categories
pub const CATEGORY_USER_HOME: &str = "user_home";
pub const CATEGORY_DOCKER_VOLUME: &str = "docker_volume";
pub const CATEGORY_SYSTEM: &str = "system";

/// Docker volume exclusions
pub const DOCKER_BACKING_FS_BLOCK_DEV: &str = "backingFsBlockDev";
pub const DOCKER_METADATA_DB: &str = "metadata.db";
