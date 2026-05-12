use std::collections::HashMap;
use std::path::Path;

use tracing::{debug, info, warn};

use super::template;
use super::types::{ErrorAction, Step, Workflow};
use crate::MvError;

/// Trait for executing prompt steps — enables mocking in tests.
#[allow(async_fn_in_trait)]
pub trait PromptExecutor {
    async fn execute_prompt(
        &self,
        prompt_text: &str,
        model: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> Result<String, MvError>;
}

/// Trait for executing tool steps — enables mocking in tests.
#[allow(async_fn_in_trait)]
pub trait ToolExecutor {
    async fn execute_tool(
        &self,
        tool_name: &str,
        inputs: &HashMap<String, serde_json::Value>,
    ) -> Result<String, MvError>;
}

/// Context for workflow execution, tracking step outputs and inputs.
#[derive(Debug)]
pub struct ExecutionContext {
    pub inputs: HashMap<String, String>,
    pub outputs: HashMap<String, String>,
}

impl ExecutionContext {
    pub fn new(inputs: HashMap<String, String>) -> Self {
        Self {
            inputs,
            outputs: HashMap::new(),
        }
    }

    /// Build a template variable map: outputs shadow inputs.
    pub fn to_template_context(&self) -> HashMap<String, String> {
        let mut vars = self.inputs.clone();
        vars.extend(self.outputs.clone());
        vars
    }
}

/// Result of executing a workflow.
#[derive(Debug)]
pub struct WorkflowResult {
    pub outputs: HashMap<String, String>,
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
#[tracing::instrument(
    name = "workflow_execute",
    skip(workflow, inputs, prompt_executor, tool_executor, workflow_dir),
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
) -> Result<WorkflowResult, MvError> {
    // Validate inputs
    let resolved_inputs = validate_inputs(workflow, inputs)?;
    let mut ctx = ExecutionContext::new(resolved_inputs);

    // Get default model
    let default_model = workflow
        .defaults
        .as_ref()
        .and_then(|d| d.model.clone())
        .unwrap_or_else(|| "qwen3:4b".to_string());
    let default_temp = workflow.defaults.as_ref().and_then(|d| d.temperature);
    let default_max_tokens = workflow.defaults.as_ref().and_then(|d| d.max_tokens);

    // Execute steps sequentially
    for step in &workflow.steps {
        let step_span = tracing::info_span!(
            "workflow_step",
            step.id = %step.id(),
            step.type = %step_type_name(step),
            step.output = %step.output(),
        );
        let _enter = step_span.enter();
        let start = std::time::Instant::now();

        debug!(step_id = %step.id(), step_type = %step_type_name(step), "executing step");

        let output = match step {
            Step::Prompt(ps) => {
                let model = ps.model.as_deref().unwrap_or(&default_model);
                let temp = ps.temperature.or(default_temp);
                let max_tok = ps.max_tokens.or(default_max_tokens);

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

                prompt_executor
                    .execute_prompt(&rendered, model, temp, max_tok)
                    .await
                    .map_err(|e| MvError::WorkflowStepFailed {
                        step: ps.id.clone(),
                        details: e.to_string(),
                    })?
            }
            Step::Tool(ts) => {
                // Render tool inputs from context
                let vars = ctx.to_template_context();
                let mut rendered_inputs = HashMap::new();
                for (key, val) in &ts.inputs {
                    if let Some(s) = val.as_str() {
                        let rendered = template::render_template(s, &vars).map_err(|e| {
                            MvError::WorkflowTemplateError {
                                step: ts.id.clone(),
                                details: e.to_string(),
                            }
                        })?;
                        rendered_inputs.insert(key.clone(), serde_json::Value::String(rendered));
                    } else {
                        rendered_inputs.insert(key.clone(), val.clone());
                    }
                }

                execute_tool_with_error_handling(ts, &rendered_inputs, tool_executor).await?
            }
            Step::Transform(ts) => {
                let vars = ctx.to_template_context();
                let input_value = template::render_template(&ts.input, &vars).map_err(|e| {
                    MvError::WorkflowTemplateError {
                        step: ts.id.clone(),
                        details: e.to_string(),
                    }
                })?;

                execute_transform(&ts.id, &ts.operation, &input_value, ts.schema.as_ref())?
            }
        };

        info!(
            step_id = %step.id(),
            output_name = %step.output(),
            output_len = output.len(),
            duration_ms = start.elapsed().as_millis() as u64,
            "step completed"
        );
        ctx.outputs.insert(step.output().to_string(), output);
    }

    // Build final outputs
    let final_outputs = build_workflow_outputs(workflow, &ctx);
    Ok(WorkflowResult {
        outputs: final_outputs,
    })
}

fn step_type_name(step: &Step) -> &'static str {
    match step {
        Step::Prompt(_) => "prompt",
        Step::Tool(_) => "tool",
        Step::Transform(_) => "transform",
    }
}

async fn execute_tool_with_error_handling<T: ToolExecutor>(
    ts: &super::types::ToolStep,
    rendered_inputs: &HashMap<String, serde_json::Value>,
    tool_executor: &T,
) -> Result<String, MvError> {
    let execute = || async { tool_executor.execute_tool(&ts.tool, rendered_inputs).await };

    match &ts.on_error {
        ErrorAction::Fail => execute().await.map_err(|e| MvError::WorkflowStepFailed {
            step: ts.id.clone(),
            details: format!("tool '{}' failed: {e}", ts.tool),
        }),
        ErrorAction::Skip => match execute().await {
            Ok(output) => Ok(output),
            Err(e) => {
                warn!(
                    step_id = %ts.id,
                    tool = %ts.tool,
                    error = %e,
                    "tool failed, skipping"
                );
                Ok(String::new())
            }
        },
        ErrorAction::Retry => {
            let retry = ts.retry.as_ref();
            let max_attempts = retry.map_or(3, |r| r.max_attempts);
            let is_exponential = retry.is_none_or(|r| {
                r.backoff == super::types::BackoffStrategy::Exponential
            });

            for attempt in 1..=max_attempts {
                match execute().await {
                    Ok(output) => return Ok(output),
                    Err(e) => {
                        if attempt == max_attempts {
                            return Err(MvError::WorkflowStepFailed {
                                step: ts.id.clone(),
                                details: format!(
                                    "tool '{}' failed after {max_attempts} attempts: {e}",
                                    ts.tool
                                ),
                            });
                        }
                        let delay_ms = if is_exponential {
                            100 * 2u64.pow(attempt - 1)
                        } else {
                            100
                        };
                        warn!(
                            step_id = %ts.id,
                            tool = %ts.tool,
                            attempt = attempt,
                            max_attempts = max_attempts,
                            "tool failed, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }
            unreachable!()
        }
    }
}

/// Execute a transform step.
pub fn execute_transform(
    step_id: &str,
    operation: &str,
    input: &str,
    schema: Option<&serde_json::Value>,
) -> Result<String, MvError> {
    match operation {
        "extract_json" => extract_json(step_id, input, schema),
        other => Err(MvError::WorkflowStepFailed {
            step: step_id.to_string(),
            details: format!("unknown transform operation: {other}"),
        }),
    }
}

/// Extract JSON from text, handling markdown code fences.
fn extract_json(
    step_id: &str,
    input: &str,
    schema: Option<&serde_json::Value>,
) -> Result<String, MvError> {
    // Try to extract JSON from markdown code fences first
    let json_str = if let Some(start) = input.find("```json") {
        let content_start = start + 7;
        let end = input[content_start..]
            .find("```")
            .map(|e| content_start + e)
            .unwrap_or(input.len());
        input[content_start..end].trim()
    } else if let Some(start) = input.find("```") {
        let content_start = start + 3;
        // Skip the language identifier line
        let after_lang = input[content_start..]
            .find('\n')
            .map(|n| content_start + n + 1)
            .unwrap_or(content_start);
        let end = input[after_lang..]
            .find("```")
            .map(|e| after_lang + e)
            .unwrap_or(input.len());
        input[after_lang..end].trim()
    } else {
        input.trim()
    };

    // Parse JSON
    let parsed: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| MvError::WorkflowStepFailed {
            step: step_id.to_string(),
            details: format!("extract_json failed: {e}"),
        })?;

    // Optional schema validation (structural comparison)
    if let Some(expected) = schema {
        validate_json_structure(&parsed, expected).map_err(|msg| MvError::WorkflowStepFailed {
            step: step_id.to_string(),
            details: format!("schema validation failed: {msg}"),
        })?;
    }

    Ok(parsed.to_string())
}

/// Simple structural comparison: check that the parsed JSON has the same
/// top-level keys and value types as the schema template.
fn validate_json_structure(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
) -> Result<(), String> {
    match (actual, expected) {
        (serde_json::Value::Object(a), serde_json::Value::Object(e)) => {
            for key in e.keys() {
                if !a.contains_key(key) {
                    return Err(format!("missing key: '{key}'"));
                }
            }
            Ok(())
        }
        (serde_json::Value::Array(_), serde_json::Value::Array(_)) => Ok(()),
        (a, e) if std::mem::discriminant(a) == std::mem::discriminant(e) => Ok(()),
        (a, e) => Err(format!(
            "type mismatch: expected {}, got {}",
            json_type_name(e),
            json_type_name(a)
        )),
    }
}

fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn build_workflow_outputs(workflow: &Workflow, ctx: &ExecutionContext) -> HashMap<String, String> {
    if workflow.outputs.is_empty() {
        // When no outputs specified, return last step's output
        if let Some(last_step) = workflow.steps.last() {
            let mut map = HashMap::new();
            if let Some(output) = ctx.outputs.get(last_step.output()) {
                map.insert(last_step.output().to_string(), output.clone());
            }
            map
        } else {
            HashMap::new()
        }
    } else {
        workflow
            .outputs
            .iter()
            .filter_map(|wo| {
                // wo.from is a step ID — find that step's output name
                let step = workflow.steps.iter().find(|s| s.id() == wo.from)?;
                let output_name = step.output();
                ctx.outputs
                    .get(output_name)
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
            _tool_name: &str,
            _inputs: &HashMap<String, serde_json::Value>,
        ) -> Result<String, MvError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Err(MvError::WorkflowStepFailed {
                    step: String::new(),
                    details: "no more mock responses".to_string(),
                })
            } else {
                responses
                    .remove(0)
                    .map_err(|e| MvError::WorkflowStepFailed {
                        step: String::new(),
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
        ctx.outputs
            .insert("topic".to_string(), "output_value".to_string());
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
        ctx.outputs
            .insert("output_key".to_string(), "output_val".to_string());
        let vars = ctx.to_template_context();
        assert_eq!(vars["input_key"], "input_val");
        assert_eq!(vars["output_key"], "output_val");
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

        let result = execute_workflow(&wf, inputs, &prompt_exec, &tool_exec, Path::new("."))
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

        let result = execute_workflow(&wf, inputs, &prompt_exec, &tool_exec, Path::new("."))
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

        let err = execute_workflow(&wf, inputs, &prompt_exec, &tool_exec, Path::new("."))
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

        let result = execute_workflow(&wf, inputs, &prompt_exec, &tool_exec, Path::new("."))
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
        )
        .await
        .unwrap_err();

        assert!(matches!(err, MvError::WorkflowStepFailed { .. }));
        assert_eq!(prompt_exec.call_count(), 0); // Next step never executed
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
        )
        .await
        .unwrap();

        assert_eq!(result.outputs["result"], "processed");
    }
}
