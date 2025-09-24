use thiserror::Error;

/// Comprehensive error enum for the backup service using thiserror
#[derive(Error, Debug)]
pub enum BackupServiceError {
    // Core operational errors
    #[error("Authentication failed: Invalid credentials or access denied")]
    AuthenticationFailed,

    #[error("Network error: Cannot connect to repository")]
    NetworkError,

    #[error("Repository not found: {0}")]
    RepositoryNotFound(String),

    #[error("Command execution failed: {0}")]
    CommandFailed(String),


    // Context-specific operation errors
    #[error("Credential validation failed: {0}")]
    CredentialValidationFailed(#[source] Box<BackupServiceError>),

    // Automatic conversions from standard library errors
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    JsonError(#[from] serde_json::Error),

    #[error(transparent)]
    ChronoError(#[from] chrono::ParseError),

    #[error(transparent)]
    DialogueError(#[from] dialoguer::Error),

    #[error(transparent)]
    TemplateError(#[from] indicatif::style::TemplateError),

    #[error(transparent)]
    EnvVarError(#[from] std::env::VarError),

    #[error("Command not found or execution error: {0}")]
    CommandNotFound(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),
}

impl BackupServiceError {

    pub fn with_validation_context(self) -> BackupServiceError {
        BackupServiceError::CredentialValidationFailed(Box::new(self))
    }

    /// Parse stderr output to determine specific error type
    pub fn from_stderr(stderr: &str, context: &str) -> Self {
        let stderr_lower = stderr.to_lowercase();

        if stderr_lower.contains("access denied") || stderr_lower.contains("invalid credentials") ||
           stderr_lower.contains("authorization") || stderr_lower.contains("forbidden") ||
           stderr_lower.contains("access key") || stderr_lower.contains("secret key") {
            BackupServiceError::AuthenticationFailed
        } else if stderr_lower.contains("network") || stderr_lower.contains("connection") ||
                  stderr_lower.contains("timeout") || stderr_lower.contains("unreachable") ||
                  stderr_lower.contains("dns") {
            BackupServiceError::NetworkError
        } else if stderr_lower.contains("repository") && stderr_lower.contains("not found") {
            BackupServiceError::RepositoryNotFound(context.to_string())
        } else {
            BackupServiceError::CommandFailed(stderr.to_string())
        }
    }
}




#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_from_stderr() {
        assert!(matches!(
            BackupServiceError::from_stderr("access denied", "test"),
            BackupServiceError::AuthenticationFailed
        ));

        assert!(matches!(
            BackupServiceError::from_stderr("network timeout", "test"),
            BackupServiceError::NetworkError
        ));

        assert!(matches!(
            BackupServiceError::from_stderr("repository not found", "test"),
            BackupServiceError::RepositoryNotFound(_)
        ));

        assert!(matches!(
            BackupServiceError::from_stderr("some other error", "test"),
            BackupServiceError::CommandFailed(_)
        ));
    }

    #[test]
    fn test_error_context_wrapping() {
        let base_error = BackupServiceError::AuthenticationFailed;
        let wrapped = base_error.with_validation_context();

        assert!(matches!(wrapped, BackupServiceError::CredentialValidationFailed(_)));
    }
}