# Restic Backup Service

Rust CLI to run restic backups to S3-compatible storage (e.g., Cloudflare R2), list backups, and restore interactively. Paths are organized by category: `user_home`, `docker_volume`, and `system`; Docker volumes are auto-discovered from the hardcoded path `/mnt/docker-data/volumes/`.

## Requirements

- `restic` and `aws` CLI in PATH
- S3-compatible storage

## Configuration (env)

The binary preloads env files `/etc/restic-backup-nonsecret.env`, a secrets file path from `BACKUP_SECRETS_FILE` if set, and `.env` (unless `RBS_NO_DOTENV=1`). Required env vars (keys must be CAPITALIZED exactly as shown):

```env
RESTIC_PASSWORD=...
RESTIC_REPO_BASE=s3:https://<endpoint>/<bucket>[/optional/base]
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
AWS_DEFAULT_REGION=auto
AWS_S3_ENDPOINT=https://<endpoint>
# Optional
BACKUP_PATHS=/path/one,/path/two
BACKUP_HOSTNAME=custom-host
# Restic excludes (optional; official restic flags)
# Path to an exclude file (one pattern per line)
BACKUP_EXCLUDE_FILE=/etc/restic-backup.exclude
# Comma-separated list of marker filenames for --exclude-if-present
BACKUP_EXCLUDE_IF_PRESENT=.nobackup,CACHEDIR.TAG
# Exclude files larger than this size (e.g., 100M, 2G)
BACKUP_EXCLUDE_LARGER_THAN=2G
```

Create a sample `.env`:

```bash
restic-backup-service init
```

## CLI

```bash
# Backup all configured paths (+ auto-discovered Docker volumes from /mnt/docker-data/volumes/)
restic-backup-service run

# Backup additional paths (comma-separated)
restic-backup-service run /path/one,/path/two

# List backups (human) or JSON
restic-backup-service list
restic-backup-service list --json

# List available hosts
restic-backup-service hosts

# Size estimate for latest snapshot of a path
restic-backup-service size /path/one

# Interactive restore (host → repositories → timestamp → restore)
restic-backup-service restore

# Non-interactive restore
restic-backup-service restore --host HOST --path "/path/one" --timestamp "2025-01-15T10:30:00Z"
```

Logs: `./logs/restic-backup.log.YYYY-MM-DD` and stdout.

## NixOS (flake module)

Use the module via your flake and the wrapper interface `services.restic-backup-service`.

Add input:

```nix
{
  inputs.restic-backup-service = {
    url = "github:timlisemer/restic-backup-service";
    inputs.nixpkgs.follows = "nixpkgs-stable";
  };
}
```

Import and configure:

```nix
{ inputs, ... }:
{
  imports = [ inputs.restic-backup-service.nixosModules.default ];

  # sops-nix: creates /run/secrets/resticENV with env-style content
  sops.secrets.resticENV = {};

  services.restic-backup-service = {
    enable = true;
    backupTime = "06:30";                # OnCalendar (e.g., "06:30", "daily")
    backupPaths = [
      "/home/user/Documents"
      "/home/user/.config"
    ];
    secret_file_path = "/run/secrets/resticENV";  # env file path

    # Official restic exclude support
    # Option 1: provide patterns; module writes /etc/restic-backup.exclude and sets BACKUP_EXCLUDE_FILE
    exclude.patterns = [
      "*.vk3"                  # by extension
      "**/node_modules/**"     # recursive folder pattern
      "tmp/"                   # subfolder relative to each source
      "My Exact File.txt"      # exact filename match anywhere
    ];
    # Option 2: use your own exclude file and point to it
    # exclude.file = "/etc/my-restic.exclude";
    # Optional: also exclude directories containing these marker files
    exclude.ifPresent = [ "CACHEDIR.TAG" ".nobackup" ];
    # Optional: skip very large files
    exclude.largerThan = "2G";
  };
}
```

### Excluding files and directories (restic)

This project supports restic's official exclude mechanisms during `backup`:

- `--exclude-file <file>`: one pattern per line, comments `#` and blanks allowed.
- `--exclude-if-present <name>`: skip any directory that contains the given file.
- `--exclude-larger-than <size>`: skip files larger than size like `100M`, `2G`.

Pattern tips (restic):

- Patterns are matched against full paths; leading `/` anchors to the source root.
- `*` matches inside a single path segment; `**` crosses directory boundaries.
- Examples: `*.vk3`, `**/node_modules/**`, `tmp/`, `My Exact File.txt`, `/home/user/Downloads/*`.

NixOS integration writes a persistent non-secret env file at `/etc/restic-backup-nonsecret.env` and, if `services.restic_backup.exclude.patterns` is set, a generated exclude file at `/etc/restic-backup.exclude`. Manual `restic-backup-service run` loads these automatically.

Example secret file content (uppercase keys):

```env
RESTIC_PASSWORD=redacted
RESTIC_REPO_BASE=s3:https://redacted.r2.cloudflarestorage.com/restic
AWS_ACCESS_KEY_ID=redacted
AWS_SECRET_ACCESS_KEY=redacted
AWS_DEFAULT_REGION=auto
AWS_S3_ENDPOINT=https://redacted.r2.cloudflarestorage.com
```

Notes:

- The service runs as oneshot `restic-backup` and can be scheduled by the timer from `backupTime`.
- Ensure the secret file exists and is readable by the service user (default `root`).
- Docker volumes are discovered from the hardcoded path `/mnt/docker-data/volumes/`.
