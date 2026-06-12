use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::MvError;

/// Render a template string with the given context variables.
///
/// The context holds typed [`Value`]s (sprint 012), so a template may reach
/// into structure — `{{report.title}}` — not just substitute scalars. A
/// `Value::String` renders raw (no quotes); containers render via minijinja's
/// native value formatting.
pub fn render_template(
    template: &str,
    context: &HashMap<String, Value>,
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

/// Evaluate a `branch`/`loop` condition — a minijinja expression — against the
/// context, returning its truthiness. The same template language as `{{…}}`,
/// so `style == 'detailed'` works. With the typed context (sprint 012) numeric
/// and structured comparisons evaluate correctly — `result.score >= 8` is a
/// real numeric comparison, not a string compare — so the old `"false"`-is-
/// truthy trap no longer applies to values produced as JSON (e.g. by
/// `extract_json` or a nested workflow). A bare string variable is still truthy
/// when non-empty.
pub fn evaluate_condition(
    condition: &str,
    context: &HashMap<String, Value>,
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

    fn ctx(pairs: &[(&str, &str)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String((*v).to_string())))
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
        context.insert(
            "topic".to_string(),
            Value::String("from_output".to_string()),
        );
        let result = render_template("{{topic}}", &context).unwrap();
        assert_eq!(result, "from_output");
    }

    // --- 012/WS2: typed context — field access, numeric conditions, rendering ---

    #[test]
    fn template_reaches_into_structured_value() {
        let mut context = HashMap::new();
        context.insert(
            "report".to_string(),
            serde_json::json!({"title": "Findings", "score": 9}),
        );
        let result =
            render_template("Title: {{report.title}} ({{report.score}})", &context).unwrap();
        assert_eq!(result, "Title: Findings (9)");
    }

    #[test]
    fn condition_compares_numbers_typed() {
        let mut context = HashMap::new();
        context.insert("result".to_string(), serde_json::json!({"score": 9}));
        assert!(evaluate_condition("result.score >= 8", &context).unwrap());
        assert!(!evaluate_condition("result.score < 5", &context).unwrap());
    }

    #[test]
    fn json_bool_false_is_falsy_unlike_a_string() {
        // The retired trap: a real JSON false is falsy (a *string* "false" would
        // be truthy — that was the pre-012 caveat).
        let mut context = HashMap::new();
        context.insert("flag".to_string(), Value::Bool(false));
        assert!(!evaluate_condition("flag", &context).unwrap());
        context.insert("flag".to_string(), Value::String("false".to_string()));
        assert!(evaluate_condition("flag", &context).unwrap());
    }

    #[test]
    fn string_value_interpolates_raw() {
        // A string value renders without quotes (back-compat with pre-012).
        let result = render_template("{{x}}", &ctx(&[("x", "plain")])).unwrap();
        assert_eq!(result, "plain");
    }

    #[test]
    fn container_value_interpolation_is_pinned() {
        // Whole-container interpolation: this test IS the spec for the behavior
        // documented in docs/06. minijinja renders a sequence in Python-ish
        // debug form; authors should reach into fields rather than dump
        // containers into prompts.
        let mut context = HashMap::new();
        context.insert("items".to_string(), serde_json::json!(["a", "b"]));
        let result = render_template("{{items}}", &context).unwrap();
        assert_eq!(result, "[\"a\", \"b\"]");
    }
}
