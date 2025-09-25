# Restic Backup Service

A Rust-based CLI application for managing restic backups with S3-compatible storage. Built to handle complex filesystem structures including gaming directories, Docker volumes, and development environments with sophisticated path categorization and parallel processing.

## What it does

This tool automates restic backups to S3-compatible storage (primarily Cloudflare R2) with intelligent path organization and provides an interactive restoration system. It was built to replace a shell script-based backup system with something more reliable and feature-rich.

## Key Features

- **Intelligent Path Categorization**: Automatically classifies paths into `user_home`, `docker_volume`, and `system` categories with proper S3 repository structure
- **Parallel Repository Operations**: Uses tokio for concurrent repository scanning and operations
- **Interactive 5-Phase Restoration**: Host selection → repository discovery → path selection → time window selection → restoration with post-restore actions
- **Complex Path Support**: Handles paths with spaces, gaming directories (Steam, Paradox Interactive), and application data correctly
- **Docker Integration**: Auto-discovers Docker volumes with intelligent filtering of system files
- **Time Window Grouping**: Groups snapshots into 5-minute windows for intuitive restore point selection

## Architecture

The application follows a 3-tier architecture with modular design:

1. **CLI Layer** (`main.rs`) - Command parsing and dispatch
2. **Workflow Layer** (`shared/{backup,restore}_workflow.rs`) - Multi-phase orchestration
3. **Operations Layer** (`shared/{commands,operations}.rs`) - Core business logic

```
src/
├── main.rs              # CLI entry point with structured logging
├── config.rs            # S3 URL parsing supporting multiple providers
├── errors.rs            # Structured error handling with stderr parsing
├── repository.rs        # Path categorization and repository modeling
└── shared/              # Core functionality modules
    ├── commands.rs      # Unified AWS/restic command execution
    ├── operations.rs    # Parallel repository operations and scanning
    ├── backup_workflow.rs    # 3-phase backup orchestration
    ├── restore_workflow.rs   # Interactive restoration workflow
    ├── ui.rs            # Interactive selection interfaces
    ├── display.rs       # Structured output formatting
    └── paths.rs         # Path mapping and Docker volume discovery
```

## Technical Highlights

### Path Categorization System
The application includes a sophisticated path categorization system that handles real-world complexity:

```rust
// Examples of automatic path mapping:
"/home/gamer/.local/share/Paradox Interactive" → "user_home/gamer/.local_share_Paradox Interactive"
"/mnt/docker-data/volumes/my app data" → "docker_volume/my app data"
"/usr/share/applications/Visual Studio Code" → "system/usr_share_applications_Visual Studio Code"
```

This system has extensive test coverage (900+ lines) for edge cases including whitespace, special characters, and complex gaming/development directory structures.

### Concurrent Repository Scanning
Uses `tokio::spawn` for true parallelization when scanning repositories with proper progress tracking and error handling. The `RepositoryOperations` orchestrates concurrent scanning while `SnapshotCollector` caches path mappings.

### Interactive Restoration Workflow
The restore process is implemented as a 5-phase workflow:
1. Host selection from available backups
2. Concurrent repository discovery and scanning
3. Repository selection (category-based or individual)
4. Time window selection (5-minute snapshot grouping)
5. Restoration with copy/move options to original locations

### S3 Provider Support
Includes intelligent S3 URL parsing that supports multiple providers (AWS S3, Cloudflare R2, MinIO) with automatic endpoint extraction and credential management.

### Data Flow Architecture
The application follows clear data flows: S3 bucket scanning → concurrent repository discovery → snapshot collection → UI presentation. Native filesystem paths are mapped through `PathMapper::path_to_repo_subpath` to create the S3 repository structure.

## Installation

### For Users
```bash
git clone https://github.com/timlisemer/restic-backup-service.git
cd restic-backup-service
cargo build --release
```

### For Development
```bash
# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run -- <command>

# Code quality checks
cargo check && cargo clippy
```

## Configuration

Create a `.env` file or initialize with:
```bash
# Using built binary
./restic-backup-service init

# Or during development
cargo run -- init
```

Example configuration:
```env
RESTIC_PASSWORD=your_password
RESTIC_REPO_BASE=s3:https://account-id.r2.cloudflarestorage.com/bucket/restic
AWS_ACCESS_KEY_ID=your_key
AWS_SECRET_ACCESS_KEY=your_secret
AWS_DEFAULT_REGION=auto
AWS_S3_ENDPOINT=https://account-id.r2.cloudflarestorage.com
BACKUP_PATHS=/home/user/Documents,/home/user/.config,/home/user/.local/share/Steam
```

## Usage

### Backup Operations
```bash
# Backup all configured paths + auto-discovered Docker volumes
./restic-backup-service run

# Backup specific additional paths
./restic-backup-service run /path/to/backup,/another/path
```

### List Backups
```bash
# Human-readable categorized output
./restic-backup-service list

# JSON output for scripting
./restic-backup-service list --json

# List available hosts
./restic-backup-service hosts
```

### Interactive Restoration
```bash
# Launch interactive restore wizard
./restic-backup-service restore

# Non-interactive restore with specific parameters
./restic-backup-service restore --host hostname --path "/home/user/Documents" --timestamp "2024-01-15T10:30:00Z"
```

### Repository Analysis
```bash
# Check storage usage for a path
./restic-backup-service size /home/user/Documents
```

## Repository Structure

The application organizes backups in S3 with a hierarchical structure:

```
s3://bucket/[base-path/]hostname/category/specific-path/
├── hostname1/
│   ├── user_home/username/path_components/
│   ├── docker_volume/volume_name/
│   └── system/system_path_components/
└── hostname2/...
```

## Docker Integration

Automatically discovers Docker volumes in `/mnt/docker-data/volumes/` while filtering out system files (`backingFsBlockDev`, `metadata.db`). Supports volume names with spaces and special characters.

## Requirements

- Rust 1.70+
- `restic` command-line tool
- `aws` CLI tool
- S3-compatible storage

### Key Dependencies
- **tokio**: Async runtime and concurrency
- **clap**: CLI argument parsing
- **tracing**: Structured logging with file rotation
- **dialoguer**: Interactive UI components
- **thiserror**: Structured error handling

## Error Handling and Reliability

The application includes comprehensive error handling with:
- **Structured Errors**: Uses `thiserror` with intelligent stderr parsing in `BackupServiceError::from_stderr`
- **Context Wrapping**: Validation context and operation-specific error types
- **Graceful Degradation**: Operations continue when individual components fail
- **Credential Validation**: Proactive S3 credential testing before operations
- **Path Validation**: Existence checking before backup operations

Logging is implemented with `tracing` and includes file rotation to `./logs/restic-backup.log`.

## Performance

- Concurrent repository scanning with progress tracking using `tokio::spawn`
- Memory-efficient streaming operations
- Fast startup time for most commands
- Handles complex directory structures and multiple repositories efficiently

## Why Rust

This project was an exercise in building a production-quality CLI application in Rust, focusing on:
- Type safety and error handling
- Async/await patterns with tokio
- Modular architecture with clear separation of concerns
- Comprehensive testing including edge cases
- Performance optimization through parallelization