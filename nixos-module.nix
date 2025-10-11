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
    version = "0.9.6";
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
    AWS_DEFAULT_REGION=${cfg.aws.defaultRegion}
    BACKUP_PATHS=${lib.concatStringsSep "," cfg.backupPaths}
    ${lib.optionalString (cfg.hostname != null) "BACKUP_HOSTNAME=${cfg.hostname}"}
  '';
  # Optional inline secrets content supplied via Nix (overrides secretsFile when set)
  envInlineFile =
    if cfg.envContent == null
    then null
    else pkgs.writeText "restic-backup-inline.env" cfg.envContent;

  # Create a script that sources secrets and runs the backup
  backupScript = pkgs.writeShellScript "restic-backup-runner" ''
    set -euo pipefail

    # Source the main environment file
    set -a
    source ${envFile}

    # Source inline secrets content; no file fallback
    if [ -s "${envInlineFile}" ]; then
      source "${envInlineFile}"
    else
      echo "Error: Inline secrets content is empty" >&2
      exit 1
    fi

    # Set individual secrets if provided (overrides secretsFile)
    ${lib.optionalString (cfg.restic.passwordFile != null) ''
      if [ -r "${cfg.restic.passwordFile}" ]; then
        RESTIC_PASSWORD=$(cat "${cfg.restic.passwordFile}")
      fi
    ''}

    ${lib.optionalString (cfg.aws.accessKeyIdFile != null) ''
      if [ -r "${cfg.aws.accessKeyIdFile}" ]; then
        AWS_ACCESS_KEY_ID=$(cat "${cfg.aws.accessKeyIdFile}")
      fi
    ''}

    ${lib.optionalString (cfg.aws.secretAccessKeyFile != null) ''
      if [ -r "${cfg.aws.secretAccessKeyFile}" ]; then
        AWS_SECRET_ACCESS_KEY=$(cat "${cfg.aws.secretAccessKeyFile}")
      fi
    ''}

    # Set direct configuration values (overrides both files)
    ${lib.optionalString (cfg.restic.repoBase != null) ''
      RESTIC_REPO_BASE="${cfg.restic.repoBase}"
    ''}

    ${lib.optionalString (cfg.aws.s3Endpoint != null) ''
      AWS_S3_ENDPOINT="${cfg.aws.s3Endpoint}"
    ''}

    set +a

    # Validate required environment variables
    for var in RESTIC_PASSWORD RESTIC_REPO_BASE AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_S3_ENDPOINT; do
      if [ -z "''${!var:-}" ]; then
        echo "Error: Required environment variable $var is not set" >&2
        exit 1
      fi
    done

    # Run the backup
    exec "${cfg.package}/bin/restic-backup-service" run ${lib.concatStringsSep " " cfg.extraArgs}
  '';
in {
  options.services.restic_backup = {
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

    envContent = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Inline content of the secrets env file (overrides secretsFile when set).";
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
  };

  # Thin wrapper interface to match user's desired config shape
  options.services."restic-backup-service" = {
    enable = lib.mkEnableOption "Simple restic-backup-service wrapper (maps to services.restic_backup)";

    backupPaths = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "List of native filesystem paths to back up.";
      example = ["/home/user/Documents" "/home/user/.config"];
    };

    envContent = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Inline content of the secrets env file (via sops placeholder).";
    };

    backupTime = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "OnCalendar string (e.g., \"06:30\", \"daily\") or null to disable timer.";
      example = "06:30";
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
        services.restic_backup.envContent = simple.envContent;
        services.restic_backup.schedule = simple.backupTime;
      })

    (lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = cfg.backupPaths != [];
          message = "services.restic_backup.backupPaths must not be empty";
        }
        {
          assertion = cfg.envContent != null;
          message = "services.restic_backup.envContent must be set (supply via sops placeholder).";
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
      };

      # Optional systemd timer for scheduled backups
      systemd.timers.restic-backup = lib.mkIf (cfg.schedule != null) {
        description = "Timer for restic backup service";
        wantedBy = ["timers.target"];

        timerConfig = {
          OnCalendar = cfg.schedule;
          Persistent = true;
          RandomizedDelaySec = "5m";
        };
      };

      # Ensure the package is available in the system
      environment.systemPackages = [cfg.package];

      # Make inline secrets available to CLI as /etc/restic-backup.env
      environment.etc."restic-backup.env" = lib.mkIf (cfg.envContent != null) {
        text = cfg.envContent;
        mode = "0400";
        user = "root";
        group = "root";
      };

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
        secretsMode =
          if cfg.envContent != null
          then "inline"
          else "none";
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

          case "${secretsMode}" in
            inline)
              if [ -s "${envInlineFile}" ]; then
                echo "[restic-backup] secrets: inline (readable)"
              else
                echo "[restic-backup] ERROR: inline secrets content is empty" >&2
                exit 1
              fi
              ;;
            none)
              echo "[restic-backup] using individual secret files or env (no combined env file set)"
              ;;
          esac
        '';
      };
    })
  ];
}
