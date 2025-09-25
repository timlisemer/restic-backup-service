# Restic Backup Service

A Rust-based CLI application for managing restic backups with S3 (Cloudflare R2) backend storage. This tool provides automated backup, listing, and interactive restoration capabilities with the exact same repository structure as the original Nix implementation.

## Features

- **Automated Backups**: Backup configured paths, user directories, and Docker volumes
- **Interactive Restore**: User-friendly restoration with host, repository, and timestamp selection
- **S3/R2 Backend**: Supports S3-compatible storage (optimized for Cloudflare R2)
- **Repository Organization**: Maintains structured organization by host, type, and path
- **Colored Output**: Clear, color-coded feedback for all operations
- **NixOS Integration**: Designed to work seamlessly with NixOS and sops-nix

## Installation

```bash
# Clone the repository
git clone https://github.com/timlisemer/restic-backup-service.git
cd restic-backup-service

# Build the application
cargo build --release

# The binary will be at target/release/restic-backup-service
```

## Configuration

### 1. Create Configuration File

Copy the example configuration and edit with your credentials:

```bash
cp .env.example .env
# Edit .env with your actual values
```

### 2. Environment Variables

The `.env` file should contain:

```env
# Restic repository password
RESTIC_PASSWORD=your_restic_password_here

# S3/R2 Repository base URL
RESTIC_REPO_BASE=s3:https://your-bucket.r2.cloudflarestorage.com/restic

# AWS/S3 Credentials
AWS_ACCESS_KEY_ID=your_access_key_here
AWS_SECRET_ACCESS_KEY=your_secret_key_here
AWS_DEFAULT_REGION=auto
AWS_S3_ENDPOINT=https://your-bucket.r2.cloudflarestorage.com

# Backup paths (comma-separated)
BACKUP_PATHS=/home/user/documents,/home/user/projects

# Optional: Custom hostname (defaults to system hostname)
# BACKUP_HOSTNAME=my-custom-hostname
```

### 3. NixOS Integration

For NixOS users with sops-nix, you can source credentials directly:

```nix
# In your NixOS configuration
sops.secrets = {
  restic_password = {};
  restic_repo_base = {};
  restic_environment = {
    # Contains AWS credentials
  };
};
```

## Usage

### Initialize Configuration

Generate a sample `.env` file:

```bash
./restic-backup-service init
```

### Run Backup

Backup all configured paths:

```bash
./restic-backup-service run
```

Backup specific paths:

```bash
./restic-backup-service run /path/to/backup,/another/path
```

### List Backups

List backups for current host:

```bash
./restic-backup-service list
```

List backups for specific host:

```bash
./restic-backup-service list --host other-hostname
```

Output as JSON (for scripting):

```bash
./restic-backup-service list --json
```

### Interactive Restore

Start interactive restoration wizard:

```bash
./restic-backup-service restore
```

Non-interactive restore with options:

```bash
./restic-backup-service restore --host hostname --path /home/user --timestamp "2025-08-23T06:30:00Z"
```

### Show Repository Size

Check how much space a path occupies:

```bash
./restic-backup-service size /home/user/documents
```

### List Available Hosts

Show all hosts with backups:

```bash
./restic-backup-service hosts
```

## Repository Structure

The application maintains the following S3 structure:

```
s3://bucket/restic/
├── hostname1/
│   ├── user_home/
│   │   └── username/
│   │       └── subdirectory_path
│   ├── docker_volume/
│   │   └── volume_name/
│   └── system/
│       └── system_path
└── hostname2/
    └── ...
```

### Path Mapping

- `/home/user/documents` → `user_home/user/documents`
- `/home/user/my/deep/path` → `user_home/user/my_deep_path`
- `/mnt/docker-data/volumes/myapp` → `docker_volume/myapp`
- `/etc/nginx` → `system/etc_nginx`

## Docker Volume Support

The application automatically detects and backs up Docker volumes from `/mnt/docker-data/volumes/`. Non-volume entries like `backingFsBlockDev` and `metadata.db` are automatically excluded.

## Nested Repository Support

For complex directory structures, the application supports nested repositories. This is particularly useful for Docker volumes with subdirectories.

## Requirements

- Rust 1.70 or later
- `restic` command-line tool installed
- `aws` CLI tool (for S3 operations)
- S3-compatible storage (AWS S3, Cloudflare R2, MinIO, etc.)

## Safety Features

- Automatic repository initialization
- Graceful handling of missing paths
- Progress indicators for long operations
- Confirmation prompts for destructive operations
- Detailed error messages with recovery suggestions

## Performance

- Parallel backup operations for multiple paths
- Efficient S3 queries with caching
- Minimal memory footprint
- Progress bars for visual feedback

## Troubleshooting

### Missing Credentials

If you see errors about missing environment variables, ensure your `.env` file exists and contains all required values.

### Repository Not Found

The application automatically initializes repositories as needed. If you encounter issues, check your S3 credentials and network connectivity.

### Permission Errors

For system paths and Docker volumes, you may need to run with `sudo`:

```bash
sudo -E ./restic-backup-service run
```

The `-E` flag preserves environment variables.

## License

This project maintains compatibility with the original NixOS implementation and follows the same backup structure and conventions.
