use std::collections::HashSet;

use super::types::{Step, Workflow};

/// Validation errors for workflow structural checks.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    EmptySteps,
    DuplicateStepId(String),
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
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySteps => write!(f, "workflow has no steps"),
            Self::DuplicateStepId(id) => write!(f, "duplicate step id '{id}'"),
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
        }
    }
}

const KNOWN_TRANSFORMS: &[&str] = &["extract_json"];

/// Validate a parsed workflow for structural errors.
///
/// Returns an empty vec if the workflow is valid.
pub fn validate(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Check empty steps
    if workflow.steps.is_empty() {
        errors.push(ValidationError::EmptySteps);
        return errors;
    }

    // Check duplicate step IDs
    let mut seen_ids = HashSet::new();
    let mut step_outputs = HashSet::new();
    for step in &workflow.steps {
        if !seen_ids.insert(step.id()) {
            errors.push(ValidationError::DuplicateStepId(step.id().to_string()));
        }
        step_outputs.insert(step.output().to_string());
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

                // Check for circular self-reference in template
                if let Some(ref tmpl) = ps.template {
                    if tmpl.contains(&format!("{{{{{}}}}}", ps.output)) {
                        errors.push(ValidationError::CircularReference(ps.id.clone()));
                    }
                    // Check for unresolvable references to step outputs
                    check_template_references(
                        &ps.id,
                        tmpl,
                        &prior_outputs,
                        &workflow.inputs.iter().map(|i| i.name.clone()).collect(),
                        &mut errors,
                    );
                }
            }
            Step::Tool(ts) => {
                // Check tool input template references
                for val in ts.inputs.values() {
                    if let Some(s) = val.as_str() {
                        check_template_references(
                            &ts.id,
                            s,
                            &prior_outputs,
                            &workflow.inputs.iter().map(|i| i.name.clone()).collect(),
                            &mut errors,
                        );
                    }
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

                // Check template references in input
                check_template_references(
                    &ts.id,
                    &ts.input,
                    &prior_outputs,
                    &workflow.inputs.iter().map(|i| i.name.clone()).collect(),
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

/// Extract `{{var}}` references from a template and check they exist in
/// prior outputs or workflow inputs.
fn check_template_references(
    step_id: &str,
    template: &str,
    prior_outputs: &HashSet<String>,
    input_names: &HashSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    let mut remaining = template;
    while let Some(start) = remaining.find("{{") {
        let after = &remaining[start + 2..];
        if let Some(end) = after.find("}}") {
            let var_name = after[..end].trim();
            if !var_name.is_empty()
                && !prior_outputs.contains(var_name)
                && !input_names.contains(var_name)
            {
                errors.push(ValidationError::UnresolvableReference {
                    step_id: step_id.to_string(),
                    reference: var_name.to_string(),
                });
            }
            remaining = &after[end + 2..];
        } else {
            break;
        }
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
        assert!(validate(&wf).is_empty());
    }

    #[test]
    fn empty_steps() {
        let yaml = r#"
name: test
version: "1.0"
steps: []
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf);
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
        let errors = validate(&wf);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::DuplicateStepId(id) if id == "dup"))
        );
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
        let errors = validate(&wf);
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
        let errors = validate(&wf);
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
        let errors = validate(&wf);
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
        let errors = validate(&wf);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::UnresolvableReference { reference, .. } if reference == "nonexistent"
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
        let errors = validate(&wf);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::MissingStepOutput { step_id, .. } if step_id == "nonexistent"
        )));
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
        let errors = validate(&wf);
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::UnknownTransformOp { operation, .. } if operation == "magic_transform"
        )));
    }
}
