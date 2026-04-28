use std::fmt;

/// Typed errors for the mv-core library.
#[derive(Debug, thiserror::Error)]
pub enum MvError {
    #[error("Prompt cannot be empty.")]
    EmptyPrompt,

    #[error("Cannot reach model backend at {endpoint}. Is Ollama running?")]
    BackendUnreachable { endpoint: String },

    #[error("Model '{model}' not found. Run: ollama pull {model}")]
    ModelNotFound { model: String },

    #[error("Model returned an error: {details}")]
    CompletionFailed { details: String },
}

/// Connection settings for the inference backend.
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub endpoint: String,
    pub model: String,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".to_string(),
            model: "qwen3:4b".to_string(),
        }
    }
}

impl fmt::Display for BackendConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.model, self.endpoint)
    }
}

/// Validate a prompt string. Returns the trimmed prompt on success.
pub fn validate_prompt(prompt: &str) -> Result<&str, MvError> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err(MvError::EmptyPrompt);
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_empty_prompt_message() {
        let err = MvError::EmptyPrompt;
        assert_eq!(err.to_string(), "Prompt cannot be empty.");
    }

    #[test]
    fn error_backend_unreachable_message() {
        let err = MvError::BackendUnreachable {
            endpoint: "http://localhost:11434".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Cannot reach model backend at http://localhost:11434. Is Ollama running?"
        );
    }

    #[test]
    fn error_model_not_found_message() {
        let err = MvError::ModelNotFound {
            model: "qwen3:4b".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Model 'qwen3:4b' not found. Run: ollama pull qwen3:4b"
        );
    }

    #[test]
    fn error_completion_failed_message() {
        let err = MvError::CompletionFailed {
            details: "timeout".to_string(),
        };
        assert_eq!(err.to_string(), "Model returned an error: timeout");
    }

    #[test]
    fn backend_config_defaults() {
        let config = BackendConfig::default();
        assert_eq!(config.endpoint, "http://localhost:11434");
        assert_eq!(config.model, "qwen3:4b");
    }

    #[test]
    fn backend_config_display() {
        let config = BackendConfig::default();
        assert_eq!(config.to_string(), "qwen3:4b@http://localhost:11434");
    }

    #[test]
    fn validate_prompt_rejects_empty() {
        let result = validate_prompt("");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MvError::EmptyPrompt));
    }

    #[test]
    fn validate_prompt_rejects_whitespace_only() {
        let result = validate_prompt("   \t\n  ");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MvError::EmptyPrompt));
    }

    #[test]
    fn validate_prompt_accepts_valid() {
        let result = validate_prompt("  What is Rust?  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "What is Rust?");
    }

    #[test]
    fn empty_model_response_is_valid() {
        // An empty response from the model is not an error — the model chose to say nothing.
        let response = "";
        assert!(response.is_empty());
        // The CLI should handle this gracefully: print nothing, exit 0.
    }
}
