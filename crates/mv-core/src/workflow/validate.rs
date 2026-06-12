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
    EmptyBranchArm {
        step_id: String,
        arm: &'static str,
    },
    ConditionSyntax {
        step_id: String,
        details: String,
    },
    EmptyParallel {
        step_id: String,
    },
    EmptyLoop {
        step_id: String,
    },
    InvalidLoopMaxIterations {
        step_id: String,
    },
    SubWorkflowInvalid {
        step_id: String,
        details: String,
    },
    EmptyPreferList {
        step_id: String,
    },
    OutputFromControlStep {
        output_name: String,
        step_id: String,
    },
    MaybeUndefinedOutput {
        output_name: String,
        step_id: String,
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
            Self::EmptyBranchArm { step_id, arm } => {
                write!(f, "branch step '{step_id}' has an empty '{arm}' arm")
            }
            Self::ConditionSyntax { step_id, details } => {
                write!(
                    f,
                    "branch step '{step_id}' condition syntax error: {details}"
                )
            }
            Self::EmptyParallel { step_id } => {
                write!(f, "parallel step '{step_id}' has no child steps")
            }
            Self::EmptyLoop { step_id } => {
                write!(f, "loop step '{step_id}' has no body steps")
            }
            Self::InvalidLoopMaxIterations { step_id } => {
                write!(f, "loop step '{step_id}' must have max_iterations >= 1")
            }
            Self::SubWorkflowInvalid { step_id, details } => {
                write!(f, "workflow step '{step_id}': nested workflow {details}")
            }
            Self::EmptyPreferList { step_id } => {
                write!(
                    f,
                    "step '{step_id}' has an empty model preference list — name at least one model"
                )
            }
            Self::OutputFromControlStep {
                output_name,
                step_id,
            } => write!(
                f,
                "output '{output_name}' maps from '{step_id}', a branch/parallel step with no \
                 output of its own — map from one of its inner steps instead"
            ),
            Self::MaybeUndefinedOutput {
                output_name,
                step_id,
            } => write!(
                f,
                "output '{output_name}' maps from step '{step_id}' whose output is not defined \
                 on every execution path — define it in every branch arm, or map from a step \
                 outside the branch"
            ),
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

    // Duplicate step IDs across the whole tree (branch arms included).
    let mut all_ids: Vec<&str> = Vec::new();
    collect_step_ids(&workflow.steps, &mut all_ids);
    let mut seen_ids: HashSet<&str> = HashSet::new();
    for id in &all_ids {
        if !seen_ids.insert(id) {
            errors.push(ValidationError::DuplicateStepId(id.to_string()));
        }
    }

    // An empty `defaults.model` preference list would resolve every
    // defaulted step to zero candidates.
    if let Some(super::types::ModelSpec::Prefer { prefer }) =
        workflow.defaults.as_ref().and_then(|d| d.model.as_ref())
        && prefer.is_empty()
    {
        errors.push(ValidationError::EmptyPreferList {
            step_id: "defaults".to_string(),
        });
    }

    // Per-step reference/template/output checks, recursing through branch arms
    // with maybe-defined semantics (see `validate_steps`). The returned set is
    // the output names *definitely* defined when the workflow finishes.
    let outer = HashSet::new();
    let defined = validate_steps(
        &workflow.steps,
        &outer,
        &input_names,
        workflow_dir,
        &mut errors,
    );

    // Workflow outputs must reference an existing step (top-level or nested)
    // whose output is definitely defined on every execution path — otherwise
    // the engine would silently omit the output (e.g. `from:` a step in a
    // branch arm that did not run).
    for output in &workflow.outputs {
        if !seen_ids.contains(output.from.as_str()) {
            errors.push(ValidationError::MissingStepOutput {
                output_name: output.name.clone(),
                step_id: output.from.clone(),
            });
        } else if let Some(step) = super::types::find_step(&workflow.steps, &output.from) {
            match step.output() {
                None => errors.push(ValidationError::OutputFromControlStep {
                    output_name: output.name.clone(),
                    step_id: output.from.clone(),
                }),
                Some(name) if !defined.contains(name) => {
                    errors.push(ValidationError::MaybeUndefinedOutput {
                        output_name: output.name.clone(),
                        step_id: output.from.clone(),
                    });
                }
                Some(_) => {}
            }
        }
    }

    errors
}

/// Collect every step id in declaration order, descending into branch arms.
fn collect_step_ids<'a>(steps: &'a [Step], out: &mut Vec<&'a str>) {
    for step in steps {
        out.push(step.id());
        match step {
            Step::Branch(bs) => {
                collect_step_ids(&bs.then, out);
                collect_step_ids(&bs.otherwise, out);
            }
            Step::Parallel(par) => collect_step_ids(&par.steps, out),
            Step::Loop(ls) => collect_step_ids(&ls.steps, out),
            _ => {}
        }
    }
}

/// Validate a linear step sequence. `outer` is the set of output names
/// definitely available from the enclosing scope (before this sequence).
/// Returns the set of names this sequence *definitely* defines — for a branch,
/// that is the intersection of what its two arms define, which is how an output
/// becomes safe to reference after the branch (maybe-defined → reference error).
fn validate_steps(
    steps: &[Step],
    outer: &HashSet<String>,
    input_names: &HashSet<String>,
    workflow_dir: Option<&Path>,
    errors: &mut Vec<ValidationError>,
) -> HashSet<String> {
    // `available` = outer scope ∪ names defined so far in this sequence.
    let mut available: HashSet<String> = outer.clone();
    // Names defined in THIS scope: for in-scope duplicate detection and the
    // arm-intersection the caller uses.
    let mut defined_here: HashSet<String> = HashSet::new();

    for step in steps {
        match step {
            Step::Prompt(ps) => {
                match (&ps.template, &ps.template_file) {
                    (None, None) => errors.push(ValidationError::MissingTemplate(ps.id.clone())),
                    (Some(_), Some(_)) => {
                        errors.push(ValidationError::BothTemplates(ps.id.clone()))
                    }
                    _ => {}
                }

                if let Some(ref tmpl) = ps.template {
                    check_template(
                        &ps.id,
                        tmpl,
                        Some(&ps.output),
                        &available,
                        input_names,
                        errors,
                    );
                } else if let Some(ref file) = ps.template_file
                    && let Some(dir) = workflow_dir
                {
                    match template::load_template_file(file, dir) {
                        Ok(contents) => check_template(
                            &ps.id,
                            &contents,
                            Some(&ps.output),
                            &available,
                            input_names,
                            errors,
                        ),
                        Err(e) => errors.push(ValidationError::TemplateFileError {
                            step_id: ps.id.clone(),
                            details: e.to_string(),
                        }),
                    }
                }
                // An empty preference list resolves to zero candidate models.
                if let Some(super::types::ModelSpec::Prefer { prefer }) = &ps.model
                    && prefer.is_empty()
                {
                    errors.push(ValidationError::EmptyPreferList {
                        step_id: ps.id.clone(),
                    });
                }
                register_output(
                    &ps.id,
                    &ps.output,
                    input_names,
                    &mut available,
                    &mut defined_here,
                    errors,
                );
            }
            Step::Tool(ts) => {
                if let Some(retry) = &ts.retry
                    && retry.max_attempts == 0
                {
                    errors.push(ValidationError::InvalidRetryConfig {
                        step_id: ts.id.clone(),
                        details: "max_attempts must be at least 1".to_string(),
                    });
                }
                for val in ts.inputs.values() {
                    check_json_value_templates(&ts.id, val, &available, input_names, errors);
                }
                register_output(
                    &ts.id,
                    &ts.output,
                    input_names,
                    &mut available,
                    &mut defined_here,
                    errors,
                );
            }
            Step::Transform(ts) => {
                if !KNOWN_TRANSFORMS.contains(&ts.operation.as_str()) {
                    errors.push(ValidationError::UnknownTransformOp {
                        step_id: ts.id.clone(),
                        operation: ts.operation.clone(),
                    });
                }
                check_template(&ts.id, &ts.input, None, &available, input_names, errors);
                register_output(
                    &ts.id,
                    &ts.output,
                    input_names,
                    &mut available,
                    &mut defined_here,
                    errors,
                );
            }
            Step::Branch(bs) => {
                // Condition references must already be available; a malformed
                // condition is a syntax error.
                match template::condition_references(&bs.condition) {
                    Ok(refs) => {
                        for var in refs {
                            if !available.contains(&var) && !input_names.contains(&var) {
                                errors.push(ValidationError::UnresolvableReference {
                                    step_id: bs.id.clone(),
                                    reference: var,
                                });
                            }
                        }
                    }
                    Err(details) => errors.push(ValidationError::ConditionSyntax {
                        step_id: bs.id.clone(),
                        details,
                    }),
                }

                if bs.then.is_empty() {
                    errors.push(ValidationError::EmptyBranchArm {
                        step_id: bs.id.clone(),
                        arm: "then",
                    });
                }

                // Each arm validates against the context available at the fork.
                let then_def =
                    validate_steps(&bs.then, &available, input_names, workflow_dir, errors);
                let else_def =
                    validate_steps(&bs.otherwise, &available, input_names, workflow_dir, errors);

                // Only outputs defined in BOTH arms are definitely available
                // afterwards. (An empty `else` defines nothing, so a branch with
                // no `else` makes none of its outputs unconditionally available.)
                for name in then_def.intersection(&else_def) {
                    available.insert(name.clone());
                    defined_here.insert(name.clone());
                }
            }
            Step::Parallel(par) => {
                if par.steps.is_empty() {
                    errors.push(ValidationError::EmptyParallel {
                        step_id: par.id.clone(),
                    });
                }

                // Every child validates against the pre-fork context only —
                // siblings are invisible to each other by construction.
                let mut produced: HashSet<String> = HashSet::new();
                for child in &par.steps {
                    let child_def = validate_steps(
                        std::slice::from_ref(child),
                        &available,
                        input_names,
                        workflow_dir,
                        errors,
                    );
                    // Children's outputs must be disjoint — at the join two
                    // children writing the same name would race.
                    for name in child_def {
                        if !produced.insert(name.clone()) {
                            errors.push(ValidationError::DuplicateOutputName {
                                output_name: name,
                                step_id: child.id().to_string(),
                            });
                        }
                    }
                }

                // All children run, so every produced output is definitely
                // available after the join.
                for name in produced {
                    available.insert(name.clone());
                    defined_here.insert(name);
                }
            }
            Step::Loop(ls) => {
                if ls.max_iterations == 0 {
                    errors.push(ValidationError::InvalidLoopMaxIterations {
                        step_id: ls.id.clone(),
                    });
                }
                if ls.steps.is_empty() {
                    errors.push(ValidationError::EmptyLoop {
                        step_id: ls.id.clone(),
                    });
                }

                // The body validates against the pre-loop context. It runs at
                // least once, so its definitely-defined set propagates after
                // the loop (unlike a branch, there is no other arm to intersect
                // with).
                let body_def =
                    validate_steps(&ls.steps, &available, input_names, workflow_dir, errors);

                // The exit condition is evaluated AFTER each iteration, so it
                // may reference the body's outputs as well as the outer scope.
                if let Some(cond) = &ls.exit_condition {
                    match template::condition_references(cond) {
                        Ok(refs) => {
                            for var in refs {
                                if !available.contains(&var)
                                    && !input_names.contains(&var)
                                    && !body_def.contains(&var)
                                {
                                    errors.push(ValidationError::UnresolvableReference {
                                        step_id: ls.id.clone(),
                                        reference: var,
                                    });
                                }
                            }
                        }
                        Err(details) => errors.push(ValidationError::ConditionSyntax {
                            step_id: ls.id.clone(),
                            details,
                        }),
                    }
                }

                for name in body_def {
                    available.insert(name.clone());
                    defined_here.insert(name);
                }
            }
            Step::SubWorkflow(sw) => {
                // Templated inputs must reference resolvable variables.
                for tmpl in sw.inputs.values() {
                    check_template(&sw.id, tmpl, None, &available, input_names, errors);
                }

                // When the directory is known, load and validate the child one
                // cross-file level deep. The child is validated with `None` as
                // its directory, so its OWN `workflow` steps are not followed
                // here — that bounds recursion (a cyclic pair cannot loop the
                // validator) and defers deeper checks (and cycle/depth) to
                // runtime. This still catches a missing, unparseable, or
                // structurally-broken direct child.
                if let Some(dir) = workflow_dir {
                    let child_path = dir.join(&sw.file);
                    match super::parser::load_from_file(&child_path) {
                        Ok(child_wf) => {
                            if let Some(first) = validate(&child_wf, None).into_iter().next() {
                                errors.push(ValidationError::SubWorkflowInvalid {
                                    step_id: sw.id.clone(),
                                    details: format!("'{}' is invalid: {first}", sw.file),
                                });
                            }
                        }
                        Err(e) => errors.push(ValidationError::SubWorkflowInvalid {
                            step_id: sw.id.clone(),
                            details: e.to_string(),
                        }),
                    }
                }

                register_output(
                    &sw.id,
                    &sw.output,
                    input_names,
                    &mut available,
                    &mut defined_here,
                    errors,
                );
            }
        }
    }

    defined_here
}

/// Register a leaf step's output: flag a duplicate — in this scope, or
/// shadowing an output from an enclosing scope (a branch/parallel arm step
/// silently overwriting an earlier top-level output is the same accident the
/// top-level duplicate check catches) — warn on input shadowing, and add it
/// to the available/defined sets. The same name in two sibling branch arms is
/// NOT a duplicate: the arms are mutually exclusive scopes, and defining an
/// output in both is how it becomes definitely-defined after the branch.
fn register_output(
    step_id: &str,
    output: &str,
    input_names: &HashSet<String>,
    available: &mut HashSet<String>,
    defined_here: &mut HashSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    let shadows_outer = available.contains(output) && !defined_here.contains(output);
    if !defined_here.insert(output.to_string()) || shadows_outer {
        errors.push(ValidationError::DuplicateOutputName {
            output_name: output.to_string(),
            step_id: step_id.to_string(),
        });
    }
    available.insert(output.to_string());
    if input_names.contains(output) {
        tracing::warn!(
            step_id = %step_id,
            output = %output,
            "step output shadows a workflow input of the same name"
        );
    }
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

    // --- 009/WS3: branch validation ---

    #[test]
    fn branch_both_arms_define_output_is_valid() {
        // The recommended pattern: both arms define `answer`, so it is
        // definitely available afterwards.
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: topic
    type: string
  - name: style
    type: string
steps:
  - id: route
    type: branch
    condition: "style == 'detailed'"
    then:
      - id: deep
        type: prompt
        output: answer
        template: "Deep {{topic}}"
    else:
      - id: quick
        type: prompt
        output: answer
        template: "Quick {{topic}}"
  - id: use
    type: prompt
    output: final
    template: "Use {{answer}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty(), "{:?}", validate(&wf, None));
    }

    #[test]
    fn branch_output_defined_in_one_arm_is_maybe_undefined() {
        // `answer` is only defined in `then`; referencing it after the branch
        // must be a reference error (the else arm would leave it undefined).
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: topic
    type: string
  - name: style
    type: string
steps:
  - id: route
    type: branch
    condition: "style == 'detailed'"
    then:
      - id: deep
        type: prompt
        output: answer
        template: "Deep {{topic}}"
  - id: use
    type: prompt
    output: final
    template: "Use {{answer}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvableReference { step_id, reference }
                    if step_id == "use" && reference == "answer"
            )),
            "expected maybe-undefined reference error, got: {errors:?}"
        );
    }

    #[test]
    fn branch_arm_can_reference_prior_output() {
        // Outputs defined before the branch are available inside the arms.
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: setup
    type: prompt
    output: base
    template: "base"
  - id: route
    type: branch
    condition: "base"
    then:
      - id: deep
        type: prompt
        output: answer
        template: "Use {{base}}"
    else:
      - id: quick
        type: prompt
        output: answer
        template: "Also {{base}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty(), "{:?}", validate(&wf, None));
    }

    #[test]
    fn branch_arm_cannot_reference_sibling_arm_output() {
        // `then` defines `a`; `else` referencing `a` must fail — arms are
        // mutually exclusive, so `a` is not available in the else arm.
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: route
    type: branch
    condition: "flag"
    then:
      - id: t
        type: prompt
        output: a
        template: "hello"
    else:
      - id: e
        type: prompt
        output: b
        template: "Use {{a}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvableReference { reference, .. } if reference == "a"
            )),
            "got: {errors:?}"
        );
    }

    #[test]
    fn branch_unknown_condition_variable_rejected() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: route
    type: branch
    condition: "missing_flag == 'x'"
    then:
      - id: t
        type: prompt
        output: out
        template: "hi"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvableReference { step_id, reference }
                    if step_id == "route" && reference == "missing_flag"
            )),
            "got: {errors:?}"
        );
    }

    #[test]
    fn branch_malformed_condition_is_syntax_error() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: route
    type: branch
    condition: "=="
    then:
      - id: t
        type: prompt
        output: out
        template: "hi"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(
                |e| matches!(e, ValidationError::ConditionSyntax { step_id, .. } if step_id == "route")
            ),
            "got: {errors:?}"
        );
    }

    #[test]
    fn branch_empty_then_arm_rejected() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: route
    type: branch
    condition: "flag"
    then: []
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(
                |e| matches!(e, ValidationError::EmptyBranchArm { step_id, arm } if step_id == "route" && *arm == "then")
            ),
            "got: {errors:?}"
        );
    }

    #[test]
    fn branch_duplicate_step_id_across_arms_rejected() {
        // The same id in both arms is a duplicate (ids are workflow-global).
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: route
    type: branch
    condition: "flag"
    then:
      - id: dup
        type: prompt
        output: a
        template: "hi"
    else:
      - id: dup
        type: prompt
        output: b
        template: "bye"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::DuplicateStepId(id) if id == "dup")),
            "got: {errors:?}"
        );
    }

    #[test]
    fn nested_branch_maybe_defined_propagates() {
        // `answer` is defined in both inner arms (so definitely defined after
        // the inner branch) AND in the outer else — so it is available after
        // the outer branch and the final step validates clean.
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: a
    type: string
  - name: b
    type: string
steps:
  - id: outer
    type: branch
    condition: "a"
    then:
      - id: inner
        type: branch
        condition: "b"
        then:
          - id: t1
            type: prompt
            output: answer
            template: "t1"
        else:
          - id: t2
            type: prompt
            output: answer
            template: "t2"
    else:
      - id: e1
        type: prompt
        output: answer
        template: "e1"
  - id: use
    type: prompt
    output: final
    template: "Use {{answer}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty(), "{:?}", validate(&wf, None));
    }

    #[test]
    fn nested_branch_maybe_defined_in_inner_one_arm_fails() {
        // Inner branch defines `answer` only in its `then`; the outer else also
        // defines it. After the inner branch `answer` is maybe-undefined, so the
        // outer `then` does not definitely define it → reference after outer fails.
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: a
    type: string
  - name: b
    type: string
steps:
  - id: outer
    type: branch
    condition: "a"
    then:
      - id: inner
        type: branch
        condition: "b"
        then:
          - id: t1
            type: prompt
            output: answer
            template: "t1"
    else:
      - id: e1
        type: prompt
        output: answer
        template: "e1"
  - id: use
    type: prompt
    output: final
    template: "Use {{answer}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvableReference { step_id, reference }
                    if step_id == "use" && reference == "answer"
            )),
            "got: {errors:?}"
        );
    }

    // --- 009/WS4: parallel validation ---

    #[test]
    fn parallel_disjoint_outputs_valid() {
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: topic
    type: string
steps:
  - id: fan
    type: parallel
    steps:
      - id: a
        type: prompt
        output: out_a
        template: "A {{topic}}"
      - id: b
        type: prompt
        output: out_b
        template: "B {{topic}}"
  - id: combine
    type: prompt
    output: final
    template: "{{out_a}} {{out_b}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty(), "{:?}", validate(&wf, None));
    }

    #[test]
    fn parallel_sibling_output_not_visible() {
        // Child `b` references `out_a` (a sibling's output) — invisible by
        // construction (each child sees only the pre-fork context).
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: topic
    type: string
steps:
  - id: fan
    type: parallel
    steps:
      - id: a
        type: prompt
        output: out_a
        template: "A {{topic}}"
      - id: b
        type: prompt
        output: out_b
        template: "B {{out_a}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::UnresolvableReference { step_id, reference }
                    if step_id == "b" && reference == "out_a"
            )),
            "got: {errors:?}"
        );
    }

    #[test]
    fn parallel_duplicate_child_output_rejected() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: fan
    type: parallel
    steps:
      - id: a
        type: prompt
        output: same
        template: "A"
      - id: b
        type: prompt
        output: same
        template: "B"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::DuplicateOutputName { output_name, step_id }
                    if output_name == "same" && step_id == "b"
            )),
            "got: {errors:?}"
        );
    }

    #[test]
    fn parallel_outputs_available_after_join() {
        // Unlike a branch (intersection), ALL parallel children run, so every
        // child output is available afterwards.
        let yaml = r#"
name: test
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
  - id: use_a
    type: prompt
    output: c
    template: "{{out_a}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty(), "{:?}", validate(&wf, None));
    }

    #[test]
    fn parallel_empty_rejected() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: fan
    type: parallel
    steps: []
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(
                |e| matches!(e, ValidationError::EmptyParallel { step_id } if step_id == "fan")
            ),
            "got: {errors:?}"
        );
    }

    #[test]
    fn subworkflow_broken_child_fails_parent_validation() {
        // `workflow validate` loads the child (dir known) and validates it; a
        // child with an unresolvable template reference fails the parent.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("child.yaml"),
            "name: child\nversion: \"1.0\"\nsteps:\n  - id: bad\n    type: prompt\n    \
             output: o\n    template: \"{{undefined_var}}\"\n",
        )
        .unwrap();
        let parent = r#"
name: parent
version: "1.0"
steps:
  - id: sub
    type: workflow
    file: child.yaml
    output: result
"#;
        let wf = parser::load_from_str(parent, "parent.yaml").unwrap();
        let errors = validate(&wf, Some(dir.path()));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::SubWorkflowInvalid { step_id, .. } if step_id == "sub")),
            "got: {errors:?}"
        );
    }

    #[test]
    fn subworkflow_missing_child_fails_validation() {
        let dir = tempfile::tempdir().unwrap();
        let parent = r#"
name: parent
version: "1.0"
steps:
  - id: sub
    type: workflow
    file: nope.yaml
    output: result
"#;
        let wf = parser::load_from_str(parent, "parent.yaml").unwrap();
        let errors = validate(&wf, Some(dir.path()));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::SubWorkflowInvalid { .. })),
            "got: {errors:?}"
        );
    }

    #[test]
    fn loop_zero_iterations_and_empty_body_rejected() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: spin
    type: loop
    max_iterations: 0
    steps: []
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::InvalidLoopMaxIterations { step_id } if step_id == "spin"
            )),
            "got: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::EmptyLoop { step_id } if step_id == "spin")),
            "got: {errors:?}"
        );
    }

    #[test]
    fn loop_exit_condition_may_reference_body_output() {
        // The condition runs after each iteration, so referencing the body's
        // own output `draft` must validate (it is defined by the time the
        // condition is evaluated).
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: refine
    type: loop
    max_iterations: 3
    exit_condition: "draft == 'good'"
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
        let errors = validate(&wf, None);
        assert!(errors.is_empty(), "expected clean, got: {errors:?}");
    }

    #[test]
    fn loop_body_output_is_available_after_the_loop() {
        // A loop runs at least once, so its body's outputs are definitely
        // defined afterwards (unlike a one-armed branch).
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: refine
    type: loop
    max_iterations: 2
    steps:
      - id: improve
        type: prompt
        output: draft
        template: "improve"
  - id: use
    type: prompt
    output: final
    template: "use {{draft}}"
outputs:
  - name: result
    from: use
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(errors.is_empty(), "expected clean, got: {errors:?}");
    }

    #[test]
    fn parallel_child_can_reference_prior_output() {
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: setup
    type: prompt
    output: base
    template: "base"
  - id: fan
    type: parallel
    steps:
      - id: a
        type: prompt
        output: out_a
        template: "A {{base}}"
      - id: b
        type: prompt
        output: out_b
        template: "B {{base}}"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty(), "{:?}", validate(&wf, None));
    }

    // --- 009 review polish: outputs maybe-defined, empty prefer, arm shadowing ---

    #[test]
    fn output_from_step_in_one_arm_rejected() {
        // `from:` a step inside a branch arm whose output is not defined in
        // every arm → the output would silently vanish when the other arm runs.
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: flag
    type: string
steps:
  - id: route
    type: branch
    condition: "flag == 'on'"
    then:
      - id: only_then
        type: prompt
        output: answer
        template: "hi"
outputs:
  - name: result
    from: only_then
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::MaybeUndefinedOutput { output_name, step_id }
                    if output_name == "result" && step_id == "only_then"
            )),
            "got: {errors:?}"
        );
    }

    #[test]
    fn output_from_both_arm_step_accepted() {
        // Both arms define `answer`, so mapping from either arm's step is safe.
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: flag
    type: string
steps:
  - id: route
    type: branch
    condition: "flag == 'on'"
    then:
      - id: t
        type: prompt
        output: answer
        template: "hi"
    else:
      - id: e
        type: prompt
        output: answer
        template: "bye"
outputs:
  - name: result
    from: t
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty(), "{:?}", validate(&wf, None));
    }

    #[test]
    fn output_from_parallel_child_accepted() {
        // All parallel children run, so their outputs are always defined.
        let yaml = r#"
name: test
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
outputs:
  - name: result
    from: a
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        assert!(validate(&wf, None).is_empty(), "{:?}", validate(&wf, None));
    }

    #[test]
    fn output_from_control_step_rejected() {
        // A branch step has no single output to map from.
        let yaml = r#"
name: test
version: "1.0"
inputs:
  - name: flag
    type: string
steps:
  - id: route
    type: branch
    condition: "flag == 'on'"
    then:
      - id: t
        type: prompt
        output: answer
        template: "hi"
    else:
      - id: e
        type: prompt
        output: answer
        template: "bye"
outputs:
  - name: result
    from: route
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::OutputFromControlStep { step_id, .. } if step_id == "route"
            )),
            "got: {errors:?}"
        );
    }

    #[test]
    fn empty_prefer_list_rejected_on_step_and_defaults() {
        let yaml = r#"
name: test
version: "1.0"
defaults:
  model:
    prefer: []
steps:
  - id: s1
    type: prompt
    output: out
    model:
      prefer: []
    template: "hi"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::EmptyPreferList { step_id } if step_id == "s1"
            )),
            "got: {errors:?}"
        );
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::EmptyPreferList { step_id } if step_id == "defaults"
            )),
            "got: {errors:?}"
        );
    }

    #[test]
    fn arm_step_shadowing_outer_output_rejected() {
        // A branch arm step reusing a top-level output name would silently
        // overwrite it at runtime — same accident as a top-level duplicate.
        let yaml = r#"
name: test
version: "1.0"
steps:
  - id: setup
    type: prompt
    output: data
    template: "base"
  - id: route
    type: branch
    condition: "data"
    then:
      - id: clobber
        type: prompt
        output: data
        template: "overwrites"
"#;
        let wf = parser::load_from_str(yaml, "test.yaml").unwrap();
        let errors = validate(&wf, None);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::DuplicateOutputName { output_name, step_id }
                    if output_name == "data" && step_id == "clobber"
            )),
            "got: {errors:?}"
        );
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
