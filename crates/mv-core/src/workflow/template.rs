use std::collections::HashMap;
use std::path::Path;

use crate::MvError;

/// Render a template string with the given context variables.
pub fn render_template(
    template: &str,
    context: &HashMap<String, String>,
) -> Result<String, MvError> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    env.add_template("__inline", template)
        .map_err(|e| MvError::WorkflowTemplateError {
            step: String::new(),
            details: e.to_string(),
        })?;
    let tmpl = env
        .get_template("__inline")
        .map_err(|e| MvError::WorkflowTemplateError {
            step: String::new(),
            details: e.to_string(),
        })?;
    tmpl.render(context)
        .map_err(|e| MvError::WorkflowTemplateError {
            step: String::new(),
            details: e.to_string(),
        })
}

/// Load a template from a file, resolving the path relative to the workflow directory.
pub fn load_template_file(template_path: &str, workflow_dir: &Path) -> Result<String, MvError> {
    let resolved = workflow_dir.join(template_path);
    std::fs::read_to_string(&resolved).map_err(|_| MvError::WorkflowTemplateError {
        step: String::new(),
        details: format!("template file not found: {}", resolved.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn variable_substitution() {
        let result = render_template("Hello {{name}}!", &ctx(&[("name", "world")])).unwrap();
        assert_eq!(result, "Hello world!");
    }

    #[test]
    fn multiple_variables() {
        let result = render_template(
            "{{greeting}}, {{name}}! Topic: {{topic}}",
            &ctx(&[("greeting", "Hi"), ("name", "Alice"), ("topic", "Rust")]),
        )
        .unwrap();
        assert_eq!(result, "Hi, Alice! Topic: Rust");
    }

    #[test]
    fn missing_variable_error() {
        let err = render_template("Hello {{missing}}!", &ctx(&[])).unwrap_err();
        assert!(matches!(err, MvError::WorkflowTemplateError { .. }));
    }

    #[test]
    fn empty_template() {
        let result = render_template("", &ctx(&[])).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn no_variables_passthrough() {
        let result = render_template("Plain text, no vars.", &ctx(&[])).unwrap();
        assert_eq!(result, "Plain text, no vars.");
    }

    #[test]
    fn template_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_template_file("nonexistent.txt", dir.path()).unwrap_err();
        assert!(matches!(err, MvError::WorkflowTemplateError { .. }));
    }

    #[test]
    fn load_template_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prompt.txt");
        std::fs::write(&path, "Research {{topic}} in depth").unwrap();
        let content = load_template_file("prompt.txt", dir.path()).unwrap();
        assert_eq!(content, "Research {{topic}} in depth");
    }

    #[test]
    fn outputs_shadow_inputs() {
        // When the same key exists in both inputs and outputs, outputs win
        let mut context = ctx(&[("topic", "from_input")]);
        context.insert("topic".to_string(), "from_output".to_string());
        let result = render_template("{{topic}}", &context).unwrap();
        assert_eq!(result, "from_output");
    }
}
