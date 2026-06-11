use std::fmt;
use std::path::Path;

use serde::Deserialize;

pub mod mcp;
pub mod providers;
pub mod tools;
pub mod trtllm;
pub mod workflow;

/// Inference backend kind. A closed enum: an unknown provider string in
/// `models.yaml` is rejected at load time instead of resolving to fictitious
/// defaults and failing later at call time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Ollama,
    Openai,
    Trtllm,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Ollama => "ollama",
            Provider::Openai => "openai",
            Provider::Trtllm => "trtllm",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a model runs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locality {
    Local,
    Cloud,
}

impl Locality {
    /// Infer locality from the provider when not explicitly set.
    pub fn from_provider(provider: Provider) -> Self {
        match provider {
            Provider::Ollama | Provider::Trtllm => Locality::Local,
            Provider::Openai => Locality::Cloud,
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
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    pub id: String,
    pub provider: Provider,
    #[serde(default)]
    pub locality: Option<Locality>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub served_name: Option<String>,
    #[serde(default)]
    pub architecture: Option<String>,
    #[serde(default)]
    pub quant: Option<String>,
    #[serde(default)]
    pub expected_vram_gb: Option<u32>,
    /// Optional per-model stop sequences forwarded to the proxy.
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    /// Optional cap on agentic tool-use turns per request.
    #[serde(default)]
    pub max_turns: Option<u32>,
}

impl ModelEntry {
    /// Resolved locality — explicit value or inferred from provider.
    pub fn locality(&self) -> Locality {
        self.locality
            .clone()
            .unwrap_or_else(|| Locality::from_provider(self.provider))
    }

    /// Resolved endpoint — explicit value or provider default.
    pub fn endpoint(&self) -> String {
        self.endpoint.clone().unwrap_or_else(|| {
            match self.provider {
                Provider::Ollama => "http://localhost:11434",
                Provider::Openai => "https://api.openai.com/v1",
                Provider::Trtllm => "http://localhost:8003/v1",
            }
            .to_string()
        })
    }

    /// Model name sent to the API — `served_name` if set, otherwise `id`.
    pub fn model_name(&self) -> &str {
        self.served_name.as_deref().unwrap_or(&self.id)
    }

    /// Effective stop sequences for this model: the explicit list if set,
    /// otherwise the TRT-LLM provider default for TRT-LLM models,
    /// otherwise `None`.
    pub fn effective_stop_sequences(&self) -> Option<Vec<String>> {
        if let Some(seq) = &self.stop_sequences {
            return Some(seq.clone());
        }
        if self.provider == Provider::Trtllm {
            return Some(trtllm::stop::default_stop_sequences());
        }
        None
    }

    /// Resolved agentic turn limit — explicit value or the default of 10.
    pub fn effective_max_turns(&self) -> usize {
        self.max_turns.map(|n| n as usize).unwrap_or(10)
    }

    /// Resolved API-key environment variable for cloud providers.
    pub fn api_key_env(&self) -> &str {
        self.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY")
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
        let content = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MvError::ConfigNotFound {
                    path: path.display().to_string(),
                }
            } else {
                MvError::ConfigParseError {
                    path: path.display().to_string(),
                    details: e.to_string(),
                }
            }
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

        // Duplicate ids would silently resolve to the first entry; multiple
        // defaults would silently pick the first. Both are config mistakes —
        // reject them at load, before routing logic depends on them.
        let mut seen = std::collections::HashSet::new();
        for m in &config.models {
            if !seen.insert(m.id.as_str()) {
                return Err(MvError::ConfigParseError {
                    path: source.to_string(),
                    details: format!("duplicate model id '{}'", m.id),
                });
            }
        }
        let defaults: Vec<&str> = config
            .models
            .iter()
            .filter(|m| m.default)
            .map(|m| m.id.as_str())
            .collect();
        if defaults.len() > 1 {
            return Err(MvError::ConfigParseError {
                path: source.to_string(),
                details: format!(
                    "multiple models marked default: {} — mark exactly one",
                    defaults.join(", ")
                ),
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
                provider: Provider::Ollama,
                locality: Some(Locality::Local),
                api_key_env: None,
                endpoint: None,
                default: true,
                served_name: None,
                architecture: None,
                quant: None,
                expected_vram_gb: None,
                stop_sequences: None,
                max_turns: None,
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

    #[error("Cannot reach model backend at {endpoint}. {hint}")]
    BackendUnreachable { endpoint: String, hint: String },

    #[error("Model '{model}' not found. Run: ollama pull {model}")]
    ModelNotFound { model: String },

    #[error("Model '{model}' is not loaded on the TRT-LLM proxy. {hint}")]
    ModelNotLoaded { model: String, hint: String },

    #[error("streaming is only supported for TRT-LLM models in this release")]
    StreamingNotSupported,

    #[error(
        "Model hit the agentic turn limit ({turns} turns) without a final answer. \
         Try a simpler prompt, or --no-tools if tools are not needed."
    )]
    MaxTurnsExceeded { turns: u64 },

    #[error("tool '{tool}' failed: {details}")]
    ToolCallFailed { tool: String, details: String },

    #[error("Model returned an error: {details}")]
    CompletionFailed { details: String },

    #[error("Config file not found: {path}")]
    ConfigNotFound { path: String },

    #[error("Failed to parse config '{path}': {details}")]
    ConfigParseError { path: String, details: String },

    #[error("Model '{model}' not found in registry. Available: {available}")]
    ModelNotInRegistry { model: String, available: String },

    #[error("API key required for {provider}. Set {env_var} environment variable.")]
    ApiKeyMissing { provider: String, env_var: String },

    #[error("MCP config file not found: {path}")]
    McpConfigNotFound { path: String },

    #[error("failed to parse MCP config '{path}': {details}")]
    McpConfigParseError { path: String, details: String },

    #[error("MCP server '{server}': {details}")]
    McpServerError { server: String, details: String },

    #[error("duplicate MCP server name: '{name}'")]
    McpDuplicateServer { name: String },

    #[error("workflow file not found: {path}")]
    WorkflowFileNotFound { path: String },

    #[error("failed to parse workflow '{path}': {details}")]
    WorkflowParseError { path: String, details: String },

    #[error("validation failed: {details}")]
    WorkflowValidationError { details: String },

    #[error("step '{step}': {details}")]
    WorkflowStepFailed { step: String, details: String },

    #[error("required input '{name}' not provided")]
    WorkflowInputMissing { name: String },

    #[error("input '{name}' value '{value}' not in allowed values: [{allowed}]")]
    WorkflowInputInvalid {
        name: String,
        value: String,
        allowed: String,
    },

    #[error("step '{step}': template error: {details}")]
    WorkflowTemplateError { step: String, details: String },
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
            hint: "Is Ollama running?".to_string(),
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
    fn error_max_turns_exceeded_message() {
        let err = MvError::MaxTurnsExceeded { turns: 10 };
        assert_eq!(
            err.to_string(),
            "Model hit the agentic turn limit (10 turns) without a final answer. \
             Try a simpler prompt, or --no-tools if tools are not needed."
        );
    }

    #[test]
    fn error_tool_call_failed_message() {
        let err = MvError::ToolCallFailed {
            tool: "file_list".to_string(),
            details: "no such tool".to_string(),
        };
        assert_eq!(err.to_string(), "tool 'file_list' failed: no such tool");
    }

    #[test]
    fn locality_from_provider_ollama() {
        assert_eq!(Locality::from_provider(Provider::Ollama), Locality::Local);
    }

    #[test]
    fn locality_from_provider_openai() {
        assert_eq!(Locality::from_provider(Provider::Openai), Locality::Cloud);
    }

    #[test]
    fn locality_display() {
        assert_eq!(Locality::Local.to_string(), "local");
        assert_eq!(Locality::Cloud.to_string(), "cloud");
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
        assert_eq!(entry.unwrap().provider, Provider::Ollama);
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
        assert_eq!(registry.default_model().provider, Provider::Ollama);
    }

    #[test]
    fn model_entry_locality_inferred() {
        let entry = ModelEntry {
            id: "test".to_string(),
            provider: Provider::Ollama,
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
            served_name: None,
            architecture: None,
            quant: None,
            expected_vram_gb: None,
            stop_sequences: None,
            max_turns: None,
        };
        assert_eq!(entry.locality(), Locality::Local);
    }

    #[test]
    fn model_entry_endpoint_defaults() {
        let ollama = ModelEntry {
            id: "test".to_string(),
            provider: Provider::Ollama,
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
            served_name: None,
            architecture: None,
            quant: None,
            expected_vram_gb: None,
            stop_sequences: None,
            max_turns: None,
        };
        assert_eq!(ollama.endpoint(), "http://localhost:11434");

        let openai = ModelEntry {
            id: "test".to_string(),
            provider: Provider::Openai,
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
            served_name: None,
            architecture: None,
            quant: None,
            expected_vram_gb: None,
            stop_sequences: None,
            max_turns: None,
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

    // --- TRT-LLM provider tests (T007) ---

    #[test]
    fn locality_from_provider_trtllm() {
        assert_eq!(Locality::from_provider(Provider::Trtllm), Locality::Local);
    }

    // --- 008/T012-T015: Provider enum, registry validation, resolvers ---

    #[test]
    fn unknown_provider_rejected_at_load() {
        let yaml = "models:\n  - id: oops\n    provider: trtlm\n";
        let err = ModelRegistry::from_yaml(yaml, "test").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("trtlm"), "got: {msg}");
        assert!(
            msg.contains("ollama") && msg.contains("openai") && msg.contains("trtllm"),
            "error must list valid providers, got: {msg}"
        );
    }

    #[test]
    fn unknown_model_field_rejected_at_load() {
        let yaml = "models:\n  - id: m\n    provider: ollama\n    serverd_name: typo\n";
        let err = ModelRegistry::from_yaml(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("serverd_name"), "got: {err}");
    }

    #[test]
    fn duplicate_model_ids_rejected() {
        let yaml = "models:\n  - id: m\n    provider: ollama\n  - id: m\n    provider: ollama\n";
        let err = ModelRegistry::from_yaml(yaml, "test").unwrap_err();
        assert!(
            err.to_string().contains("duplicate model id 'm'"),
            "got: {err}"
        );
    }

    #[test]
    fn multiple_defaults_rejected() {
        let yaml = "models:\n  - id: a\n    provider: ollama\n    default: true\n  - id: b\n    provider: ollama\n    default: true\n";
        let err = ModelRegistry::from_yaml(yaml, "test").unwrap_err();
        assert!(
            err.to_string().contains("multiple models marked default"),
            "got: {err}"
        );
    }

    #[test]
    fn missing_config_file_is_not_found_error() {
        let err = ModelRegistry::load(Path::new("/nonexistent/models.yaml")).unwrap_err();
        assert!(matches!(err, MvError::ConfigNotFound { .. }));
        assert_eq!(
            err.to_string(),
            "Config file not found: /nonexistent/models.yaml"
        );
    }

    #[test]
    fn effective_max_turns_default_and_explicit() {
        let yaml = "models:\n  - id: a\n    provider: ollama\n  - id: b\n    provider: ollama\n    max_turns: 3\n";
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        assert_eq!(registry.get("a").unwrap().effective_max_turns(), 10);
        assert_eq!(registry.get("b").unwrap().effective_max_turns(), 3);
    }

    #[test]
    fn api_key_env_default_and_explicit() {
        let yaml = "models:\n  - id: a\n    provider: openai\n  - id: b\n    provider: openai\n    api_key_env: MY_KEY\n";
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        assert_eq!(registry.get("a").unwrap().api_key_env(), "OPENAI_API_KEY");
        assert_eq!(registry.get("b").unwrap().api_key_env(), "MY_KEY");
    }

    #[test]
    fn model_entry_endpoint_default_trtllm() {
        let entry = ModelEntry {
            id: "llama-fp8".to_string(),
            provider: Provider::Trtllm,
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
            served_name: None,
            architecture: None,
            quant: None,
            expected_vram_gb: None,
            stop_sequences: None,
            max_turns: None,
        };
        assert_eq!(entry.endpoint(), "http://localhost:8003/v1");
    }

    #[test]
    fn model_entry_model_name_uses_served_name() {
        let entry = ModelEntry {
            id: "llama-fp8".to_string(),
            provider: Provider::Trtllm,
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
            served_name: Some("meta-llama/Meta-Llama-3.1-8B-Instruct".to_string()),
            architecture: None,
            quant: None,
            expected_vram_gb: None,
            stop_sequences: None,
            max_turns: None,
        };
        assert_eq!(entry.model_name(), "meta-llama/Meta-Llama-3.1-8B-Instruct");
    }

    #[test]
    fn model_entry_model_name_falls_back_to_id() {
        let entry = ModelEntry {
            id: "llama-fp8".to_string(),
            provider: Provider::Trtllm,
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
            served_name: None,
            architecture: None,
            quant: None,
            expected_vram_gb: None,
            stop_sequences: None,
            max_turns: None,
        };
        assert_eq!(entry.model_name(), "llama-fp8");
    }

    #[test]
    fn model_entry_deserialize_trtllm_with_metadata() {
        let yaml = r#"
models:
  - id: llama-fp8
    provider: trtllm
    served_name: meta-llama/Meta-Llama-3.1-8B-Instruct
    architecture: llama
    quant: fp8
    expected_vram_gb: 9
"#;
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        let entry = registry.get("llama-fp8").unwrap();
        assert_eq!(entry.provider, Provider::Trtllm);
        assert_eq!(
            entry.served_name.as_deref(),
            Some("meta-llama/Meta-Llama-3.1-8B-Instruct")
        );
        assert_eq!(entry.architecture.as_deref(), Some("llama"));
        assert_eq!(entry.quant.as_deref(), Some("fp8"));
        assert_eq!(entry.expected_vram_gb, Some(9));
        assert_eq!(entry.locality(), Locality::Local);
        assert_eq!(entry.endpoint(), "http://localhost:8003/v1");
        assert_eq!(entry.model_name(), "meta-llama/Meta-Llama-3.1-8B-Instruct");
    }

    #[test]
    fn model_entry_deserialize_trtllm_minimal() {
        let yaml = r#"
models:
  - id: llama-3_1-8b-fp8
    provider: trtllm
"#;
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        let entry = registry.get("llama-3_1-8b-fp8").unwrap();
        assert_eq!(entry.provider, Provider::Trtllm);
        assert!(entry.served_name.is_none());
        assert!(entry.architecture.is_none());
        assert!(entry.quant.is_none());
        assert!(entry.expected_vram_gb.is_none());
        assert_eq!(entry.model_name(), "llama-3_1-8b-fp8");
    }

    #[test]
    fn error_model_not_loaded_message() {
        let err = MvError::ModelNotLoaded {
            model: "llama-3_1-8b-fp8".to_string(),
            hint: "Run: just up".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Model 'llama-3_1-8b-fp8' is not loaded on the TRT-LLM proxy. Run: just up"
        );
    }

    #[test]
    fn effective_stop_sequences_explicit_list() {
        let yaml = r#"
models:
  - id: llama-3_1-8b-fp8
    provider: trtllm
    stop_sequences:
      - "<|eot_id|>"
"#;
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        let entry = registry.get("llama-3_1-8b-fp8").unwrap();
        assert_eq!(
            entry.effective_stop_sequences(),
            Some(vec!["<|eot_id|>".to_string()])
        );
    }

    #[test]
    fn effective_stop_sequences_default_for_trtllm() {
        let yaml = r#"
models:
  - id: llama-3_1-8b-fp8
    provider: trtllm
"#;
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        let entry = registry.get("llama-3_1-8b-fp8").unwrap();
        assert_eq!(
            entry.effective_stop_sequences(),
            Some(crate::trtllm::stop::default_stop_sequences())
        );
    }

    #[test]
    fn effective_stop_sequences_none_for_non_trtllm() {
        let yaml = r#"
models:
  - id: qwen3:4b
    provider: ollama
"#;
        let registry = ModelRegistry::from_yaml(yaml, "test").unwrap();
        let entry = registry.get("qwen3:4b").unwrap();
        assert!(entry.effective_stop_sequences().is_none());
    }
}
