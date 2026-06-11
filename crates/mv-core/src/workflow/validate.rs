use std::collections::HashSet;
use std::path::Path;

use super::template;
use super::transform::KNOWN_TRANSFORMS;
use super::types::{Step, Workflow};

/// Validation errors for workflow structural checks.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    EmptySteps,
    DuplicateStepId(String),
    DuplicateOutputName {
        output_name: String,
        step_id: String,
    },
    MissingTemplate(String),
    BothTemplates(String),
    MissingStepOutput {
        output_name: String,
        step_id: String,
    },
    CircularReference(String),
    UnresolvableReference {
        step_id: String,
        reference: String,
    },
    UnknownTransformOp {
        step_id: String,
        operation: String,
    },
    InvalidRetryConfig {
        step_id: String,
        details: String,
    },
    TemplateSyntax {
        step_id: String,
        details: String,
    },
    TemplateFileError {
        step_id: String,
        details: String,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySteps => write!(f, "workflow has no steps"),
            Self::DuplicateStepId(id) => write!(f, "duplicate step id '{id}'"),
            Self::DuplicateOutputName {
                output_name,
                step_id,
            } => write!(
                f,
                "step '{step_id}' reuses output name '{output_name}' — a later step would silently overwrite the earlier value"
            ),
            Self::MissingTemplate(id) => {
                write!(f, "prompt step '{id}' has no template or template_file")
            }
            Self::BothTemplates(id) => {
                write!(f, "prompt step '{id}' has both template and template_file")
            }
            Self::MissingStepOutput {
                output_name,
                step_id,
            } => write!(
                f,
                "output '{output_name}' references unknown step '{step_id}'"
            ),
            Self::CircularReference(id) => {
                write!(f, "step '{id}' references its own output")
            }
            Self::UnresolvableReference { step_id, reference } => {
                write!(
                    f,
                    "step '{step_id}' references unknown output '{reference}'"
                )
            }
            Self::UnknownTransformOp { step_id, operation } => {
                write!(
                    f,
                    "step '{step_id}' uses unknown transform operation '{operation}'"
                )
            }
            Self::InvalidRetryConfig { step_id, details } => {
                write!(f, "step '{step_id}' has invalid retry config: {details}")
            }
            Self::TemplateSyntax { step_id, details } => {
                write!(f, "step '{step_id}' template syntax error: {details}")
            }
            Self::TemplateFileError { step_id, details } => {
                write!(f, "step '{step_id}': {details}")
            }
        }
    }
}

/// Validate a parsed workflow for structural errors.
///
/// `workflow_dir` enables `template_file` validation (existence, syntax, and
/// reference checks); pass `None` when no directory context exists, in which
/// case file-based templates are only checked at runtime.
///
/// Returns an empty vec if the workflow is valid.
pub fn validate(workflow: &Workflow, workflow_dir: Option<&Path>) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Check empty steps
    if workflow.steps.is_empty() {
        errors.push(ValidationError::EmptySteps);
        return errors;
    }

    let input_names: HashSet<String> = workflow.inputs.iter().map(|i| i.name.clone()).collect();

    // Check duplicate step IDs and duplicate output names
    let mut seen_ids = HashSet::new();
    let mut seen_outputs: HashSet<&str> = HashSet::new();
    for step in &workflow.steps {
        if !seen_ids.insert(step.id()) {
            errors.push(ValidationError::DuplicateStepId(step.id().to_string()));
        }
        if !seen_outputs.insert(step.output()) {
            errors.push(ValidationError::DuplicateOutputName {
                output_name: step.output().to_string(),
                step_id: step.id().to_string(),
            });
        }
        // Shadowing an input is legal (outputs win) but easy to do by
        // accident — surface it without failing validation.
        if input_names.contains(step.output()) {
            tracing::warn!(
                step_id = %step.id(),
                output = %step.output(),
                "step output shadows a workflow input of the same name"
            );
        }
    }

    // Per-step checks
    let mut prior_outputs: HashSet<String> = HashSet::new();
    for step in &workflow.steps {
        match step {
            Step::Prompt(ps) => {
                // Check template presence
                match (&ps.template, &ps.template_file) {
                    (None, None) => {
                        errors.push(ValidationError::MissingTemplate(ps.id.clone()));
                    }
                    (Some(_), Some(_)) => {
                        errors.push(ValidationError::BothTemplates(ps.id.clone()));
                    }
                    _ => {}
                }

                if let Some(ref tmpl) = ps.template {
                    check_template(
                        &ps.id,
                        tmpl,
                        Some(&ps.output),
                        &prior_outputs,
                        &input_names,
                        &mut errors,
                    );
                } else if let Some(ref file) = ps.template_file
                    && let Some(dir) = workflow_dir
                {
                    // Validate file-based templates too — `workflow validate`
                    // must not give false confidence for template_file steps.
                    match template::load_template_file(file, dir) {
                        Ok(contents) => check_template(
                            &ps.id,
                            &contents,
                            Some(&ps.output),
                            &prior_outputs,
                            &input_names,
                            &mut errors,
                        ),
                        Err(e) => errors.push(ValidationError::TemplateFileError {
                            step_id: ps.id.clone(),
                            details: e.to_string(),
                        }),
                    }
                }
            }
            Step::Tool(ts) => {
                // Retry config: zero attempts would mean "never execute".
                if let Some(retry) = &ts.retry
                    && retry.max_attempts == 0
                {
                    errors.push(ValidationError::InvalidRetryConfig {
                        step_id: ts.id.clone(),
                        details: "max_attempts must be at least 1".to_string(),
                    });
                }

                // Check template references in every string leaf, nested
                // values included (the engine renders them the same way).
                for val in ts.inputs.values() {
                    check_json_value_templates(
                        &ts.id,
                        val,
                        &prior_outputs,
                        &input_names,
                        &mut errors,
                    );
                }
            }
            Step::Transform(ts) => {
                // Check unknown transform operations
                if !KNOWN_TRANSFORMS.contains(&ts.operation.as_str()) {
                    errors.push(ValidationError::UnknownTransformOp {
                        step_id: ts.id.clone(),
                        operation: ts.operation.clone(),
                    });
                }

                check_template(
                    &ts.id,
                    &ts.input,
                    None,
                    &prior_outputs,
                    &input_names,
                    &mut errors,
                );
            }
        }

        prior_outputs.insert(step.output().to_string());
    }

    // Check workflow outputs reference existing steps
    for output in &workflow.outputs {
        if !seen_ids.contains(output.from.as_str()) {
            errors.push(ValidationError::MissingStepOutput {
                output_name: output.name.clone(),
                step_id: output.from.clone(),
            });
        }
    }

    errors
}

/// Parse a template with the real engine and verify every referenced
/// variable resolves to a prior output or workflow input. Using minijinja's
/// own parser keeps validation and rendering in one template language:
/// filters and `{% if %}` blocks validate correctly instead of being
/// rejected (or missed) by a string scan.
fn check_template(
    step_id: &str,
    template_str: &str,
    self_output: Option<&str>,
    prior_outputs: &HashSet<String>,
    input_names: &HashSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    let refs = match template::template_references(template_str) {
        Ok(refs) => refs,
        Err(details) => {
            errors.push(ValidationError::TemplateSyntax {
                step_id: step_id.to_string(),
                details,
            });
            return;
        }
    };

    for var in refs {
        if let Some(own) = self_output
            && var == own
            && !prior_outputs.contains(own)
        {
            errors.push(ValidationError::CircularReference(step_id.to_string()));
            continue;
        }
        if !prior_outputs.contains(&var) && !input_names.contains(&var) {
            errors.push(ValidationError::UnresolvableReference {
                step_id: step_id.to_string(),
                reference: var,
            });
        }
    }
}

/// Recurse through a JSON value checking every string leaf as a template —
/// mirrors the engine's nested rendering.
fn check_json_value_templates(
    step_id: &str,
    val: &serde_json::Value,
    prior_outputs: &HashSet<String>,
    input_names: &HashSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    match val {
        serde_json::Value::String(s) => {
            check_template(step_id, s, None, prior_outputs, input_names, errors);
        }
        serde_json::Value::Array(items) => {
            for item in items {
                check_json_value_templates(step_id, item, prior_outputs, input_names, errors);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                check_json_value_templates(step_id, item, prior_outputs, input_names, errors);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::parser;

    #[test]
    fn valid_workflow_passes() {
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: topic
    type: string
steps:
  - id: s1
    type: prompt
    output: out1
    template: "Research {{topic}}"
  - id: s2
    type: prompt
    output: out2
    template: "Summarize: {{out1}}"
outputs:
  - name: result
    from: s2
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty());
    }

    #[test]
    fn empty_steps() {
        let yaml = r#"
name: test
version: "1.0"
steps: []
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.contains(&ValidationError::EmptySteps));
    }

    #[test]
    fn duplicate_step_ids() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: dup
    type: prompt
    output: out1
    template: "hello"
  - id: dup
    type: prompt
    output: out2
    template: "world"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::DuplicateStepId(id) if id == "dup"))
        );
    }

    // --- 008/T022: output-name collision validation ---

    #[test]
    fn duplicate_output_names_rejected() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: same
    template: "hello"
  - id: s2
    type: prompt
    output: same
    template: "world"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::DuplicateOutputName { output_name, step_id }
                if output_name == "same" && step_id == "s2"
        )));
    }

    #[test]
    fn missing_template() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::MissingTemplate(id) if id == "s1"))
        );
    }

    #[test]
    fn both_templates() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
    template: "hello"
    template_file: "prompt.txt"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::BothTemplates(id) if id == "s1"))
        );
    }

    #[test]
    fn circular_self_reference() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: my_output
    template: "Use {{my_output}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::CircularReference(id) if id == "s1"))
        );
    }

    #[test]
    fn unresolvable_reference() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out1
    template: "Use {{nonexistent}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::UnresolvableReference { reference, .. } if reference == "nonexistent"
        )));
    }

    // --- 008/T021: one template language for validate and render ---

    #[test]
    fn filter_syntax_validates_clean() {
        // A naive {{...}} scanner saw "topic | upper" as a variable name and
        // rejected it; minijinja-based validation must accept it.
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: topic
    type: string
steps:
  - id: s1
    type: prompt
    output: out
    template: "Research {{ topic | upper }} today"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty());
    }

    #[test]
    fn if_block_references_are_checked() {
        // Variables referenced only inside {% if %} escaped the old scanner.
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
    template: "{% if missing_flag %}yes{% endif %}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::UnresolvableReference { reference, .. } if reference == "missing_flag"
        )));
    }

    #[test]
    fn template_syntax_error_reported() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
    template: "{% if x %}unclosed"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.iter().any(
            |e| matches!(e, ValidationError::TemplateSyntax { step_id, .. } if step_id == "s1")
        ));
    }

    #[test]
    fn template_file_references_validated_when_dir_given() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("p.txt"), "Use {{undefined_var}}").unwrap();
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
    template_file: "p.txt"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, Some(dir.path()));
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::UnresolvableReference { reference, .. } if reference == "undefined_var"
        )));
        // Without a dir, file-based templates are skipped (runtime checks them).
        assert!(validate(&wf, None).is_empty());
    }

    #[test]
    fn template_file_missing_reported_when_dir_given() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
    template_file: "absent.txt"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, Some(dir.path()));
        assert!(errors.iter().any(
            |e| matches!(e, ValidationError::TemplateFileError { step_id, .. } if step_id == "s1")
        ));
    }

    #[test]
    fn nested_tool_input_references_are_checked() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: t1
    type: tool
    output: out
    tool: http_get
    inputs:
      headers:
        auth: "Bearer {{missing_token}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::UnresolvableReference { reference, .. } if reference == "missing_token"
        )));
    }

    #[test]
    fn output_references_nonexistent_step() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: out
    template: "hello"
outputs:
  - name: result
    from: nonexistent
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::MissingStepOutput { step_id, .. } if step_id == "nonexistent"
        )));
    }

    #[test]
    fn retry_zero_attempts_rejected() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: tool
    output: out
    tool: file_list
    on_error: retry
    retry:
      max_attempts: 0
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::InvalidRetryConfig { step_id, .. } if step_id == "s1"
        )));
    }

    #[test]
    fn retry_one_attempt_is_valid() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: tool
    output: out
    tool: file_list
    on_error: retry
    retry:
      max_attempts: 1
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty());
    }

    #[test]
    fn unknown_transform_operation() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: s1
    type: prompt
    output: data
    template: "hello"
  - id: s2
    type: transform
    output: result
    operation: magic_transform
    input: "{{data}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::UnknownTransformOp { operation, .. } if operation == "magic_transform"
        )));
    }
}
