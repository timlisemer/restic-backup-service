{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.restic_backup;

  # Default package - will be overridden when used through flake
  defaultPackage = pkgs.rustPlatform.buildRustPackage rec {
    pname = "restic-backup-service";
    version = "1.1.5";
    src = ./.;
    cargoLock = {
      lockFile = ./Cargo.lock;
    };

    nativeBuildInputs = with pkgs; [pkg-config makeWrapper];
    buildInputs = with pkgs; [openssl];
    propagatedBuildInputs = with pkgs; [restic awscli2];

    postInstall = ''
      wrapProgram $out/bin/restic-backup-service \
        --prefix PATH : ${pkgs.lib.makeBinPath [pkgs.restic pkgs.awscli2]}
    '';
  };

  # Create environment file with non-secret configuration
  envFile = pkgs.writeText "restic-backup-env" ''
    AWS_DEFAULT_REGION=${lib.escapeShellArg cfg.aws.defaultRegion}
    BACKUP_PATHS=${lib.escapeShellArg (lib.concatStringsSep "," cfg.backupPaths)}
    ${lib.optionalString (cfg.hostname != null) ("BACKUP_HOSTNAME=" + lib.escapeShellArg cfg.hostname)}
    ${lib.optionalString (cfg.exclude.file != null) ("BACKUP_EXCLUDE_FILE=" + lib.escapeShellArg (toString cfg.exclude.file))}
    ${lib.optionalString (cfg.exclude.largerThan != null) ("BACKUP_EXCLUDE_LARGER_THAN=" + lib.escapeShellArg cfg.exclude.largerThan)}
    ${lib.optionalString (cfg.exclude.ifPresent != []) ("BACKUP_EXCLUDE_IF_PRESENT=" + lib.escapeShellArg (lib.concatStringsSep "," cfg.exclude.ifPresent))}
  '';
  # Secrets file path provided via NixOS option
  envInlineFile = cfg.secret_file_path;

  # Create a script that sources secrets and runs the backup
  backupScript = pkgs.writeShellScript "restic-backup-runner" ''
    set -euo pipefail

    # Source the main environment file
    set -a
    source ${envFile}

    # Detect and log catch-up runs for simple HH:MM schedules
    SCHEDULE="${cfg.schedule or ""}"
    if [ -n "$SCHEDULE" ] && printf '%s' "$SCHEDULE" | grep -Eq '^[0-9]{1,2}:[0-9]{2}$'; then
      now_h=$(date +%H)
      now_m=$(date +%M)
      sched_h=$(printf '%02d' "''${SCHEDULE%:*}")
      sched_m="''${SCHEDULE#*:}"
      if [ "$now_h" != "$sched_h" ] || [ "$now_m" != "$sched_m" ]; then
        echo "[restic-backup] catch-up run (schedule=$SCHEDULE, now=$(date -Is))"
      fi
    fi

    # Secrets file presence is not validated here to avoid affecting boot/rebuild.

    # Set individual secrets if provided via Nix options (overrides files)

    # Set direct configuration values (overrides both files)
    ${lib.optionalString (cfg.restic.repoBase != null) ''
      RESTIC_REPO_BASE="${cfg.restic.repoBase}"
    ''}

    ${lib.optionalString (cfg.aws.s3Endpoint != null) ''
      AWS_S3_ENDPOINT="${cfg.aws.s3Endpoint}"
    ''}

    set +a

    # Set log directory for service context
    RBS_LOG_DIR=/var/log/restic-backup
    export RBS_LOG_DIR

    # No pre-exec env validation here; failures will be handled at runtime by the program.

    # Run the backup
    exec "${cfg.package}/bin/restic-backup-service" run ${lib.concatStringsSep " " cfg.extraArgs}
  '';

  # Provide a CLI wrapper that sources the same env files for manual usage
  cliWrapper = pkgs.writeShellScriptBin "restic-backup-service-env" ''
    set -euo pipefail
    set -a
    # Load non-secret env
    source ${envFile}
    # Secrets file presence is not validated here; binary handles its own preload.
    set +a
    exec "${cfg.package}/bin/restic-backup-service" "$@"
  '';
in {
  options.services.restic_backup = {
    exclude = {
      patterns = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        example = ["*.vk3" "**/node_modules/**" "tmp/" "My Exact File.txt"];
        description = "Patterns written to /etc/restic-backup.exclude and passed via --exclude-file (restic pattern syntax).";
      };

      file = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to an existing exclude file; if set, patterns are not written to /etc. BACKUP_EXCLUDE_FILE points here.";
      };

      ifPresent = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        example = [".nobackup" "CACHEDIR.TAG"];
        description = "List of marker filenames for --exclude-if-present (applied to all sources).";
      };

      largerThan = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "2G";
        description = "Size threshold for --exclude-larger-than (e.g. 100M, 2G).";
      };
    };
    enable = lib.mkEnableOption "Restic backup service";

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPackage;
      description = "The restic-backup-service package to use";
    };

    backupPaths = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      example = ["/home/user/Documents" "/home/user/.config"];
      description = "List of paths to backup";
    };

    hostname = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Custom hostname for backups (defaults to system hostname)";
    };

    restic = {
      passwordFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = lib.literalExpression "config.sops.secrets.restic-password.path";
        description = "Path to file containing restic repository password";
      };

      repoBase = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "s3:https://account-id.r2.cloudflarestorage.com/bucket/restic";
        description = "Restic repository base URL (can also be provided via secretsFile)";
      };
    };

    aws = {
      accessKeyIdFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = lib.literalExpression "config.sops.secrets.aws-access-key.path";
        description = "Path to file containing AWS access key ID";
      };

      secretAccessKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = lib.literalExpression "config.sops.secrets.aws-secret-key.path";
        description = "Path to file containing AWS secret access key";
      };

      s3Endpoint = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "https://account-id.r2.cloudflarestorage.com";
        description = "S3 endpoint URL (can also be provided via secretsFile)";
      };

      defaultRegion = lib.mkOption {
        type = lib.types.str;
        default = "auto";
        description = "AWS default region";
      };
    };

    schedule = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "daily";
      description = "Systemd timer schedule for periodic backups (null disables timer)";
    };

    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      example = ["--verbose"];
      description = "Additional arguments to pass to restic-backup-service";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "root";
      description = "User to run the backup service as";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "root";
      description = "Group to run the backup service as";
    };

    secret_file_path = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Absolute path to the env-style secrets file used by the unit and CLI (required).";
    };
  };

  options.services."restic-backup-service" = {
    enable = lib.mkEnableOption "Simple restic-backup-service wrapper (maps to services.restic_backup)";

    backupPaths = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "List of native filesystem paths to back up.";
      example = ["/home/user/Documents" "/home/user/.config"];
    };

    backupTime = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "OnCalendar string (e.g., \"06:30\", \"daily\") or null to disable timer.";
      example = "06:30";
    };

    secret_file_path = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Absolute path to the env-style secrets file (required).";
    };

    # Mirror of exclude options for convenience when using the wrapper interface
    exclude = {
      patterns = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Patterns written to /etc/restic-backup.exclude and used via --exclude-file.";
      };
      file = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to an existing exclude file to use instead of generated one.";
      };
      ifPresent = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Marker filenames for --exclude-if-present.";
      };
      largerThan = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Size threshold passed to --exclude-larger-than (e.g. 100M, 2G).";
      };
    };
  };

  config = lib.mkMerge [
    # Map simplified interface onto services.restic_backup
    (let
      simple = config.services."restic-backup-service";
    in
      lib.mkIf simple.enable {
        services.restic_backup.enable = true;
        services.restic_backup.backupPaths = simple.backupPaths;
        services.restic_backup.schedule = simple.backupTime;
        services.restic_backup.secret_file_path = simple.secret_file_path;
        # Map exclude subtree
        services.restic_backup.exclude.patterns = simple.exclude.patterns;
        services.restic_backup.exclude.file = simple.exclude.file;
        services.restic_backup.exclude.ifPresent = simple.exclude.ifPresent;
        services.restic_backup.exclude.largerThan = simple.exclude.largerThan;
      })

    (lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = cfg.backupPaths != [];
          message = "services.restic_backup.backupPaths must not be empty";
        }
        {
          assertion = cfg.secret_file_path != null;
          message = "services.restic_backup.secret_file_path must be set to a valid secrets file path.";
        }
      ];

      systemd.services.restic-backup = {
        description = "Restic backup service";
        after = ["network-online.target"];
        wants = ["network-online.target"];

        # No file conditions; envContent is required and validated

        serviceConfig = {
          Type = "oneshot";
          User = cfg.user;
          Group = cfg.group;
          WorkingDirectory = "/"; # Explicitly set to root to match default behavior
          ExecStart = "${backupScript}";

          # Security settings
          PrivateTmp = true;
          ProtectSystem = "strict";
          ProtectHome = false; # Need access to backup paths
          ReadWritePaths = ["/tmp" "/var/log"];
          NoNewPrivileges = true;

          # Logging
          StandardOutput = "journal";
          StandardError = "journal";
          SyslogIdentifier = "restic-backup";
        };

        preStart = ''
          install -d -m 755 -o ${cfg.user} -g ${cfg.group} /var/log/restic-backup
        '';
      };

      # Optional systemd timer for scheduled backups
      systemd.timers.restic-backup = lib.mkIf (cfg.schedule != null) {
        description = "Timer for restic backup service";
        wantedBy = ["timers.target"];
        unitConfig = {
          After = ["multi-user.target" "network-online.target"];
          Wants = ["network-online.target"];
        };

        timerConfig = {
          OnCalendar = cfg.schedule;
          Persistent = true;
        };
      };

      # Ensure the package and CLI wrapper are available in the system
      environment.systemPackages = [cfg.package cliWrapper];

      # If user supplied patterns (and no custom file), write/overwrite /etc/restic-backup.exclude
      environment.etc."restic-backup.exclude" = lib.mkIf (cfg.exclude.file == null && cfg.exclude.patterns != []) {
        text = (lib.concatStringsSep "\n" cfg.exclude.patterns) + "\n";
      };

      # Plain, unquoted env file for the binary to preload (no shell quoting)
      environment.etc."restic-backup-nonsecret.env".text = let
        lines =
          [
            ("AWS_DEFAULT_REGION=" + cfg.aws.defaultRegion)
            ("BACKUP_PATHS=" + (lib.concatStringsSep "," cfg.backupPaths))
          ]
          ++ lib.optional (cfg.hostname != null) ("BACKUP_HOSTNAME=" + cfg.hostname)
          ++ lib.optional (cfg.secret_file_path != null) ("BACKUP_SECRETS_FILE=" + (toString cfg.secret_file_path))
          ++ lib.optional (cfg.exclude.file != null) ("BACKUP_EXCLUDE_FILE=" + (toString cfg.exclude.file))
          ++ lib.optional (cfg.exclude.file == null && cfg.exclude.patterns != []) "BACKUP_EXCLUDE_FILE=/etc/restic-backup.exclude"
          ++ lib.optional (cfg.exclude.largerThan != null) ("BACKUP_EXCLUDE_LARGER_THAN=" + cfg.exclude.largerThan)
          ++ lib.optional (cfg.exclude.ifPresent != []) ("BACKUP_EXCLUDE_IF_PRESENT=" + (lib.concatStringsSep "," cfg.exclude.ifPresent));
      in
        (lib.concatStringsSep "\n" lines) + "\n";

      # Show a concise summary during activation so rebuild output is informative
      system.activationScripts.resticBackupSummary = let
        sysdAnalyze = "${pkgs.systemd}/bin/systemd-analyze";
        sedBin = "${pkgs.gnused}/bin/sed";
        pathsCount = builtins.length cfg.backupPaths;
        allList = lib.concatStringsSep ", " cfg.backupPaths;
        scheduleOrEmpty =
          if cfg.schedule == null
          then ""
          else cfg.schedule;
        secretsMode = "file";
      in {
        text = ''
          echo "[restic-backup] package: ${cfg.package.pname or "restic-backup-service"} ${cfg.package.version or "unknown"}"
          echo "[restic-backup] paths configured: ${toString pathsCount} (${allList})"
          if [ -n "${scheduleOrEmpty}" ]; then
            echo "[restic-backup] timer OnCalendar: ${cfg.schedule} (systemd calendar)"
            if ${sysdAnalyze} calendar "${cfg.schedule}" >/dev/null 2>&1; then
              next_line="$(${sysdAnalyze} calendar --iterations=1 "${cfg.schedule}" 2>/dev/null | ${sedBin} -n 's/^  Next elapse: //p' | head -1)"
              [ -n "$next_line" ] && echo "[restic-backup] next elapse: $next_line"
            else
              echo "[restic-backup] ERROR: invalid OnCalendar expression: ${cfg.schedule}" >&2
              exit 1
            fi
          else
            echo "[restic-backup] timer disabled"
          fi
          # Secrets file checks intentionally removed to avoid impacting boot/rebuild.
        '';
      };
    })
  ];
}
