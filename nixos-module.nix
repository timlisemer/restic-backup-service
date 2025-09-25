{ config, lib, pkgs, ... }:

let
  cfg = config.services.restic_backup;

  # Default package - will be overridden when used through flake
  defaultPackage = pkgs.rustPlatform.buildRustPackage rec {
    pname = "restic-backup-service";
    version = "0.1.0";
    src = ./.;
    cargoLock = {
      lockFile = ./Cargo.lock;
    };

    nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
    buildInputs = with pkgs; [ openssl ];
    propagatedBuildInputs = with pkgs; [ restic awscli2 ];

    postInstall = ''
      wrapProgram $out/bin/restic-backup-service \
        --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.restic pkgs.awscli2 ]}
    '';
  };

  # Create environment file with non-secret configuration
  envFile = pkgs.writeText "restic-backup-env" ''
    AWS_DEFAULT_REGION=${cfg.aws.defaultRegion}
    BACKUP_PATHS=${lib.concatStringsSep "," cfg.backupPaths}
    ${lib.optionalString (cfg.hostname != null) "BACKUP_HOSTNAME=${cfg.hostname}"}
  '';

  # Create a script that sources secrets and runs the backup
  backupScript = pkgs.writeShellScript "restic-backup-runner" ''
    set -euo pipefail

    # Source the main environment file
    set -a
    source ${envFile}

    # Source secrets file if provided
    ${lib.optionalString (cfg.secretsFile != null) ''
      if [ -r "${cfg.secretsFile}" ]; then
        source "${cfg.secretsFile}"
      else
        echo "Error: Cannot read secrets file ${cfg.secretsFile}" >&2
        exit 1
      fi
    ''}

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
      default = [ ];
      example = [ "/home/user/Documents" "/home/user/.config" ];
      description = "List of paths to backup";
    };

    hostname = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Custom hostname for backups (defaults to system hostname)";
    };

    secretsFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      example = lib.literalExpression "config.sops.secrets.restic-env.path";
      description = "Path to file containing all restic and AWS secrets (environment file format)";
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
      default = [ ];
      example = [ "--verbose" ];
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

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.backupPaths != [ ];
        message = "services.restic_backup.backupPaths must not be empty";
      }
      {
        assertion = cfg.secretsFile != null || (
          cfg.restic.passwordFile != null &&
          cfg.aws.accessKeyIdFile != null &&
          cfg.aws.secretAccessKeyFile != null &&
          (cfg.restic.repoBase != null || cfg.aws.s3Endpoint != null)
        );
        message = "services.restic_backup requires either secretsFile or individual secret files for all required credentials";
      }
    ];

    systemd.services.restic-backup = {
      description = "Restic backup service";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "oneshot";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${backupScript}";

        # Security settings
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = false; # Need access to backup paths
        ReadWritePaths = [ "/tmp" "/var/log" ];
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
      wantedBy = [ "timers.target" ];

      timerConfig = {
        OnCalendar = cfg.schedule;
        Persistent = true;
        RandomizedDelaySec = "5m";
      };
    };

    # Ensure the package is available in the system
    environment.systemPackages = [ cfg.package ];
  };
}