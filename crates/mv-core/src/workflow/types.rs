use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Top-level workflow definition parsed from YAML.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub defaults: Option<WorkflowDefaults>,
    #[serde(default)]
    pub inputs: Vec<WorkflowInput>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub outputs: Vec<WorkflowOutput>,
}

/// Default settings inherited by all steps unless overridden.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefaults {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
}

/// A named parameter provided by the user at runtime.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowInput {
    pub name: String,
    #[serde(rename = "type")]
    pub input_type: InputType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub values: Vec<String>,
}

/// Type of a workflow input parameter.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    String,
    Enum,
}

/// A single unit of work within a workflow, discriminated by `type`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Step {
    #[serde(rename = "prompt")]
    Prompt(PromptStep),
    #[serde(rename = "tool")]
    Tool(ToolStep),
    #[serde(rename = "transform")]
    Transform(TransformStep),
}

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Step::Prompt(s) => &s.id,
            Step::Tool(s) => &s.id,
            Step::Transform(s) => &s.id,
        }
    }

    pub fn output(&self) -> &str {
        match self {
            Step::Prompt(s) => &s.output,
            Step::Tool(s) => &s.output,
            Step::Transform(s) => &s.output,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Step::Prompt(s) => s.name.as_deref(),
            Step::Tool(s) => s.name.as_deref(),
            Step::Transform(s) => s.name.as_deref(),
        }
    }
}

/// A prompt step — sends a templated prompt to an LLM.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PromptStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub output: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub template_file: Option<String>,
}

/// A tool step — invokes a built-in or MCP tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub output: String,
    pub tool: String,
    #[serde(default)]
    pub inputs: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub on_error: ErrorAction,
    #[serde(default)]
    pub retry: Option<RetryConfig>,
}

/// A transform step — applies a data transformation.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransformStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub output: String,
    pub operation: String,
    pub input: String,
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
}

/// Error handling strategy for tool steps.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorAction {
    #[default]
    Fail,
    Skip,
    Retry,
}

/// Retry configuration for tool steps.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub backoff: BackoffStrategy,
}

fn default_max_attempts() -> u32 {
    3
}

/// Backoff strategy for retries.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackoffStrategy {
    #[default]
    Exponential,
    Fixed,
}

/// Maps a workflow output name to a step's output.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowOutput {
    pub name: String,
    pub from: String,
}
