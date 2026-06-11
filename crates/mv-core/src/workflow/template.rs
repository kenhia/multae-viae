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

/// Parse `template` with the same engine used for rendering and return the
/// variables it references. This is what keeps validation and rendering
/// speaking one template language — a naive `{{…}}` scan would reject valid
/// minijinja (filters, `{%- -%}` trim markers) and miss `{% if %}` blocks.
pub fn template_references(template: &str) -> Result<std::collections::HashSet<String>, String> {
    let mut env = minijinja::Environment::new();
    env.add_template("__inline", template)
        .map_err(|e| e.to_string())?;
    let tmpl = env.get_template("__inline").map_err(|e| e.to_string())?;
    Ok(tmpl.undeclared_variables(false))
}

/// Evaluate a `branch` condition — a minijinja expression — against the
/// context, returning its truthiness. The same template language as `{{…}}`,
/// so `style == 'detailed'` or a bare `flag` both work. Note that with today's
/// all-string context a bare `flag` is truthy whenever it is a non-empty string
/// (so the literal string `"false"` is truthy); prefer explicit comparisons.
pub fn evaluate_condition(
    condition: &str,
    context: &HashMap<String, String>,
) -> Result<bool, String> {
    let env = minijinja::Environment::new();
    let expr = env
        .compile_expression(condition)
        .map_err(|e| e.to_string())?;
    let value = expr.eval(context).map_err(|e| e.to_string())?;
    Ok(value.is_true())
}

/// Variables a `branch` condition references — for validation. Compiles the
/// expression with the same engine that evaluates it (one template language),
/// so a malformed condition surfaces as an `Err` before execution.
pub fn condition_references(condition: &str) -> Result<std::collections::HashSet<String>, String> {
    let env = minijinja::Environment::new();
    let expr = env
        .compile_expression(condition)
        .map_err(|e| e.to_string())?;
    Ok(expr.undeclared_variables(false))
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
    fn condition_equality_true_and_false() {
        let vars = ctx(&[("style", "detailed")]);
        assert!(evaluate_condition("style == 'detailed'", &vars).unwrap());
        assert!(!evaluate_condition("style == 'brief'", &vars).unwrap());
    }

    #[test]
    fn condition_bare_string_truthiness() {
        // Non-empty string is truthy; empty string is falsy.
        assert!(evaluate_condition("flag", &ctx(&[("flag", "x")])).unwrap());
        assert!(!evaluate_condition("flag", &ctx(&[("flag", "")])).unwrap());
    }

    #[test]
    fn condition_malformed_is_error() {
        assert!(evaluate_condition("== =", &ctx(&[])).is_err());
    }

    #[test]
    fn condition_references_extracted() {
        let refs = condition_references("a == 'x' and b").unwrap();
        assert!(refs.contains("a"));
        assert!(refs.contains("b"));
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
