# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Building and Testing
```bash
# Build the application
cargo build --release

# Run all tests (comprehensive test suite with 900+ lines)
cargo test

# Run specific test module
cargo test repository::tests

# Run with debug logging
RUST_LOG=debug cargo run -- <command>

# Check code quality
cargo check
cargo clippy

# Format code
cargo fmt
```

### Running the Application
```bash
# Initialize configuration
cargo run -- init

# Run backup with specific paths
cargo run -- run /path/to/backup

# Interactive restore
cargo run -- restore

# List backups with JSON output
cargo run -- list --json
```

## High-Level Architecture

### Core Design Principles
This is a **modular async Rust application** built around a **3-tier architecture**:

1. **CLI Layer** (`main.rs`) - Command parsing and dispatch
2. **Workflow Layer** (`shared/{backup,restore}_workflow.rs`) - Multi-phase orchestration
3. **Operations Layer** (`shared/{commands,operations}.rs`) - Core business logic

The modular design separates concerns across the `shared/` modules with clear interfaces between components.

### Key Architectural Components

**Path Categorization System** (`repository.rs` + `shared/paths.rs`):
- Automatically classifies filesystem paths into `user_home`, `docker_volume`, `system`
- Maps native paths to S3 repository structure with intelligent path encoding
- Extensive test coverage for edge cases (whitespace, special characters, gaming directories)

**Parallel Repository Operations** (`shared/operations.rs`):
- Uses `tokio::spawn` for true concurrency when scanning repositories
- `RepositoryOperations` orchestrates concurrent scanning with progress tracking
- `SnapshotCollector` caches path mappings and extracts actual paths from restic snapshots

**Workflow Orchestration**:
- **BackupWorkflow**: 3-phase process (path preparation → parallel execution → result reporting)
- **RestoreWorkflow**: 5-phase interactive process (host selection → discovery → path selection → time window → restoration)

**Command Execution** (`shared/commands.rs`):
- Unified `CommandExecutor` base class for AWS CLI and restic operations
- Specialized executors (`ResticCommandExecutor`, `S3CommandExecutor`) with environment management
- Support for both live output (progress) and captured output modes

### Data Flow Architecture

**Repository Discovery Flow**:
```
S3 bucket scanning → UnscannedRepository → RepositoryOperations →
concurrent tokio::spawn tasks → SnapshotCollector → RepositoryData → BackupRepo/UI items
```

**Path Mapping Flow**:
```
Native filesystem path → PathMapper::path_to_repo_subpath → S3 repository structure →
Config::get_repo_url → restic operations
```

**Interactive Restore Flow**:
```
Host selection → Parallel repository scanning → UI selection workflows →
Time window grouping → Restoration with post-restore actions
```

### Configuration Architecture
- **Environment-first**: Loads from `.env` files with fallback to system environment
- **S3 URL Intelligence**: Parses various S3 provider formats (AWS, R2, MinIO) automatically
- **Hostname Resolution**: env var → system hostname → "unknown" fallback chain

### Error Handling Strategy
- **Structured Errors**: Uses `thiserror` with intelligent stderr parsing in `BackupServiceError::from_stderr`
- **Context Wrapping**: Validation context and operation-specific error types
- **Graceful Degradation**: Operations continue when individual components fail

### Testing Strategy
The codebase includes comprehensive testing focusing on:
- **Edge Case Coverage**: Whitespace paths, special characters, gaming directory structures
- **Path Categorization**: Extensive testing of the path mapping logic
- **Data Structure Integrity**: Validation of conversions between different data representations
- **Concurrent Operations**: Testing of parallel scanning and error handling

### Key Dependencies
- **tokio**: Async runtime and concurrency (`tokio::spawn` for parallel operations)
- **clap**: CLI argument parsing with subcommands
- **tracing**: Structured logging with file rotation
- **dialoguer**: Interactive UI components for restoration workflow
- **serde_json**: JSON parsing for restic command output
- **chrono**: Timestamp handling and time window calculations

### Development Notes
- **Live Output Mode**: Commands support both captured output and live progress display
- **Docker Integration**: Auto-discovery of volumes with system file filtering
- **Multi-Provider S3**: Supports AWS, Cloudflare R2, MinIO with automatic endpoint detection
- **Time Window Grouping**: 5-minute snapshot windows for intuitive restore point selection

## Repository Structure Context
The application maintains hierarchical S3 organization: `s3://bucket/[base-path/]hostname/category/specific-path/` where category is automatically determined by path analysis, not user configuration.