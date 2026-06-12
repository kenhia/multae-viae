use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;
use tracing::{debug, info};

use super::retry::execute_tool_with_error_handling;
use super::template;
use super::types::{Step, Workflow};
use crate::MvError;

// Re-exported so existing callers (and tests) keep one import path.
pub use super::transform::execute_transform;

/// Trait for executing prompt steps — enables mocking in tests.
///
/// Methods return `Send` futures and implementors are `Send + Sync`; the
/// `parallel` step type runs children concurrently with
/// `futures::future::join_all` on the current task (no `tokio::spawn`, so no
/// `'static` bound on the executor borrows).
pub trait PromptExecutor: Send + Sync {
    /// `models` is the candidate list in preference order (a single id for a
    /// bare `model:`, or the `prefer:` list). The executor tries them through
    /// the fallback chain mechanism; the first reachable model serves the step.
    fn execute_prompt(
        &self,
        prompt_text: &str,
        models: &[String],
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> impl std::future::Future<Output = Result<String, MvError>> + Send;
}

/// Trait for executing tool steps — enables mocking in tests.
pub trait ToolExecutor: Send + Sync {
    fn execute_tool(
        &self,
        tool_name: &str,
        inputs: &HashMap<String, serde_json::Value>,
    ) -> impl std::future::Future<Output = Result<String, MvError>> + Send;
}

/// Context for workflow execution, tracking step outputs and inputs.
///
/// Fields are private so the representation can evolve (snapshot isolation
/// for parallel arms, scoped frames for loop iterations) without breaking
/// callers — go through the accessors.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    inputs: HashMap<String, Value>,
    outputs: HashMap<String, Value>,
}

impl ExecutionContext {
    /// Build a context from CLI/workflow inputs. Inputs arrive as strings (the
    /// `--input KEY=VALUE` boundary) and are stored as `Value::String`;
    /// structure enters the context later, via `extract_json` and nested
    /// workflow outputs.
    pub fn new(inputs: HashMap<String, String>) -> Self {
        Self {
            inputs: inputs
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
            outputs: HashMap::new(),
        }
    }

    /// Record a step output as a typed value. Outputs shadow inputs of the same
    /// name in subsequent template contexts.
    pub fn insert_output(&mut self, name: impl Into<String>, value: Value) {
        self.outputs.insert(name.into(), value);
    }

    /// Look up a recorded step output.
    pub fn output(&self, name: &str) -> Option<&Value> {
        self.outputs.get(name)
    }

    /// Build a template variable map: outputs shadow inputs.
    pub fn to_template_context(&self) -> HashMap<String, Value> {
        let mut vars = self.inputs.clone();
        vars.extend(self.outputs.clone());
        vars
    }

    /// Immutable copy of the current state. Parallel children each receive a
    /// snapshot taken at the fork, never a shared mutable context.
    pub fn snapshot(&self) -> Self {
        self.clone()
    }

    /// Outputs present here but not in `base` — the new outputs a parallel
    /// child produced on top of its fork snapshot, for merging at the join.
    pub fn outputs_added_since(&self, base: &ExecutionContext) -> Vec<(String, Value)> {
        self.outputs
            .iter()
            .filter(|(name, _)| !base.outputs.contains_key(*name))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}

/// Result of executing a workflow. Output values are typed; a front end prints
/// strings raw and everything else as JSON (see `commands/workflow.rs`).
#[derive(Debug)]
pub struct WorkflowResult {
    pub outputs: HashMap<String, Value>,
}

/// Defaults applied to prompt steps that don't override them.
struct StepDefaults {
    /// Default candidate model list (preference order) for steps with no
    /// explicit `model:`.
    models: Vec<String>,
    temperature: Option<f64>,
    max_tokens: Option<u64>,
}

/// Validate required inputs and apply defaults. Returns the resolved inputs map.
pub fn validate_inputs(
    workflow: &Workflow,
    provided: HashMap<String, String>,
) -> Result<HashMap<String, String>, MvError> {
    let mut resolved = provided;

    for input in &workflow.inputs {
        if resolved.contains_key(&input.name) {
            // Validate enum values
            if input.input_type == super::types::InputType::Enum && !input.values.is_empty() {
                let value = &resolved[&input.name];
                if !input.values.contains(value) {
                    return Err(MvError::WorkflowInputInvalid {
                        name: input.name.clone(),
                        value: value.clone(),
                        allowed: input.values.join(", "),
                    });
                }
            }
        } else if let Some(ref default) = input.default {
            info!(input = %input.name, default = %default, "using default for input");
            resolved.insert(input.name.clone(), default.clone());
        } else if input.required {
            return Err(MvError::WorkflowInputMissing {
                name: input.name.clone(),
            });
        }
    }

    Ok(resolved)
}

/// Execute a workflow sequentially.
///
/// `default_model` is used by prompt steps when neither the step nor the
/// workflow `defaults` name a model — pass the registry's default; the
/// engine itself has no provider knowledge and no built-in model name.
#[tracing::instrument(
    name = "workflow_execute",
    skip(workflow, inputs, prompt_executor, tool_executor, workflow_dir, default_model),
    fields(
        workflow.name = %workflow.name,
        workflow.version = %workflow.version,
        workflow.step_count = workflow.steps.len(),
    )
)]
pub async fn execute_workflow<P: PromptExecutor, T: ToolExecutor>(
    workflow: &Workflow,
    inputs: HashMap<String, String>,
    prompt_executor: &P,
    tool_executor: &T,
    workflow_dir: &Path,
    default_model: &str,
) -> Result<WorkflowResult, MvError> {
    execute_workflow_inner(
        workflow,
        inputs,
        prompt_executor,
        tool_executor,
        workflow_dir,
        default_model,
        &[],
    )
    .await
}

/// Maximum nesting depth for `workflow` steps (a child running a child …).
const MAX_WORKFLOW_DEPTH: usize = 8;

/// The real workflow body, threading `chain` — the canonical paths of nested
/// workflows currently executing — so a `workflow` step can reject a cycle or
/// over-deep nesting before re-entering. The top-level call passes `&[]`.
async fn execute_workflow_inner<P: PromptExecutor, T: ToolExecutor>(
    workflow: &Workflow,
    inputs: HashMap<String, String>,
    prompt_executor: &P,
    tool_executor: &T,
    workflow_dir: &Path,
    default_model: &str,
    chain: &[std::path::PathBuf],
) -> Result<WorkflowResult, MvError> {
    // Validate inputs
    let resolved_inputs = validate_inputs(workflow, inputs)?;
    let mut ctx = ExecutionContext::new(resolved_inputs);

    let defaults = StepDefaults {
        models: workflow
            .defaults
            .as_ref()
            .and_then(|d| d.model.as_ref())
            .map(|spec| spec.candidates())
            .unwrap_or_else(|| vec![default_model.to_string()]),
        temperature: workflow.defaults.as_ref().and_then(|d| d.temperature),
        max_tokens: workflow.defaults.as_ref().and_then(|d| d.max_tokens),
    };

    execute_steps(
        &workflow.steps,
        &mut ctx,
        &defaults,
        prompt_executor,
        tool_executor,
        workflow_dir,
        default_model,
        chain,
    )
    .await?;

    // Build final outputs
    let final_outputs = build_workflow_outputs(workflow, &ctx);
    Ok(WorkflowResult {
        outputs: final_outputs,
    })
}

/// Run a step list against a mutable context. Recurses into nested step lists
/// (the `branch` arms) via `Box::pin` for the async recursion.
#[allow(clippy::too_many_arguments)]
async fn execute_steps<P: PromptExecutor, T: ToolExecutor>(
    steps: &[Step],
    ctx: &mut ExecutionContext,
    defaults: &StepDefaults,
    prompt_executor: &P,
    tool_executor: &T,
    workflow_dir: &Path,
    default_model: &str,
    chain: &[std::path::PathBuf],
) -> Result<(), MvError> {
    for step in steps {
        let step_span = tracing::info_span!(
            "workflow_step",
            step.id = %step.id(),
            step.type = %step_type_name(step),
            step.output = step.output().unwrap_or(""),
        );
        let _enter = step_span.enter();
        let start = std::time::Instant::now();

        debug!(step_id = %step.id(), step_type = %step_type_name(step), "executing step");

        execute_step(
            step,
            ctx,
            defaults,
            prompt_executor,
            tool_executor,
            workflow_dir,
            default_model,
            chain,
        )
        .await?;

        // Leaf steps just recorded their output into ctx — surface its name
        // and size; control steps (branch/parallel) log empty/0.
        let output_name = step.output().unwrap_or("");
        let output_len = step
            .output()
            .and_then(|name| ctx.output(name))
            // A string value's own length; otherwise its JSON length.
            .map_or(0, |v| {
                v.as_str().map_or_else(|| v.to_string().len(), str::len)
            });
        info!(
            step_id = %step.id(),
            output_name,
            output_len,
            duration_ms = start.elapsed().as_millis() as u64,
            "step completed"
        );
    }
    Ok(())
}

/// Execute a single step, recording any leaf output into the context. Control
/// steps (`branch`) mutate the context by recursing into the chosen arm rather
/// than producing a single output.
#[allow(clippy::too_many_arguments)]
async fn execute_step<P: PromptExecutor, T: ToolExecutor>(
    step: &Step,
    ctx: &mut ExecutionContext,
    defaults: &StepDefaults,
    prompt_executor: &P,
    tool_executor: &T,
    workflow_dir: &Path,
    default_model: &str,
    chain: &[std::path::PathBuf],
) -> Result<(), MvError> {
    match step {
        Step::Prompt(ps) => {
            // Resolve the candidate model list: the step's `model:` spec, or
            // the workflow default candidate list.
            let models = ps
                .model
                .as_ref()
                .map(|spec| spec.candidates())
                .unwrap_or_else(|| defaults.models.clone());
            let temp = ps.temperature.or(defaults.temperature);
            let max_tok = ps.max_tokens.or(defaults.max_tokens);

            // Resolve template
            let template_str = if let Some(ref tmpl) = ps.template {
                tmpl.clone()
            } else if let Some(ref file) = ps.template_file {
                template::load_template_file(file, workflow_dir).map_err(|_| {
                    MvError::WorkflowTemplateError {
                        step: ps.id.clone(),
                        details: format!("template file not found: {file}"),
                    }
                })?
            } else {
                return Err(MvError::WorkflowStepFailed {
                    step: ps.id.clone(),
                    details: "no template or template_file specified".to_string(),
                });
            };

            let vars = ctx.to_template_context();
            let rendered = template::render_template(&template_str, &vars).map_err(|e| {
                MvError::WorkflowTemplateError {
                    step: ps.id.clone(),
                    details: e.to_string(),
                }
            })?;

            let output = prompt_executor
                .execute_prompt(&rendered, &models, temp, max_tok)
                .await
                .map_err(|e| MvError::WorkflowStepError {
                    step: ps.id.clone(),
                    source: Box::new(e),
                })?;
            // Model output is text; structure enters via `extract_json`.
            ctx.insert_output(&ps.output, Value::String(output));
        }
        Step::Tool(ts) => {
            // Render tool inputs from context — every string leaf, including
            // ones nested inside objects and arrays.
            let vars = ctx.to_template_context();
            let mut rendered_inputs = HashMap::new();
            for (key, val) in &ts.inputs {
                rendered_inputs.insert(key.clone(), render_json_value(val, &vars, &ts.id)?);
            }

            let output =
                execute_tool_with_error_handling(ts, &rendered_inputs, tool_executor).await?;
            // Tool output is text (truncated upstream); structure via transform.
            ctx.insert_output(&ts.output, Value::String(output));
        }
        Step::Transform(ts) => {
            let vars = ctx.to_template_context();
            let input_value = template::render_template(&ts.input, &vars).map_err(|e| {
                MvError::WorkflowTemplateError {
                    step: ts.id.clone(),
                    details: e.to_string(),
                }
            })?;

            let output =
                execute_transform(&ts.id, &ts.operation, &input_value, ts.schema.as_ref())?;
            ctx.insert_output(&ts.output, output);
        }
        Step::Branch(bs) => {
            // Evaluate the condition against the current context, then run the
            // chosen arm — its steps write their outputs into `ctx` directly.
            let vars = ctx.to_template_context();
            let take_then = template::evaluate_condition(&bs.condition, &vars).map_err(|e| {
                MvError::WorkflowTemplateError {
                    step: bs.id.clone(),
                    details: e,
                }
            })?;
            debug!(step_id = %bs.id, take_then, "branch condition evaluated");
            let arm = if take_then { &bs.then } else { &bs.otherwise };
            Box::pin(execute_steps(
                arm,
                ctx,
                defaults,
                prompt_executor,
                tool_executor,
                workflow_dir,
                default_model,
                chain,
            ))
            .await?;
        }
        Step::Parallel(par) => {
            // Fork: each child runs against an immutable snapshot taken now, so
            // siblings never observe each other's outputs.
            let snapshot = ctx.snapshot();
            let child_futures = par.steps.iter().map(|child| {
                let mut child_ctx = snapshot.clone();
                async move {
                    Box::pin(execute_steps(
                        std::slice::from_ref(child),
                        &mut child_ctx,
                        defaults,
                        prompt_executor,
                        tool_executor,
                        workflow_dir,
                        default_model,
                        chain,
                    ))
                    .await
                    .map(|()| child_ctx)
                }
            });

            // Join: run every child to completion (no early abort — partial
            // side effects must not be scheduling-dependent).
            let results = futures::future::join_all(child_futures).await;

            let mut failures: Vec<(String, String)> = Vec::new();
            let mut completed: Vec<ExecutionContext> = Vec::new();
            for (child, result) in par.steps.iter().zip(results) {
                match result {
                    Ok(child_ctx) => completed.push(child_ctx),
                    Err(e) => failures.push((child.id().to_string(), e.to_string())),
                }
            }
            if !failures.is_empty() {
                return Err(MvError::WorkflowParallelFailed {
                    step: par.id.clone(),
                    failures,
                });
            }

            // Merge each child's new outputs back into the parent context, in
            // declaration order (outputs are validated disjoint, so order only
            // affects determinism, not correctness).
            for child_ctx in completed {
                for (name, value) in child_ctx.outputs_added_since(&snapshot) {
                    ctx.insert_output(name, value);
                }
            }
        }
        Step::Loop(ls) => {
            // Do-while over the shared context: the body runs, then the exit
            // condition (if any) is evaluated against the full context, so it
            // can read what the iteration just produced. Reaching
            // `max_iterations` is normal termination, not an error.
            let mut iterations: u32 = 0;
            loop {
                iterations += 1;
                debug!(step_id = %ls.id, iteration = iterations, "loop iteration");
                Box::pin(execute_steps(
                    &ls.steps,
                    ctx,
                    defaults,
                    prompt_executor,
                    tool_executor,
                    workflow_dir,
                    default_model,
                    chain,
                ))
                .await?;

                if let Some(cond) = &ls.exit_condition {
                    let vars = ctx.to_template_context();
                    let done = template::evaluate_condition(cond, &vars).map_err(|e| {
                        MvError::WorkflowTemplateError {
                            step: ls.id.clone(),
                            details: e,
                        }
                    })?;
                    if done {
                        break;
                    }
                }
                if iterations >= ls.max_iterations {
                    break;
                }
            }
            debug!(step_id = %ls.id, iterations, "loop completed");
        }
        Step::SubWorkflow(sw) => {
            // Resolve the child file relative to the parent's directory.
            let child_path = workflow_dir.join(&sw.file);
            let canonical =
                child_path
                    .canonicalize()
                    .map_err(|_| MvError::WorkflowFileNotFound {
                        path: child_path.display().to_string(),
                    })?;

            // Cycle: re-entering a workflow already running in this chain.
            if chain.contains(&canonical) {
                let mut rendered: Vec<String> =
                    chain.iter().map(|p| p.display().to_string()).collect();
                rendered.push(canonical.display().to_string());
                return Err(MvError::WorkflowCycle {
                    chain: rendered.join(" -> "),
                });
            }
            // Depth: bound runaway nesting (chain length is the current depth).
            if chain.len() + 1 > MAX_WORKFLOW_DEPTH {
                return Err(MvError::WorkflowDepthExceeded {
                    max: MAX_WORKFLOW_DEPTH,
                });
            }

            // Load + validate the child (runtime re-validation, mirroring the
            // template_file precedent).
            let child_wf = super::parser::load_from_file(&canonical).map_err(|e| {
                MvError::WorkflowStepError {
                    step: sw.id.clone(),
                    source: Box::new(e),
                }
            })?;
            // Owned child dir so it outlives moving `canonical` into the chain.
            let child_dir = canonical
                .parent()
                .map_or_else(|| std::path::PathBuf::from("."), Path::to_path_buf);
            let verrs = super::validate::validate(&child_wf, Some(&child_dir));
            if let Some(first) = verrs.first() {
                return Err(MvError::WorkflowStepFailed {
                    step: sw.id.clone(),
                    details: format!("nested workflow '{}' is invalid: {first}", sw.file),
                });
            }

            // Render the templated inputs against the parent context; the child
            // sees ONLY these (no parent-context leakage).
            let vars = ctx.to_template_context();
            let mut child_inputs: HashMap<String, String> = HashMap::new();
            for (key, tmpl) in &sw.inputs {
                let rendered = template::render_template(tmpl, &vars).map_err(|e| {
                    MvError::WorkflowTemplateError {
                        step: sw.id.clone(),
                        details: e.to_string(),
                    }
                })?;
                child_inputs.insert(key.clone(), rendered);
            }

            let mut new_chain = chain.to_vec();
            new_chain.push(canonical);
            let child_result = Box::pin(execute_workflow_inner(
                &child_wf,
                child_inputs,
                prompt_executor,
                tool_executor,
                &child_dir,
                default_model,
                &new_chain,
            ))
            .await?;

            // The child's declared outputs become one object stored at `output`,
            // so a later step can reach into them (`{{sub.answer}}`).
            let obj: serde_json::Map<String, Value> = child_result.outputs.into_iter().collect();
            ctx.insert_output(&sw.output, Value::Object(obj));
        }
    }
    Ok(())
}

/// Render every string leaf of a JSON value through the template engine —
/// nested objects/arrays included, so `inputs: {headers: {auth: "{{token}}"}}`
/// interpolates instead of passing the literal braces to the tool.
fn render_json_value(
    val: &serde_json::Value,
    vars: &HashMap<String, Value>,
    step_id: &str,
) -> Result<serde_json::Value, MvError> {
    Ok(match val {
        serde_json::Value::String(s) => {
            let rendered =
                template::render_template(s, vars).map_err(|e| MvError::WorkflowTemplateError {
                    step: step_id.to_string(),
                    details: e.to_string(),
                })?;
            serde_json::Value::String(rendered)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|v| render_json_value(v, vars, step_id))
                .collect::<Result<_, _>>()?,
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), render_json_value(v, vars, step_id)?)))
                .collect::<Result<_, MvError>>()?,
        ),
        other => other.clone(),
    })
}

fn step_type_name(step: &Step) -> &'static str {
    match step {
        Step::Prompt(_) => "prompt",
        Step::Tool(_) => "tool",
        Step::Transform(_) => "transform",
        Step::Branch(_) => "branch",
        Step::Parallel(_) => "parallel",
        Step::Loop(_) => "loop",
        Step::SubWorkflow(_) => "workflow",
    }
}

fn build_workflow_outputs(workflow: &Workflow, ctx: &ExecutionContext) -> HashMap<String, Value> {
    if workflow.outputs.is_empty() {
        // When no outputs specified, return the last step's output (if it is a
        // leaf step that produced one — a trailing branch has no single output).
        let mut map = HashMap::new();
        if let Some(output_name) = workflow.steps.last().and_then(|s| s.output())
            && let Some(value) = ctx.output(output_name)
        {
            map.insert(output_name.to_string(), value.clone());
        }
        map
    } else {
        workflow
            .outputs
            .iter()
            .filter_map(|wo| {
                // wo.from is a step ID — find that step's output name.
                // Validation guarantees the step exists and its output is
                // definitely defined on every execution path.
                let step = super::types::find_step(&workflow.steps, &wo.from)?;
                let output_name = step.output()?;
                ctx.output(output_name)
                    .map(|v| (wo.name.clone(), v.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::parser;
    use std::sync::Mutex;

    /// Mock prompt executor that returns pre-configured responses.
    struct MockPromptExecutor {
        responses: Mutex<Vec<String>>,
        calls: Mutex<Vec<(String, String)>>, // (prompt_text, model)
    }

    impl MockPromptExecutor {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(String::from).collect()),
                calls: Mutex::new(vec![]),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl PromptExecutor for MockPromptExecutor {
        async fn execute_prompt(
            &self,
            prompt_text: &str,
            models: &[String],
            _temperature: Option<f64>,
            _max_tokens: Option<u64>,
        ) -> Result<String, MvError> {
            // Record the candidate list as a comma-joined string — a single
            // model is just its id (back-compat with the pre-prefer tests).
            self.calls
                .lock()
                .unwrap()
                .push((prompt_text.to_string(), models.join(",")));
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Err(MvError::CompletionFailed {
                    details: "no more mock responses".to_string(),
                })
            } else {
                Ok(responses.remove(0))
            }
        }
    }

    /// Mock tool executor.
    struct MockToolExecutor {
        responses: Mutex<Vec<Result<String, String>>>,
    }

    impl MockToolExecutor {
        fn new(responses: Vec<Result<&str, &str>>) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|r| r.map(String::from).map_err(String::from))
                        .collect(),
                ),
            }
        }

        fn always_ok(response: &str) -> Self {
            // Return a large number of OK responses
            Self::new(vec![Ok(response); 100])
        }
    }

    impl ToolExecutor for MockToolExecutor {
        async fn execute_tool(
            &self,
            tool_name: &str,
            _inputs: &HashMap<String, serde_json::Value>,
        ) -> Result<String, MvError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Err(MvError::ToolCallFailed {
                    tool: tool_name.to_string(),
                    details: "no more mock responses".to_string(),
                })
            } else {
                responses.remove(0).map_err(|e| MvError::ToolCallFailed {
                    tool: tool_name.to_string(),
                    details: e,
                })
            }
        }
    }

    #[test]
    fn context_outputs_shadow_inputs() {
        let mut ctx = ExecutionContext::new(
            [("topic".to_string(), "input_value".to_string())]
                .into_iter()
                .collect(),
        );
        ctx.insert_output(
            "topic".to_string(),
            Value::String("output_value".to_string()),
        );
        let vars = ctx.to_template_context();
        assert_eq!(vars["topic"], "output_value");
    }

    #[test]
    fn context_merges_inputs_and_outputs() {
        let mut ctx = ExecutionContext::new(
            [("input_key".to_string(), "input_val".to_string())]
                .into_iter()
                .collect(),
        );
        ctx.insert_output(
            "output_key".to_string(),
            Value::String("output_val".to_string()),
        );
        let vars = ctx.to_template_context();
        assert_eq!(vars["input_key"], "input_val");
        assert_eq!(vars["output_key"], "output_val");
    }

    // --- 008/F4: invalid retry config must error, never panic ---

    #[tokio::test]
    async fn retry_zero_attempts_errors_instead_of_panicking() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: t1
    type: tool
    output: out
    tool: some_tool
    on_error: retry
    retry:
      max_attempts: 0
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![]);
        let tool_exec = MockToolExecutor::always_ok("never reached");

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await;

        let err = result.expect_err("max_attempts: 0 must be an error");
        assert!(
            err.to_string().contains("max_attempts must be at least 1"),
            "got: {err}"
        );
    }

    // --- T013: Sequential engine execution tests ---

    #[tokio::test]
    async fn two_step_prompt_workflow() {
        let yaml = r#"
name: two-step
version: "1.0"
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: research
    type: prompt
    output: research_result
    template: "Research {{topic}}"
  - id: summarize
    type: prompt
    output: summary
    template: "Summarize: {{research_result}}"
outputs:
  - name: result
    from: summarize
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec =
            MockPromptExecutor::new(vec!["Research findings about Rust", "Summary of findings"]);
        let tool_exec = MockToolExecutor::always_ok("");
        let inputs = [("topic".to_string(), "Rust async".to_string())]
            .into_iter()
            .collect();

        let result = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        assert_eq!(prompt_exec.call_count(), 2);
        let calls = prompt_exec.calls();
        assert_eq!(calls[0].0, "Research Rust async");
        assert_eq!(calls[1].0, "Summarize: Research findings about Rust");
        assert_eq!(result.outputs["result"], "Summary of findings");
    }

    #[tokio::test]
    async fn five_step_workflow() {
        let yaml = r#"
name: five-step
version: "1.0"
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: s1
    type: prompt
    output: out1
    template: "Step 1: {{topic}}"
  - id: s2
    type: prompt
    output: out2
    template: "Step 2: {{out1}}"
  - id: s3
    type: prompt
    output: out3
    template: "Step 3: {{out2}}"
  - id: s4
    type: prompt
    output: out4
    template: "Step 4: {{out3}}"
  - id: s5
    type: prompt
    output: out5
    template: "Step 5: {{out4}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec =
            MockPromptExecutor::new(vec!["result1", "result2", "result3", "result4", "result5"]);
        let tool_exec = MockToolExecutor::always_ok("");
        let inputs = [("topic".to_string(), "Rust".to_string())]
            .into_iter()
            .collect();

        let result = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        assert_eq!(prompt_exec.call_count(), 5);
        // Last step output is returned when no outputs specified
        assert_eq!(result.outputs["out5"], "result5");
    }

    #[tokio::test]
    async fn output_context_accumulates() {
        let yaml = r#"
name: accumulate
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: a
    template: "first"
  - id: s2
    type: prompt
    output: b
    template: "{{a}} plus more"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["alpha", "beta"]);
        let tool_exec = MockToolExecutor::always_ok("");

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        let calls = prompt_exec.calls();
        assert_eq!(calls[1].0, "alpha plus more");
        assert_eq!(result.outputs["b"], "beta");
    }

    // --- T014: Workflow input validation tests ---

    #[tokio::test]
    async fn required_input_missing() {
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: s1
    type: prompt
    output: out
    template: "hello"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![]);
        let tool_exec = MockToolExecutor::always_ok("");

        let err = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, MvError::WorkflowInputMissing { .. }));
    }

    #[tokio::test]
    async fn enum_value_invalid() {
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: style
    type: enum
    required: true
    values: [brief, detailed]
steps:
  - id: s1
    type: prompt
    output: out
    template: "hello"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![]);
        let tool_exec = MockToolExecutor::always_ok("");
        let inputs = [("style".to_string(), "verbose".to_string())]
            .into_iter()
            .collect();

        let err = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, MvError::WorkflowInputInvalid { .. }));
    }

    #[tokio::test]
    async fn default_value_applied() {
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: style
    type: string
    default: brief
steps:
  - id: s1
    type: prompt
    output: out
    template: "Style: {{style}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["done"]);
        let tool_exec = MockToolExecutor::always_ok("");

        let _result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        let calls = prompt_exec.calls();
        assert_eq!(calls[0].0, "Style: brief");
    }

    // --- T020: Workflow defaults merging tests ---

    #[tokio::test]
    async fn step_model_overrides_default() {
        let yaml = r#"
name: test
version: "1.0"
defaults:
  model: default-model
steps:
  - id: s1
    type: prompt
    output: out1
    template: "hello"
  - id: s2
    type: prompt
    output: out2
    model: custom-model
    template: "world"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["a", "b"]);
        let tool_exec = MockToolExecutor::always_ok("");

        execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        let calls = prompt_exec.calls();
        assert_eq!(calls[0].1, "default-model");
        assert_eq!(calls[1].1, "custom-model");
    }

    // --- T027: Tool step execution tests ---

    #[tokio::test]
    async fn tool_step_executes_and_stores_output() {
        let yaml = r#"
name: tool-test
version: "1.0"
steps:
  - id: list_files
    type: tool
    output: file_listing
    tool: file_list
    inputs:
      path: "."
  - id: summarize
    type: prompt
    output: summary
    template: "Files: {{file_listing}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["summarized files"]);
        let tool_exec = MockToolExecutor::new(vec![Ok("file1.rs\nfile2.rs")]);

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        // Tool output fed into prompt template
        let calls = prompt_exec.calls();
        assert_eq!(calls[0].0, "Files: file1.rs\nfile2.rs");
        assert_eq!(result.outputs["summary"], "summarized files");
    }

    #[tokio::test]
    async fn tool_step_renders_inputs_from_context() {
        let yaml = r#"
name: tool-input-test
version: "1.0"
inputs:
  - name: dir
    type: string
    required: true
steps:
  - id: list
    type: tool
    output: files
    tool: file_list
    inputs:
      path: "{{dir}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![]);
        let tool_exec = MockToolExecutor::new(vec![Ok("contents")]);
        let inputs = [("dir".to_string(), "/tmp".to_string())]
            .into_iter()
            .collect();

        let result = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        assert_eq!(result.outputs["files"], "contents");
    }

    // --- T028: Tool error handling tests ---

    #[tokio::test]
    async fn tool_on_error_skip_continues() {
        let yaml = r#"
name: skip-test
version: "1.0"
steps:
  - id: failing_tool
    type: tool
    output: tool_output
    tool: bad_tool
    on_error: skip
  - id: next
    type: prompt
    output: result
    template: "After skip: '{{tool_output}}'"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["continued"]);
        let tool_exec = MockToolExecutor::new(vec![Err("tool failure")]);

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        // Skipped tool produces empty output, workflow continues
        let calls = prompt_exec.calls();
        assert_eq!(calls[0].0, "After skip: ''");
        assert_eq!(result.outputs["result"], "continued");
    }

    #[tokio::test]
    async fn tool_on_error_fail_stops_workflow() {
        let yaml = r#"
name: fail-test
version: "1.0"
steps:
  - id: failing_tool
    type: tool
    output: tool_output
    tool: bad_tool
    on_error: fail
  - id: never_reached
    type: prompt
    output: result
    template: "should not get here"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["should not be called"]);
        let tool_exec = MockToolExecutor::new(vec![Err("tool failure")]);

        let err = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap_err();

        // 008/T023: the typed source survives the step wrapper.
        match &err {
            MvError::WorkflowStepError { step, source } => {
                assert_eq!(step, "failing_tool");
                assert!(matches!(**source, MvError::ToolCallFailed { .. }));
            }
            other => panic!("expected WorkflowStepError, got: {other:?}"),
        }
        assert_eq!(prompt_exec.call_count(), 0); // Next step never executed
    }

    #[tokio::test]
    async fn retry_does_not_reattempt_permanent_errors() {
        // 008/T023: only is_retryable() errors are re-attempted. A permanent
        // failure under `on_error: retry` fails on attempt 1.
        struct PermanentErrorToolExecutor {
            calls: Mutex<u32>,
        }
        impl ToolExecutor for PermanentErrorToolExecutor {
            async fn execute_tool(
                &self,
                _tool_name: &str,
                _inputs: &HashMap<String, serde_json::Value>,
            ) -> Result<String, MvError> {
                *self.calls.lock().unwrap() += 1;
                Err(MvError::EmptyPrompt) // stand-in for a non-retryable class
            }
        }

        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: t1
    type: tool
    output: out
    tool: some_tool
    on_error: retry
    retry:
      max_attempts: 3
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![]);
        let tool_exec = PermanentErrorToolExecutor {
            calls: Mutex::new(0),
        };

        let err = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap_err();

        assert!(matches!(
            &err,
            MvError::WorkflowStepError { source, .. } if matches!(**source, MvError::EmptyPrompt)
        ));
        assert_eq!(
            *tool_exec.calls.lock().unwrap(),
            1,
            "permanent error must not be retried"
        );
    }

    #[tokio::test]
    async fn tool_on_error_retry_eventual_success() {
        let yaml = r#"
name: retry-test
version: "1.0"
steps:
  - id: flaky_tool
    type: tool
    output: tool_output
    tool: flaky
    on_error: retry
    retry:
      max_attempts: 3
      backoff: fixed
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![]);
        // Fails twice, succeeds on third attempt
        let tool_exec = MockToolExecutor::new(vec![Err("fail 1"), Err("fail 2"), Ok("success!")]);

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        assert_eq!(result.outputs["tool_output"], "success!");
    }

    #[tokio::test]
    async fn tool_on_error_retry_eventual_failure() {
        let yaml = r#"
name: retry-fail-test
version: "1.0"
steps:
  - id: always_fails
    type: tool
    output: tool_output
    tool: broken
    on_error: retry
    retry:
      max_attempts: 2
      backoff: exponential
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![]);
        let tool_exec = MockToolExecutor::new(vec![Err("fail 1"), Err("fail 2")]);

        let err = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap_err();

        match err {
            MvError::WorkflowStepFailed { details, .. } => {
                assert!(details.contains("2 attempts"), "got: {details}");
            }
            other => panic!("expected WorkflowStepFailed, got: {other}"),
        }
    }

    // --- 009/WS3: branch execution ---

    #[tokio::test]
    async fn branch_runs_then_arm_when_condition_true() {
        let yaml = r#"
name: branch-then
version: "1.0"
inputs:
  - name: style
    type: string
    required: true
steps:
  - id: route
    type: branch
    condition: "style == 'detailed'"
    then:
      - id: deep
        type: prompt
        output: answer
        template: "Deep dive"
    else:
      - id: quick
        type: prompt
        output: answer
        template: "Quick take"
  - id: use
    type: prompt
    output: final
    template: "Answer: {{answer}}"
outputs:
  - name: result
    from: use
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["DEEP", "used"]);
        let tool_exec = MockToolExecutor::always_ok("");
        let inputs = [("style".to_string(), "detailed".to_string())]
            .into_iter()
            .collect();

        let result = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        // Only the then arm + the post-branch step ran.
        let calls = prompt_exec.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "Deep dive");
        assert_eq!(calls[1].0, "Answer: DEEP");
        assert_eq!(result.outputs["result"], "used");
    }

    #[tokio::test]
    async fn branch_runs_else_arm_when_condition_false() {
        let yaml = r#"
name: branch-else
version: "1.0"
inputs:
  - name: style
    type: string
    required: true
steps:
  - id: route
    type: branch
    condition: "style == 'detailed'"
    then:
      - id: deep
        type: prompt
        output: answer
        template: "Deep dive"
    else:
      - id: quick
        type: prompt
        output: answer
        template: "Quick take"
  - id: use
    type: prompt
    output: final
    template: "Answer: {{answer}}"
outputs:
  - name: result
    from: use
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["QUICK", "used"]);
        let tool_exec = MockToolExecutor::always_ok("");
        let inputs = [("style".to_string(), "brief".to_string())]
            .into_iter()
            .collect();

        let result = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        let calls = prompt_exec.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "Quick take");
        assert_eq!(calls[1].0, "Answer: QUICK");
        assert_eq!(result.outputs["result"], "used");
    }

    #[tokio::test]
    async fn branch_false_with_no_else_is_noop() {
        let yaml = r#"
name: branch-noop
version: "1.0"
inputs:
  - name: flag
    type: string
    required: true
steps:
  - id: first
    type: prompt
    output: base
    template: "base"
  - id: maybe
    type: branch
    condition: "flag == 'on'"
    then:
      - id: extra
        type: prompt
        output: extra_out
        template: "extra"
  - id: last
    type: prompt
    output: done
    template: "After {{base}}"
outputs:
  - name: result
    from: last
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        // flag is off → only `first` and `last` run (2 prompt calls).
        let prompt_exec = MockPromptExecutor::new(vec!["B", "D"]);
        let tool_exec = MockToolExecutor::always_ok("");
        let inputs = [("flag".to_string(), "off".to_string())]
            .into_iter()
            .collect();

        let result = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        assert_eq!(prompt_exec.call_count(), 2);
        let calls = prompt_exec.calls();
        assert_eq!(calls[1].0, "After B");
        assert_eq!(result.outputs["result"], "D");
    }

    #[tokio::test]
    async fn nested_branch_executes_inner_arm() {
        let yaml = r#"
name: nested
version: "1.0"
inputs:
  - name: a
    type: string
    required: true
  - name: b
    type: string
    required: true
steps:
  - id: outer
    type: branch
    condition: "a == 'yes'"
    then:
      - id: inner
        type: branch
        condition: "b == 'yes'"
        then:
          - id: both
            type: prompt
            output: answer
            template: "both yes"
        else:
          - id: only_a
            type: prompt
            output: answer
            template: "only a"
    else:
      - id: neither
        type: prompt
        output: answer
        template: "neither"
  - id: use
    type: prompt
    output: final
    template: "Got {{answer}}"
outputs:
  - name: result
    from: use
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["INNER", "used"]);
        let tool_exec = MockToolExecutor::always_ok("");
        let inputs = [
            ("a".to_string(), "yes".to_string()),
            ("b".to_string(), "yes".to_string()),
        ]
        .into_iter()
        .collect();

        let result = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        let calls = prompt_exec.calls();
        assert_eq!(calls[0].0, "both yes");
        assert_eq!(calls[1].0, "Got INNER");
        assert_eq!(result.outputs["result"], "used");
    }

    // --- 009/WS4: parallel execution ---

    #[tokio::test]
    async fn parallel_runs_all_children_and_merges_outputs() {
        let yaml = r#"
name: parallel-merge
version: "1.0"
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: fan
    type: parallel
    steps:
      - id: a
        type: prompt
        output: out_a
        template: "A: {{topic}}"
      - id: b
        type: prompt
        output: out_b
        template: "B: {{topic}}"
  - id: combine
    type: prompt
    output: final
    template: "{{out_a}} | {{out_b}}"
outputs:
  - name: result
    from: combine
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        // Children run concurrently; the mock returns responses in call order,
        // which is non-deterministic, so make both arms return the same marker
        // to keep the assertion order-independent.
        let prompt_exec = MockPromptExecutor::new(vec!["RA", "RB", "combined"]);
        let tool_exec = MockToolExecutor::always_ok("");
        let inputs = [("topic".to_string(), "x".to_string())]
            .into_iter()
            .collect();

        let result = execute_workflow(
            &wf,
            inputs,
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        // Both children ran plus the combine step.
        assert_eq!(prompt_exec.call_count(), 3);
        // The combine step saw both merged outputs (order of RA/RB may vary).
        let combine_prompt = &prompt_exec.calls()[2].0;
        assert!(
            combine_prompt.contains("RA") && combine_prompt.contains("RB"),
            "combine should see both parallel outputs, got: {combine_prompt}"
        );
        assert_eq!(result.outputs["result"], "combined");
    }

    #[tokio::test]
    async fn parallel_aggregates_all_child_failures() {
        let yaml = r#"
name: parallel-fail
version: "1.0"
steps:
  - id: fan
    type: parallel
    steps:
      - id: a
        type: prompt
        output: out_a
        template: "A"
      - id: b
        type: prompt
        output: out_b
        template: "B"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        // No mock responses → every prompt call fails.
        let prompt_exec = MockPromptExecutor::new(vec![]);
        let tool_exec = MockToolExecutor::always_ok("");

        let err = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap_err();

        match err {
            MvError::WorkflowParallelFailed { step, failures } => {
                assert_eq!(step, "fan");
                // Both children ran to completion and both failures reported.
                assert_eq!(failures.len(), 2, "got: {failures:?}");
                let ids: Vec<&str> = failures.iter().map(|(id, _)| id.as_str()).collect();
                assert!(ids.contains(&"a") && ids.contains(&"b"), "got: {ids:?}");
            }
            other => panic!("expected WorkflowParallelFailed, got: {other:?}"),
        }
    }

    /// Prompt executor whose calls rendezvous on a shared barrier: each call
    /// blocks until `n` calls are in flight. If the engine ran children
    /// sequentially, the first call would block forever — so completing within
    /// the timeout proves genuine concurrency.
    struct RendezvousExecutor {
        barrier: std::sync::Arc<tokio::sync::Barrier>,
    }

    impl PromptExecutor for RendezvousExecutor {
        async fn execute_prompt(
            &self,
            prompt_text: &str,
            _models: &[String],
            _temperature: Option<f64>,
            _max_tokens: Option<u64>,
        ) -> Result<String, MvError> {
            self.barrier.wait().await;
            Ok(prompt_text.to_string())
        }
    }

    #[tokio::test]
    async fn parallel_children_run_concurrently() {
        let yaml = r#"
name: rendezvous
version: "1.0"
steps:
  - id: fan
    type: parallel
    steps:
      - id: a
        type: prompt
        output: out_a
        template: "A"
      - id: b
        type: prompt
        output: out_b
        template: "B"
      - id: c
        type: prompt
        output: out_c
        template: "C"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        // Barrier of 3: all three children must be in flight simultaneously.
        let prompt_exec = RendezvousExecutor {
            barrier: std::sync::Arc::new(tokio::sync::Barrier::new(3)),
        };
        let tool_exec = MockToolExecutor::always_ok("");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            execute_workflow(
                &wf,
                HashMap::new(),
                &prompt_exec,
                &tool_exec,
                Path::new("."),
                "qwen3:4b",
            ),
        )
        .await
        .expect(
            "parallel children must run concurrently (sequential would deadlock on the barrier)",
        )
        .unwrap();

        // All three completed and merged.
        assert_eq!(result.outputs.len(), 0); // no `outputs:` mapping; last step is the parallel (no single output)
        // The merge happened: assert via a follow-up is unnecessary — reaching
        // here past the barrier already proves all three ran concurrently.
    }

    // --- T032: Transform step tests ---

    #[test]
    fn extract_json_valid() {
        let input = r#"{"title": "Rust Guide", "sections": 5}"#;
        let parsed = execute_transform("test", "extract_json", input, None).unwrap();
        assert_eq!(parsed["title"], "Rust Guide");
        assert_eq!(parsed["sections"], 5);
    }

    #[test]
    fn extract_json_from_markdown_fence() {
        let input = r#"Here is the result:

```json
{"key": "value"}
```

That's it."#;
        let parsed = execute_transform("test", "extract_json", input, None).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn extract_json_invalid() {
        let err = execute_transform("test", "extract_json", "not json at all", None).unwrap_err();
        assert!(matches!(err, MvError::WorkflowStepFailed { .. }));
    }

    #[test]
    fn extract_json_schema_pass() {
        let schema = serde_json::json!({"title": "", "count": 0});
        let input = r#"{"title": "Hello", "count": 42, "extra": true}"#;
        let parsed = execute_transform("test", "extract_json", input, Some(&schema)).unwrap();
        assert_eq!(parsed["title"], "Hello");
    }

    #[test]
    fn extract_json_schema_fail_missing_key() {
        let schema = serde_json::json!({"title": "", "missing_field": ""});
        let input = r#"{"title": "Hello"}"#;
        let err = execute_transform("test", "extract_json", input, Some(&schema)).unwrap_err();
        match err {
            MvError::WorkflowStepFailed { details, .. } => {
                assert!(details.contains("missing key"), "got: {details}");
            }
            other => panic!("expected WorkflowStepFailed, got: {other}"),
        }
    }

    #[tokio::test]
    async fn transform_step_in_workflow() {
        let yaml = r#"
name: transform-test
version: "1.0"
steps:
  - id: generate
    type: prompt
    output: raw
    template: "Generate JSON"
  - id: extract
    type: transform
    output: data
    operation: extract_json
    input: "{{raw}}"
  - id: use_data
    type: prompt
    output: result
    template: "Data: {{data}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![
            r#"```json
{"name": "test"}
```"#,
            "processed",
        ]);
        let tool_exec = MockToolExecutor::always_ok("");

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        assert_eq!(result.outputs["result"], "processed");
    }

    // --- 012/WS3: loop step ---

    #[tokio::test]
    async fn loop_exits_early_on_condition() {
        let yaml = r#"
name: refine
version: "1.0"
steps:
  - id: spin
    type: loop
    max_iterations: 5
    exit_condition: "draft == 'stop'"
    steps:
      - id: improve
        type: prompt
        output: draft
        template: "improve"
outputs:
  - name: result
    from: improve
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        // Iteration 2 produces "stop" → the loop exits; the 3rd response is
        // never consumed.
        let prompt_exec = MockPromptExecutor::new(vec!["go", "stop", "NEVER"]);
        let tool_exec = MockToolExecutor::always_ok("");

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        assert_eq!(prompt_exec.call_count(), 2, "body should run exactly twice");
        assert_eq!(result.outputs["result"], "stop");
    }

    #[tokio::test]
    async fn loop_without_condition_runs_to_cap() {
        let yaml = r#"
name: spin
version: "1.0"
steps:
  - id: spin
    type: loop
    max_iterations: 3
    steps:
      - id: tick
        type: prompt
        output: t
        template: "tick"
outputs:
  - name: result
    from: tick
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["a", "b", "c"]);
        let tool_exec = MockToolExecutor::always_ok("");

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            Path::new("."),
            "qwen3:4b",
        )
        .await
        .unwrap();

        assert_eq!(
            prompt_exec.call_count(),
            3,
            "no condition → runs to the cap"
        );
        assert_eq!(result.outputs["result"], "c");
    }

    // --- 012/WS4: nested workflow step ---

    #[tokio::test]
    async fn subworkflow_runs_child_and_exposes_its_outputs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("child.yaml"),
            r#"
name: child
version: "1.0"
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: answer
    type: prompt
    output: answer
    template: "About {{topic}}"
outputs:
  - name: answer
    from: answer
"#,
        )
        .unwrap();
        let parent_yaml = r#"
name: parent
version: "1.0"
steps:
  - id: sub
    type: workflow
    file: child.yaml
    inputs:
      topic: "rust"
    output: child
  - id: use
    type: prompt
    output: final
    template: "Child said: {{child.answer}}"
outputs:
  - name: result
    from: use
"#;
        let wf = parser::load_from_str(parent_yaml, "parent.yaml").unwrap();
        // First response = child's `answer` step; second = parent's `use` step.
        let prompt_exec = MockPromptExecutor::new(vec!["CHILD_ANSWER", "PARENT_DONE"]);
        let tool_exec = MockToolExecutor::always_ok("");

        let result = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            dir.path(),
            "qwen3:4b",
        )
        .await
        .unwrap();

        // The parent's prompt rendered the child's output via field access.
        let calls = prompt_exec.calls();
        assert_eq!(calls[1].0, "Child said: CHILD_ANSWER");
        assert_eq!(result.outputs["result"], "PARENT_DONE");
    }

    #[tokio::test]
    async fn subworkflow_cycle_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // a.yaml → b.yaml → a.yaml
        std::fs::write(
            dir.path().join("a.yaml"),
            "name: a\nversion: \"1.0\"\nsteps:\n  - id: s\n    type: workflow\n    \
             file: b.yaml\n    output: o\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.yaml"),
            "name: b\nversion: \"1.0\"\nsteps:\n  - id: s\n    type: workflow\n    \
             file: a.yaml\n    output: o\n",
        )
        .unwrap();
        let wf = parser::load_from_file(&dir.path().join("a.yaml")).unwrap();
        let prompt_exec = MockPromptExecutor::new(vec![]);
        let tool_exec = MockToolExecutor::always_ok("");

        let err = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            dir.path(),
            "qwen3:4b",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, MvError::WorkflowCycle { .. }), "got: {err:?}");
    }

    #[tokio::test]
    async fn subworkflow_depth_is_bounded() {
        // A chain n0 → n1 → … longer than MAX_WORKFLOW_DEPTH, all distinct
        // files (no cycle), must fail with the depth error.
        let dir = tempfile::tempdir().unwrap();
        let n = MAX_WORKFLOW_DEPTH + 2;
        for i in 0..n {
            let body = if i + 1 < n {
                format!(
                    "name: n{i}\nversion: \"1.0\"\nsteps:\n  - id: s\n    type: workflow\n    \
                     file: n{}.yaml\n    output: o\n",
                    i + 1
                )
            } else {
                format!(
                    "name: n{i}\nversion: \"1.0\"\nsteps:\n  - id: s\n    type: prompt\n    \
                     output: o\n    template: \"done\"\n"
                )
            };
            std::fs::write(dir.path().join(format!("n{i}.yaml")), body).unwrap();
        }
        let wf = parser::load_from_file(&dir.path().join("n0.yaml")).unwrap();
        let prompt_exec = MockPromptExecutor::new(vec!["x"; n]);
        let tool_exec = MockToolExecutor::always_ok("");

        let err = execute_workflow(
            &wf,
            HashMap::new(),
            &prompt_exec,
            &tool_exec,
            dir.path(),
            "qwen3:4b",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, MvError::WorkflowDepthExceeded { .. }),
            "got: {err:?}"
        );
    }
}
