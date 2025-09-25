use crate::config::Config;
use crate::errors::BackupServiceError;
use crate::shared::display::DisplayFormatter;
use crate::shared::operations::RepositoryOperations;
use crate::utils::validate_credentials;
use serde_json::json;
use tracing::{info, warn};

// CLI command to retrieve and display available backup hosts from S3
pub async fn list_hosts(config: Config) -> Result<(), BackupServiceError> {
    info!("Getting available hosts...");
    config.set_aws_env()?;

    // Validate AWS credentials before accessing S3
    validate_credentials(&config).await?;

    use crate::shared::operations::RepositoryOperations;
    let operations = RepositoryOperations::new(config)?;
    let hosts = operations.get_available_hosts().await?;

    if hosts.is_empty() {
        warn!("No hosts found in backup repository (repository is empty)");
    } else {
        info!("\nAvailable hosts:");
        for host in hosts {
            info!("  - {}", host);
        }
    }

    Ok(())
}

// Main CLI command to list backups with human-readable or JSON output
pub async fn list_backups(
    config: Config,
    host: Option<String>,
    json_output: bool,
) -> Result<(), BackupServiceError> {
    // Use provided hostname or fall back to config hostname
    let hostname = host.unwrap_or_else(|| config.hostname.clone());
    config.set_aws_env()?;

    if !json_output {
        info!(hostname = %hostname, "Listing backups from S3 bucket");
    }

    validate_credentials(&config).await?;

    // Collect and process repository data for display
    let (repos, all_snapshots) = {
        let operations = RepositoryOperations::new(config)?;
        let repo_data = operations.collect_backup_data(&hostname).await?;
        (
            operations.convert_to_backup_repos(repo_data.clone())?,
            operations.extract_all_snapshots(&repo_data),
        )
    };

    if json_output {
        // Format output as structured JSON for scripting
        let output = json!({
            "host": hostname,
            "repositories": repos.iter().map(|r| json!({
                "path": r.native_path.to_string_lossy(),
                "category": r.category().unwrap_or("unknown"),
                "snapshot_count": r.snapshot_count
            })).collect::<Vec<_>>(),
            "snapshots": all_snapshots.iter().map(|s| json!({
                "time": s.time.to_rfc3339(),
                "path": s.path.to_string_lossy(),
                "id": s.id
            })).collect::<Vec<_>>()
        });
        info!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        DisplayFormatter::display_backup_summary(&repos, &all_snapshots)?;
    }

    Ok(())
}
