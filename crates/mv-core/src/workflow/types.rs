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

/// How a prompt step selects its model: a single id, or an ordered preference
/// list resolved through the fallback chain mechanism (the first reachable
/// model serves the step).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ModelSpec {
    /// A bare model id, e.g. `model: qwen3:8b`.
    Single(String),
    /// `model: { prefer: [a, b] }` — try `a`, then `b`, …
    Prefer { prefer: Vec<String> },
}

impl ModelSpec {
    /// The candidate model ids in preference order.
    pub fn candidates(&self) -> Vec<String> {
        match self {
            ModelSpec::Single(id) => vec![id.clone()],
            ModelSpec::Prefer { prefer } => prefer.clone(),
        }
    }
}

/// Default settings inherited by all steps unless overridden.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefaults {
    #[serde(default)]
    pub model: Option<ModelSpec>,
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
///
/// `Branch` is a control-flow step: it owns nested arm step-lists rather than
/// producing a single output, which is why [`Step::output`] returns `Option`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Step {
    #[serde(rename = "prompt")]
    Prompt(PromptStep),
    #[serde(rename = "tool")]
    Tool(ToolStep),
    #[serde(rename = "transform")]
    Transform(TransformStep),
    #[serde(rename = "branch")]
    Branch(BranchStep),
    #[serde(rename = "parallel")]
    Parallel(ParallelStep),
    #[serde(rename = "loop")]
    Loop(LoopStep),
}

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Step::Prompt(s) => &s.id,
            Step::Tool(s) => &s.id,
            Step::Transform(s) => &s.id,
            Step::Branch(s) => &s.id,
            Step::Parallel(s) => &s.id,
            Step::Loop(s) => &s.id,
        }
    }

    /// The single output name a leaf step produces, or `None` for control-flow
    /// steps (`branch`, `parallel`, `loop`) whose outputs come from their
    /// nested steps.
    pub fn output(&self) -> Option<&str> {
        match self {
            Step::Prompt(s) => Some(&s.output),
            Step::Tool(s) => Some(&s.output),
            Step::Transform(s) => Some(&s.output),
            Step::Branch(_) | Step::Parallel(_) | Step::Loop(_) => None,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Step::Prompt(s) => s.name.as_deref(),
            Step::Tool(s) => s.name.as_deref(),
            Step::Transform(s) => s.name.as_deref(),
            Step::Branch(s) => s.name.as_deref(),
            Step::Parallel(s) => s.name.as_deref(),
            Step::Loop(s) => s.name.as_deref(),
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
    pub model: Option<ModelSpec>,
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

/// A branch step — runs one of two nested step-lists based on a condition.
///
/// The `condition` is a minijinja expression (the same template language as
/// `{{…}}`, e.g. `style == 'detailed'`) evaluated against the execution
/// context. `then` runs when it is truthy; the optional `else` runs otherwise.
/// Arms may nest further branch steps.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub condition: String,
    pub then: Vec<Step>,
    #[serde(default, rename = "else")]
    pub otherwise: Vec<Step>,
}

/// A parallel step — runs its child steps concurrently (fork-join).
///
/// Each child executes against an immutable snapshot of the context taken at
/// the fork, so siblings never see each other's outputs; their outputs must be
/// disjoint and merge back into the context at the join. All children run to
/// completion; if any fail, the step reports every failure.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub steps: Vec<Step>,
}

/// A loop step — runs its body repeatedly (do-while).
///
/// The body executes against the shared context (each iteration sees the
/// previous one's outputs — the refine/accumulator pattern), then
/// `exit_condition` (an optional minijinja expression, like `branch`) is
/// evaluated against the full context; a truthy result stops the loop. The
/// body always runs at least once and at most `max_iterations` times (reaching
/// the cap is normal termination, not an error). The condition may reference
/// the body's outputs since it runs after each iteration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoopStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub max_iterations: u32,
    #[serde(default)]
    pub exit_condition: Option<String>,
    pub steps: Vec<Step>,
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
    /// Base delay between attempts in milliseconds (default 100).
    #[serde(default)]
    pub base_delay_ms: Option<u64>,
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

impl Workflow {
    /// Every `(step_id, model_id)` a prompt step or the workflow defaults
    /// reference, recursing through `branch`/`parallel`. The binary checks
    /// these against the model registry before running (mv-core stays
    /// registry-free); `defaults.model` ids are reported under `"defaults"`.
    pub fn model_references(&self) -> Vec<(String, String)> {
        let mut refs = Vec::new();
        if let Some(spec) = self.defaults.as_ref().and_then(|d| d.model.as_ref()) {
            for id in spec.candidates() {
                refs.push(("defaults".to_string(), id));
            }
        }
        collect_model_references(&self.steps, &mut refs);
        refs
    }
}

/// Find a step by id, descending into branch arms and parallel children —
/// shared by the engine (workflow `outputs` mapping) and the validator.
pub fn find_step<'a>(steps: &'a [Step], id: &str) -> Option<&'a Step> {
    for step in steps {
        if step.id() == id {
            return Some(step);
        }
        match step {
            Step::Branch(bs) => {
                if let Some(found) = find_step(&bs.then, id) {
                    return Some(found);
                }
                if let Some(found) = find_step(&bs.otherwise, id) {
                    return Some(found);
                }
            }
            Step::Parallel(par) => {
                if let Some(found) = find_step(&par.steps, id) {
                    return Some(found);
                }
            }
            Step::Loop(ls) => {
                if let Some(found) = find_step(&ls.steps, id) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn collect_model_references(steps: &[Step], refs: &mut Vec<(String, String)>) {
    for step in steps {
        match step {
            Step::Prompt(ps) => {
                if let Some(spec) = &ps.model {
                    for id in spec.candidates() {
                        refs.push((ps.id.clone(), id));
                    }
                }
            }
            Step::Branch(bs) => {
                collect_model_references(&bs.then, refs);
                collect_model_references(&bs.otherwise, refs);
            }
            Step::Parallel(par) => collect_model_references(&par.steps, refs),
            Step::Loop(ls) => collect_model_references(&ls.steps, refs),
            Step::Tool(_) | Step::Transform(_) => {}
        }
    }
}
