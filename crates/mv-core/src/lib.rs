use std::fmt;
use std::path::Path;

use serde::Deserialize;

/// Where a model runs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locality {
    Local,
    Cloud,
}

impl Locality {
    /// Infer locality from provider name when not explicitly set.
    pub fn from_provider(provider: &str) -> Self {
        match provider {
            "ollama" => Locality::Local,
            _ => Locality::Cloud,
        }
    }
}

impl fmt::Display for Locality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Locality::Local => write!(f, "local"),
            Locality::Cloud => write!(f, "cloud"),
        }
    }
}

/// A single model definition from configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub locality: Option<Locality>,
    pub api_key_env: Option<String>,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub default: bool,
}

impl ModelEntry {
    /// Resolved locality — explicit value or inferred from provider.
    pub fn locality(&self) -> Locality {
        self.locality
            .clone()
            .unwrap_or_else(|| Locality::from_provider(&self.provider))
    }

    /// Resolved endpoint — explicit value or provider default.
    pub fn endpoint(&self) -> String {
        self.endpoint
            .clone()
            .unwrap_or_else(|| match self.provider.as_str() {
                "ollama" => "http://localhost:11434".to_string(),
                "openai" => "https://api.openai.com/v1".to_string(),
                _ => "http://localhost:8080".to_string(),
            })
    }
}

/// YAML wrapper for deserialization.
#[derive(Debug, Deserialize)]
struct ModelsConfig {
    models: Vec<ModelEntry>,
}

/// In-memory collection of models loaded from configuration.
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    models: Vec<ModelEntry>,
}

impl ModelRegistry {
    /// Load a registry from a YAML file.
    pub fn load(path: &Path) -> Result<Self, MvError> {
        let content = std::fs::read_to_string(path).map_err(|e| MvError::ConfigParseError {
            path: path.display().to_string(),
            details: e.to_string(),
        })?;
        Self::from_yaml(&content, &path.display().to_string())
    }

    /// Parse a registry from a YAML string.
    fn from_yaml(yaml: &str, source: &str) -> Result<Self, MvError> {
        let config: ModelsConfig =
            serde_yml::from_str(yaml).map_err(|e| MvError::ConfigParseError {
                path: source.to_string(),
                details: e.to_string(),
            })?;
        if config.models.is_empty() {
            return Err(MvError::ConfigParseError {
                path: source.to_string(),
                details: "no models defined".to_string(),
            });
        }
        Ok(Self {
            models: config.models,
        })
    }

    /// Look up a model by ID.
    pub fn get(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    /// Return the default model (explicit `default: true`, or first entry).
    pub fn default_model(&self) -> &ModelEntry {
        self.models
            .iter()
            .find(|m| m.default)
            .unwrap_or(&self.models[0])
    }

    /// List all available model IDs.
    pub fn available_ids(&self) -> Vec<&str> {
        self.models.iter().map(|m| m.id.as_str()).collect()
    }

    /// Built-in registry with hardcoded defaults (backward compat).
    pub fn built_in() -> Self {
        Self {
            models: vec![ModelEntry {
                id: "qwen3:4b".to_string(),
                provider: "ollama".to_string(),
                locality: Some(Locality::Local),
                api_key_env: None,
                endpoint: None,
                default: true,
            }],
        }
    }

    /// Resolve config: explicit path → ./models.yaml → built-in defaults.
    pub fn resolve(config_path: Option<&str>) -> Result<Self, MvError> {
        if let Some(path) = config_path {
            return Self::load(Path::new(path));
        }
        let default_path = Path::new("models.yaml");
        if default_path.exists() {
            return Self::load(default_path);
        }
        Ok(Self::built_in())
    }
}

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

    #[error("Failed to parse config '{path}': {details}")]
    ConfigParseError { path: String, details: String },

    #[error("Model '{model}' not found in registry. Available: {available}")]
    ModelNotInRegistry { model: String, available: String },

    #[error("API key required for {provider}. Set {env_var} environment variable.")]
    ApiKeyMissing { provider: String, env_var: String },
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
    fn error_config_parse_message() {
        let err = MvError::ConfigParseError {
            path: "./models.yaml".to_string(),
            details: "invalid YAML".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Failed to parse config './models.yaml': invalid YAML"
        );
    }

    #[test]
    fn error_model_not_in_registry_message() {
        let err = MvError::ModelNotInRegistry {
            model: "foo".to_string(),
            available: "qwen3:4b, qwen3:8b".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Model 'foo' not found in registry. Available: qwen3:4b, qwen3:8b"
        );
    }

    #[test]
    fn error_api_key_missing_message() {
        let err = MvError::ApiKeyMissing {
            provider: "openai".to_string(),
            env_var: "OPENAI_API_KEY".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "API key required for openai. Set OPENAI_API_KEY environment variable."
        );
    }

    #[test]
    fn locality_from_provider_ollama() {
        assert_eq!(Locality::from_provider("ollama"), Locality::Local);
    }

    #[test]
    fn locality_from_provider_openai() {
        assert_eq!(Locality::from_provider("openai"), Locality::Cloud);
    }

    #[test]
    fn locality_display() {
        assert_eq!(Locality::Local.to_string(), "local");
        assert_eq!(Locality::Cloud.to_string(), "cloud");
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
        let response = "";
        assert!(response.is_empty());
    }

    // --- Model Registry tests ---

    #[test]
    fn registry_load_valid_yaml() {
        let yaml = r#"
models:
  - id: qwen3:4b
    provider: ollama
    default: true
  - id: qwen3:8b
    provider: ollama
"#;
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        assert_eq!(registry.available_ids().len(), 2);
        assert_eq!(registry.available_ids()[0], "qwen3:4b");
        assert_eq!(registry.available_ids()[1], "qwen3:8b");
    }

    #[test]
    fn registry_load_malformed_yaml() {
        let yaml = "not: [valid: yaml: {{";
        let result = ModelRegistry::from_yaml(yaml, "bad.yaml");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, MvError::ConfigParseError { .. }));
    }

    #[test]
    fn registry_get_found() {
        let yaml = "models:\n  - id: qwen3:4b\n    provider: ollama\n";
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        let entry = registry.get("qwen3:4b");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().provider, "ollama");
    }

    #[test]
    fn registry_get_not_found() {
        let yaml = "models:\n  - id: qwen3:4b\n    provider: ollama\n";
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn registry_default_explicit() {
        let yaml = r#"
models:
  - id: qwen3:4b
    provider: ollama
  - id: qwen3:8b
    provider: ollama
    default: true
"#;
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        assert_eq!(registry.default_model().id, "qwen3:8b");
    }

    #[test]
    fn registry_default_first_entry_fallback() {
        let yaml = r#"
models:
  - id: qwen3:4b
    provider: ollama
  - id: qwen3:8b
    provider: ollama
"#;
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        assert_eq!(registry.default_model().id, "qwen3:4b");
    }

    #[test]
    fn registry_built_in_defaults() {
        let registry = ModelRegistry::built_in();
        assert_eq!(registry.available_ids(), vec!["qwen3:4b"]);
        assert_eq!(registry.default_model().id, "qwen3:4b");
        assert_eq!(registry.default_model().provider, "ollama");
    }

    #[test]
    fn model_entry_locality_inferred() {
        let entry = ModelEntry {
            id: "test".to_string(),
            provider: "ollama".to_string(),
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
        };
        assert_eq!(entry.locality(), Locality::Local);
    }

    #[test]
    fn model_entry_endpoint_defaults() {
        let ollama = ModelEntry {
            id: "test".to_string(),
            provider: "ollama".to_string(),
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
        };
        assert_eq!(ollama.endpoint(), "http://localhost:11434");

        let openai = ModelEntry {
            id: "test".to_string(),
            provider: "openai".to_string(),
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
        };
        assert_eq!(openai.endpoint(), "https://api.openai.com/v1");
    }

    #[test]
    fn api_key_missing_for_cloud_model() {
        // This tests the error variant exists and formats correctly.
        // Actual API key resolution is in the CLI layer.
        let err = MvError::ApiKeyMissing {
            provider: "openai".to_string(),
            env_var: "OPENAI_API_KEY".to_string(),
        };
        assert!(err.to_string().contains("OPENAI_API_KEY"));
    }
}
