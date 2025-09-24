use clap::{Parser, Subcommand};
use tracing::{info, warn};

mod backup;
mod config;
mod errors;
mod helpers;
mod list;
mod repository;
mod restore;
mod shared;
mod utils;

#[derive(Parser)]
#[command(name = "restic-backup-service")]
#[command(about = "A Rust-based restic backup service for S3 storage", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run backup for configured paths
    Run {
        /// Optional specific paths to backup (otherwise uses config)
        #[arg(value_delimiter = ',')]
        paths: Vec<String>,
    },
    /// List all available backups
    List {
        /// Hostname to list backups for (default: current host)
        #[arg(short = 'H', long)]
        host: Option<String>,
        /// Return data as JSON (for scripting)
        #[arg(short, long)]
        json: bool,
    },
    /// Interactively restore backups
    Restore {
        /// Non-interactive mode with specific options
        #[arg(short = 'H', long)]
        host: Option<String>,
        #[arg(short, long)]
        path: Option<String>,
        #[arg(short, long)]
        timestamp: Option<String>,
    },
    /// Show repository size for a given path
    Size {
        /// Path to check size for
        path: String,
    },
    /// List available hosts in the repository
    Hosts,
    /// Generate sample .env file
    Init,
}

fn init_logging() -> Result<(), crate::errors::BackupServiceError> {
    use tracing_appender::rolling;
    use tracing_subscriber::{fmt::writer::MakeWriterExt, EnvFilter};

    // Create logs directory if it doesn't exist
    std::fs::create_dir_all("./logs")?;

    let file_appender = rolling::daily("./logs", "restic-backup.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_writer(std::io::stdout.and(non_blocking))
        .with_env_filter(env_filter)
        .init();

    // Keep the guard alive
    std::mem::forget(_guard);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging first
    init_logging()?;

    let cli = Cli::parse();

    // Load configuration for all commands except init
    let config = match &cli.command {
        Commands::Init => None,
        _ => Some(config::Config::load()?),
    };

    match cli.command {
        Commands::Run { paths } => {
            backup::run_backup(config.unwrap(), paths).await?;
        }
        Commands::List { host, json } => {
            list::list_backups(config.unwrap(), host, json).await?;
        }
        Commands::Restore {
            host,
            path,
            timestamp,
        } => {
            restore::restore_interactive(config.unwrap(), host, path, timestamp).await?;
        }
        Commands::Size { path } => {
            utils::show_size(config.unwrap(), path).await?;
        }
        Commands::Hosts => {
            list::list_hosts(config.unwrap()).await?;
        }
        Commands::Init => {
            init_env_file()?;
        }
    }

    Ok(())
}

fn init_env_file() -> Result<(), crate::errors::BackupServiceError> {
    use std::fs;
    use std::path::Path;

    let env_file = ".env";
    if Path::new(env_file).exists() {
        warn!(file = %env_file, ".env file already exists, not overwriting");
        return Ok(());
    }

    let content = r#"# Restic Backup Service Configuration
# Fill in your actual values below

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
# Example: /home/user/documents,/home/user/projects
BACKUP_PATHS=/home/user/important_data

# Optional: Custom hostname (defaults to system hostname)
# BACKUP_HOSTNAME=my-custom-hostname
"#;

    fs::write(env_file, content)?;
    info!(file = %env_file, "Created sample .env file, please edit with your actual credentials");

    Ok(())
}
