use std::collections::HashMap;
use std::path::Path;

use tracing::{debug, info};

use super::retry::execute_tool_with_error_handling;
use super::template;
use super::types::{Step, Workflow};
use crate::MvError;

// Re-exported so existing callers (and tests) keep one import path.
pub use super::transform::execute_transform;

/// Trait for executing prompt steps — enables mocking in tests.
///
/// Methods return `Send` futures and implementors are `Send + Sync` so step
/// execution can be moved onto worker tasks (the Phase 5 `parallel` step
/// type spawns arms with `tokio::spawn`).
pub trait PromptExecutor: Send + Sync {
    fn execute_prompt(
        &self,
        prompt_text: &str,
        model: &str,
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
    inputs: HashMap<String, String>,
    outputs: HashMap<String, String>,
}

impl ExecutionContext {
    pub fn new(inputs: HashMap<String, String>) -> Self {
        Self {
            inputs,
            outputs: HashMap::new(),
        }
    }

    /// Record a step output. Outputs shadow inputs of the same name in
    /// subsequent template contexts.
    pub fn insert_output(&mut self, name: impl Into<String>, value: String) {
        self.outputs.insert(name.into(), value);
    }

    /// Look up a recorded step output.
    pub fn output(&self, name: &str) -> Option<&str> {
        self.outputs.get(name).map(String::as_str)
    }

    /// Build a template variable map: outputs shadow inputs.
    pub fn to_template_context(&self) -> HashMap<String, String> {
        let mut vars = self.inputs.clone();
        vars.extend(self.outputs.clone());
        vars
    }

    /// Immutable copy of the current state. Parallel arms will each receive
    /// a snapshot taken at the fork, never a shared mutable context.
    pub fn snapshot(&self) -> Self {
        self.clone()
    }
}

/// Result of executing a workflow.
#[derive(Debug)]
pub struct WorkflowResult {
    pub outputs: HashMap<String, String>,
}

/// Defaults applied to prompt steps that don't override them.
struct StepDefaults {
    model: String,
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
    // Validate inputs
    let resolved_inputs = validate_inputs(workflow, inputs)?;
    let mut ctx = ExecutionContext::new(resolved_inputs);

    let defaults = StepDefaults {
        model: workflow
            .defaults
            .as_ref()
            .and_then(|d| d.model.clone())
            .unwrap_or_else(|| default_model.to_string()),
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
async fn execute_steps<P: PromptExecutor, T: ToolExecutor>(
    steps: &[Step],
    ctx: &mut ExecutionContext,
    defaults: &StepDefaults,
    prompt_executor: &P,
    tool_executor: &T,
    workflow_dir: &Path,
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
        )
        .await?;

        info!(
            step_id = %step.id(),
            duration_ms = start.elapsed().as_millis() as u64,
            "step completed"
        );
    }
    Ok(())
}

/// Execute a single step, recording any leaf output into the context. Control
/// steps (`branch`) mutate the context by recursing into the chosen arm rather
/// than producing a single output.
async fn execute_step<P: PromptExecutor, T: ToolExecutor>(
    step: &Step,
    ctx: &mut ExecutionContext,
    defaults: &StepDefaults,
    prompt_executor: &P,
    tool_executor: &T,
    workflow_dir: &Path,
) -> Result<(), MvError> {
    match step {
        Step::Prompt(ps) => {
            let model = ps.model.as_deref().unwrap_or(&defaults.model);
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
                .execute_prompt(&rendered, model, temp, max_tok)
                .await
                .map_err(|e| MvError::WorkflowStepError {
                    step: ps.id.clone(),
                    source: Box::new(e),
                })?;
            ctx.insert_output(&ps.output, output);
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
            ctx.insert_output(&ts.output, output);
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
            ))
            .await?;
        }
    }
    Ok(())
}

/// Render every string leaf of a JSON value through the template engine —
/// nested objects/arrays included, so `inputs: {headers: {auth: "{{token}}"}}`
/// interpolates instead of passing the literal braces to the tool.
fn render_json_value(
    val: &serde_json::Value,
    vars: &HashMap<String, String>,
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
    }
}

/// Find a step by id, descending into branch arms — workflow `outputs` may map
/// `from:` to a step nested inside a branch.
fn find_step_by_id<'a>(steps: &'a [Step], id: &str) -> Option<&'a Step> {
    for step in steps {
        if step.id() == id {
            return Some(step);
        }
        if let Step::Branch(bs) = step {
            if let Some(found) = find_step_by_id(&bs.then, id) {
                return Some(found);
            }
            if let Some(found) = find_step_by_id(&bs.otherwise, id) {
                return Some(found);
            }
        }
    }
    None
}

fn build_workflow_outputs(workflow: &Workflow, ctx: &ExecutionContext) -> HashMap<String, String> {
    if workflow.outputs.is_empty() {
        // When no outputs specified, return the last step's output (if it is a
        // leaf step that produced one — a trailing branch has no single output).
        let mut map = HashMap::new();
        if let Some(output_name) = workflow.steps.last().and_then(|s| s.output())
            && let Some(value) = ctx.output(output_name)
        {
            map.insert(output_name.to_string(), value.to_string());
        }
        map
    } else {
        workflow
            .outputs
            .iter()
            .filter_map(|wo| {
                // wo.from is a step ID — find that step's output name.
                let step = find_step_by_id(&workflow.steps, &wo.from)?;
                let output_name = step.output()?;
                ctx.output(output_name)
                    .map(|v| (wo.name.clone(), v.to_string()))
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
            model: &str,
            _temperature: Option<f64>,
            _max_tokens: Option<u64>,
        ) -> Result<String, MvError> {
            self.calls
                .lock()
                .unwrap()
                .push((prompt_text.to_string(), model.to_string()));
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
        ctx.insert_output("topic".to_string(), "output_value".to_string());
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
        ctx.insert_output("output_key".to_string(), "output_val".to_string());
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

    // --- T032: Transform step tests ---

    #[test]
    fn extract_json_valid() {
        let input = r#"{"title": "Rust Guide", "sections": 5}"#;
        let result = execute_transform("test", "extract_json", input, None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
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
        let result = execute_transform("test", "extract_json", input, None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
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
        let result = execute_transform("test", "extract_json", input, Some(&schema)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
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
}
