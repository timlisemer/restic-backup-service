use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::*;

mod config;
mod repository;
mod utils;
mod helpers;
mod backup;
mod list;
mod restore;

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

#[tokio::main]
async fn main() -> Result<()> {
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
        Commands::Restore { host, path, timestamp } => {
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

fn init_env_file() -> Result<()> {
    use std::fs;
    use std::path::Path;

    let env_file = ".env";
    if Path::new(env_file).exists() {
        println!("{} {} already exists. Not overwriting.",
            "[WARNING]".yellow().bold(),
            env_file
        );
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
    println!("{} Created sample .env file. Please edit it with your actual credentials.",
        "[SUCCESS]".green().bold()
    );

    Ok(())
}