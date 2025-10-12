# CLAUDE.md

This file teaches AI coding assistants how this repository works end-to-end. It is authoritative and code-accurate. Prefer this over assumptions.

## Quick Dev Commands

```bash
# Build and test
cargo build --release
cargo test

# Run with debug logging
RUST_LOG=debug cargo run -- <command>

# Lints and formatting
cargo check
cargo clippy
cargo fmt
```

## What this project does

Rust CLI to orchestrate restic backups to S3-compatible storage (AWS S3, Cloudflare R2, MinIO). It:

- Runs backups for configured paths plus auto-discovered Docker volumes (from the hardcoded path `/mnt/docker-data/volumes/`)
- Lists backups per host (human or JSON)
- Restores interactively with a 5-phase guided flow
- Organizes repositories under hostname and category: `user_home`, `docker_volume`, `system`

External dependencies: system `restic` and `aws` CLI.

## CLI surface (src/main.rs)

Subcommands (via `clap`):

- `run [paths]`: Run backup. Optional `paths` is comma-separated to add to configured paths.
- `list [--host HOST] [--json]`: List repos and recent snapshots for a host (default: current host).
- `restore [--host H] [--path P] [--timestamp ISO8601]`: Interactive restore, optionally pre-filled.
- `size <path>`: Show raw-data size of latest snapshot for a path.
- `hosts`: List available hosts in the repository.
- `init`: Create a sample `.env` in the CWD.

Logging to stdout and rotating file `./logs/restic-backup.log.YYYY-MM-DD` (via `tracing`).

## Configuration model (src/config.rs)

- Required env vars:
  - `RESTIC_PASSWORD`
  - `RESTIC_REPO_BASE` (e.g., `s3:https://<endpoint>/<bucket>[/base]`)
  - `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`
  - `AWS_S3_ENDPOINT` (fallback if parsing repo base fails)
- Optional env vars:
  - `AWS_DEFAULT_REGION` (default `auto`)
  - `BACKUP_PATHS` (comma-separated absolute paths)
  - `BACKUP_HOSTNAME` (defaults to system hostname)

Env preload order at process start (unless `RBS_NO_DOTENV=1`):

1. `/etc/restic-backup.env` (literal key=value line parsing)
2. `.env` in CWD

Key helpers:

- `Config::s3_endpoint()` derives endpoint from `RESTIC_REPO_BASE` (e.g., `s3:https://minio.example.com/bucket/path` → `https://minio.example.com`). Falls back to `AWS_S3_ENDPOINT` if parsing fails.
- `Config::s3_bucket()` extracts the bucket from `RESTIC_REPO_BASE` (error if not extractable).
- `Config::s3_base_path()` extracts any path suffix after the bucket (may be empty).
- `Config::get_repo_url(subpath)` builds final restic repo URL: `<RESTIC_REPO_BASE>/<hostname>/<subpath>`.
- `Config::set_aws_env()` exports `AWS_*` and `RESTIC_PASSWORD` for child processes.

## Path mapping and categories

- Categories are determined by absolute path prefix (see `repository.rs` and `shared/paths.rs`):
  - `user_home`: paths under `/home/<user>/...`
  - `docker_volume`: paths under `/mnt/docker-data/volumes/...`
  - `system`: everything else (including `/`, `/etc`, `/var`, etc.)
- Path → repo subpath (`PathMapper::path_to_repo_subpath`):
  - `/home/<user>/a/b` → `user_home/<user>/a_b`
  - `/mnt/docker-data/volumes/<vol>/a/b` → `docker_volume/<vol>_a_b`
  - `/etc/nginx` → `system/etc_nginx`
- `BackupRepo::category()` mirrors the same rules.
- Tags used for `restic backup` (see `determine_backup_tag`): `user-path`, `docker-volume`, `system-path`.

## Command execution (src/shared/commands.rs)

- `CommandExecutor` runs commands with proper env and error mapping.
- `execute_aws_command(args, context)`: spawns `aws` with `AWS_*` env, returns stdout or maps stderr via `BackupServiceError::from_stderr`.
- `execute_restic_command(repo_url, args, context, show_live_output)`:
  - When `show_live_output=true` (e.g., restore or live backup), runs `restic` with inherited stdio and checks exit status.
  - When `false`, captures stdout/stderr.
- `ResticCommandExecutor` convenience methods:
  - `init_if_needed()` → `restic init` if snapshots query shows repo missing
  - `repo_exists()`
  - `backup(path, hostname, show_live_output)`
  - `snapshots()` → `restic snapshots --json`
  - `restore(snapshot_id, --path, --target)` (live output)
  - `stats(path)` → parse `restic stats latest --mode raw-data --json` → `total_size`
- `S3CommandExecutor`:
  - `list_directories("prefix")` → `aws s3 ls s3://<bucket>/<prefix>/ --endpoint-url <endpoint>` and parse `PRE <dir>/` lines
  - `get_hosts()` uses `Config::s3_base_path()` + `list_directories`

## Workflows

### Backup (src/shared/backup_workflow.rs)

1. Set AWS env and validate credentials (`aws s3 ls s3://<bucket>/ --endpoint-url <endpoint>`)
2. Build path list: `BACKUP_PATHS` + CLI-added paths + discovered Docker volumes
3. Filter non-existent paths
4. For each path: map → repo subpath → repo URL → `restic init` if needed → `restic backup` (live output) with tag
5. Summarize successes/skips

Docker discovery (src/shared/paths.rs): scan the hardcoded path `/mnt/docker-data/volumes/`, skipping `backingFsBlockDev` and `metadata.db`.

### List (src/list.rs, src/shared/display.rs, src/shared/operations.rs)

- Validate credentials
- Discover repositories for host by category via S3 listing
- In parallel, query `restic snapshots --json` for each repo to resolve the actual native path and collect snapshot metadata
- Output:
  - JSON: `{ host, repositories: [{ path, category, snapshot_count }], snapshots: [{ time, path, id }] }`
  - Human: grouped counts by category + recent snapshot timeline (latest 20 minutes)

### Restore (src/shared/restore_workflow.rs)

1. Set AWS env and validate credentials
2. Host selection (from S3); default to current host if present
3. Repository discovery and snapshot collection (parallel)
4. Repository selection: all, by category, multi-select, or single; optional `--path` pre-filter
5. Timestamp selection: 5-minute windows grouped from snapshot times; optional `--timestamp` ISO-8601
6. Restore best snapshot per repo to `/tmp/restic/interactive` (last ≤5 min window match, else closest prior)
7. Post-restore action: copy, move, or leave in place. Copy/move attempts to replace originals safely and clean up.

Empty restore handling: if `restic restore` indicates `0 B` and target directory is empty, it logs as an empty-volume restore.

## Error handling (src/errors.rs)

- `BackupServiceError` classifies errors: authentication, network, repository-not-found, command missing/failure, config errors, and wrapped contexts
- `from_stderr(stderr, context)` inspects lowercased stderr for known substrings and maps accordingly

## Logging

- `tracing` to stdout + rotating daily file `./logs/restic-backup.log`
- `RUST_LOG` via `tracing_subscriber::EnvFilter` (default `info`)

## NixOS integration (nixos-module.nix)

The flake exposes a NixOS module with two interfaces:

- Low-level: `services.restic_backup` (full control)
- Thin wrapper: `services.restic-backup-service` (simple interface) → maps to `services.restic_backup`

### Wrapper interface (recommended)

```nix
{
  imports = [ inputs.restic-backup-service.nixosModules.default ];

  # sops-nix: creates an env-style file with required uppercase keys
  sops.secrets.resticENV = {};

  services.restic-backup-service = {
    enable = true;
    backupTime = "06:30";           # OnCalendar string (e.g., "06:30", "daily")
    backupPaths = [ "/home/user/Documents" "/home/user/.config" ];
    secret_file_path = "/run/secrets/resticENV"; # path to the env file
  };
}
```

Important details:

- The runner script exports non-secrets (`AWS_DEFAULT_REGION`, `BACKUP_PATHS`, optional `BACKUP_HOSTNAME`) from a generated file, but for security it does not `source` the secrets file directly.
- The binary itself preloads env files from `/etc/restic-backup.env` and `.env`.
- Therefore, make the secrets file available at `/etc/restic-backup.env` so the binary reads it automatically, or create a symlink:

```nix
# Ensure the binary sees the env at /etc/restic-backup.env
{ config, ... }:
{
  environment.etc."restic-backup.env".source = config.sops.secrets.resticENV.path;
}
```

Alternatively, set `secret_file_path = "/etc/restic-backup.env"` so the same file path is used everywhere.

### Low-level interface

```nix
services.restic_backup = {
  enable = true;
  backupPaths = [ "/home/user/Documents" "/home/user/.config" ];
  hostname = null;                   # optional override

  restic = {
    passwordFile = null;             # optional if provided in env file
    repoBase = null;                 # optional override of RESTIC_REPO_BASE
  };

  aws = {
    accessKeyIdFile = null;          # optional if provided in env file
    secretAccessKeyFile = null;      # optional if provided in env file
    s3Endpoint = null;               # optional override of AWS_S3_ENDPOINT
    defaultRegion = "auto";
  };

  schedule = null;                   # OnCalendar, enables timer if non-null
  extraArgs = [ ];                   # passed to the CLI
  user = "root"; group = "root";

  # REQUIRED: absolute path to env-style secrets file (validated for readability)
  secret_file_path = "/etc/restic-backup.env";
};
```

Systemd details:

- Service `restic-backup` is `Type=oneshot` with restrictive sandboxing (read/write `/tmp`, `/var/log`, needs access to backup paths)
- Optional timer `restic-backup.timer` when `schedule`/`backupTime` is set (uses `OnCalendar`, `Persistent=true`, random delay)
- The module adds a `restic-backup-service-env` wrapper to `environment.systemPackages` for manual CLI use with the same non-secret env preloaded

## JSON output shape (for tooling)

From `list --json`:

```json
{
  "host": "<hostname>",
  "repositories": [
    {
      "path": "/path",
      "category": "user_home|docker_volume|system",
      "snapshot_count": 0
    }
  ],
  "snapshots": [{ "time": "RFC3339", "path": "/path", "id": "<short_id>" }]
}
```

## Gotchas and invariants

- CLI requires `restic` and `aws` in PATH; the NixOS package wrapper sets PATH via `makeWrapper`.
- `RESTIC_REPO_BASE` must be an `s3:` URL. Endpoint/bucket/base are extracted heuristically; invalid formats fall back or error as appropriate.
- Restore destination is fixed at `/tmp/restic/interactive` and is cleared before restore (with a user prompt when non-empty).
- Timestamp selection groups by 5-minute windows; non-interactive `--timestamp` must be ISO-8601.
- Paths with spaces/special characters are fully supported across mapping, S3 discovery, and display.
