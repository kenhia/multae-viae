use std::path::Path;

use super::types::Workflow;
use crate::MvError;

/// Load a workflow from a YAML file.
pub fn load_from_file(path: &Path) -> Result<Workflow, MvError> {
    let path_str = path.display().to_string();
    let content = std::fs::read_to_string(path).map_err(|_| MvError::WorkflowFileNotFound {
        path: path_str.clone(),
    })?;
    load_from_str(&content, &path_str)
}

/// Parse a workflow from a YAML string.
pub fn load_from_str(yaml: &str, source: &str) -> Result<Workflow, MvError> {
    serde_yml::from_str(yaml).map_err(|e| MvError::WorkflowParseError {
        path: source.to_string(),
        details: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::types::{InputType, Step};

    const VALID_WORKFLOW: &str = r#"
name: test-workflow
version: "1.0"
description: A test workflow
defaults:
  model: qwen3:4b
inputs:
  - name: topic
    type: string
    required: true
    description: The topic to research
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

    #[test]
    fn parse_valid_workflow() {
        let wf = load_from_str(VALID_WORKFLOW, "test.yaml").unwrap();
        assert_eq!(wf.name, "test-workflow");
        assert_eq!(wf.version, "1.0");
        assert_eq!(wf.description.as_deref(), Some("A test workflow"));
        assert_eq!(wf.inputs.len(), 1);
        assert_eq!(wf.inputs[0].name, "topic");
        assert_eq!(wf.inputs[0].input_type, InputType::String);
        assert!(wf.inputs[0].required);
        assert_eq!(wf.steps.len(), 2);
        assert_eq!(wf.outputs.len(), 1);
        assert_eq!(wf.outputs[0].from, "summarize");
    }

    #[test]
    fn parse_step_types() {
        let wf = load_from_str(VALID_WORKFLOW, "test.yaml").unwrap();
        assert!(matches!(&wf.steps[0], Step::Prompt(_)));
        assert!(matches!(&wf.steps[1], Step::Prompt(_)));
    }

    #[test]
    fn unknown_field_rejected() {
        let yaml = r#"
name: bad
version: "1.0"
bogus_field: oops
steps:
  - id: s1
    type: prompt
    output: out
    template: "hello"
"#;
        let err = load_from_str(yaml, "bad.yaml").unwrap_err();
        match err {
            MvError::WorkflowParseError { details, .. } => {
                assert!(details.contains("unknown field"), "got: {details}");
            }
            other => panic!("expected WorkflowParseError, got: {other}"),
        }
    }

    #[test]
    fn missing_required_fields() {
        let yaml = r#"
name: incomplete
"#;
        let err = load_from_str(yaml, "bad.yaml").unwrap_err();
        assert!(matches!(err, MvError::WorkflowParseError { .. }));
    }

    #[test]
    fn unknown_step_type_rejected() {
        let yaml = r#"
name: bad-step
version: "1.0"
steps:
  - id: s1
    type: magic
    output: out
    template: "hello"
"#;
        let err = load_from_str(yaml, "bad.yaml").unwrap_err();
        assert!(matches!(err, MvError::WorkflowParseError { .. }));
    }

    #[test]
    fn empty_yaml() {
        let err = load_from_str("", "empty.yaml").unwrap_err();
        assert!(matches!(err, MvError::WorkflowParseError { .. }));
    }

    #[test]
    fn parse_workflow_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.yaml");
        std::fs::write(&path, VALID_WORKFLOW).unwrap();
        let wf = load_from_file(&path).unwrap();
        assert_eq!(wf.name, "test-workflow");
    }

    #[test]
    fn file_not_found() {
        let err = load_from_file(Path::new("/nonexistent/workflow.yaml")).unwrap_err();
        assert!(matches!(err, MvError::WorkflowFileNotFound { .. }));
    }

    #[test]
    fn parse_tool_step() {
        let yaml = r#"
name: tool-test
version: "1.0"
steps:
  - id: list
    type: tool
    output: files
    tool: file_list
    inputs:
      path: "."
"#;
        let wf = load_from_str(yaml, "test.yaml").unwrap();
        assert!(matches!(&wf.steps[0], Step::Tool(_)));
        if let Step::Tool(t) = &wf.steps[0] {
            assert_eq!(t.tool, "file_list");
        }
    }

    #[test]
    fn parse_transform_step() {
        let yaml = r#"
name: transform-test
version: "1.0"
steps:
  - id: extract
    type: transform
    output: data
    operation: extract_json
    input: "{{raw_output}}"
"#;
        let wf = load_from_str(yaml, "test.yaml").unwrap();
        assert!(matches!(&wf.steps[0], Step::Transform(_)));
    }

    #[test]
    fn parse_branch_step() {
        let yaml = r#"
name: branch-test
version: "1.0"
steps:
  - id: route
    type: branch
    condition: "style == 'detailed'"
    then:
      - id: deep
        type: prompt
        output: answer
        template: "Deep: {{topic}}"
    else:
      - id: quick
        type: prompt
        output: answer
        template: "Quick: {{topic}}"
"#;
        let wf = load_from_str(yaml, "test.yaml").unwrap();
        match &wf.steps[0] {
            Step::Branch(b) => {
                assert_eq!(b.id, "route");
                assert_eq!(b.condition, "style == 'detailed'");
                assert_eq!(b.then.len(), 1);
                assert_eq!(b.otherwise.len(), 1);
                assert_eq!(b.then[0].id(), "deep");
                assert_eq!(b.otherwise[0].id(), "quick");
            }
            other => panic!("expected Branch, got {other:?}"),
        }
        // A branch step produces no single output.
        assert_eq!(wf.steps[0].output(), None);
    }

    #[test]
    fn parse_branch_without_else() {
        let yaml = r#"
name: branch-no-else
version: "1.0"
steps:
  - id: maybe
    type: branch
    condition: "flag"
    then:
      - id: act
        type: prompt
        output: out
        template: "Act"
"#;
        let wf = load_from_str(yaml, "test.yaml").unwrap();
        match &wf.steps[0] {
            Step::Branch(b) => assert!(b.otherwise.is_empty()),
            other => panic!("expected Branch, got {other:?}"),
        }
    }

    #[test]
    fn parse_parallel_step() {
        let yaml = r#"
name: parallel-test
version: "1.0"
steps:
  - id: fanout
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
"#;
        let wf = load_from_str(yaml, "test.yaml").unwrap();
        match &wf.steps[0] {
            Step::Parallel(p) => {
                assert_eq!(p.id, "fanout");
                assert_eq!(p.steps.len(), 2);
                assert_eq!(p.steps[0].id(), "a");
                assert_eq!(p.steps[1].id(), "b");
            }
            other => panic!("expected Parallel, got {other:?}"),
        }
        assert_eq!(wf.steps[0].output(), None);
    }

    #[test]
    fn parse_defaults() {
        let wf = load_from_str(VALID_WORKFLOW, "test.yaml").unwrap();
        let defaults = wf.defaults.unwrap();
        assert_eq!(defaults.model.as_deref(), Some("qwen3:4b"));
    }

    #[test]
    fn parse_enum_input() {
        let yaml = r#"
name: enum-test
version: "1.0"
inputs:
  - name: style
    type: enum
    values: [brief, detailed]
    default: brief
steps:
  - id: s1
    type: prompt
    output: out
    template: "hello"
"#;
        let wf = load_from_str(yaml, "test.yaml").unwrap();
        assert_eq!(wf.inputs[0].input_type, InputType::Enum);
        assert_eq!(wf.inputs[0].values, vec!["brief", "detailed"]);
        assert_eq!(wf.inputs[0].default.as_deref(), Some("brief"));
    }
}
