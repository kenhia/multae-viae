//! Transform-step operations. The single source for the operation list:
//! `validate` checks against [`KNOWN_TRANSFORMS`], the engine dispatches in
//! [`execute_transform`] — adding an operation touches only this file.

use crate::MvError;

/// All transform operations the engine implements.
pub const KNOWN_TRANSFORMS: &[&str] = &["extract_json"];

/// Execute a transform step, returning the structured result. As of sprint
/// 012 the result is a typed [`serde_json::Value`] (no re-stringify), so a
/// later step can reach into its fields (`{{out.title}}`, `out.score >= 8`).
pub fn execute_transform(
    step_id: &str,
    operation: &str,
    input: &str,
    schema: Option<&serde_json::Value>,
) -> Result<serde_json::Value, MvError> {
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
) -> Result<serde_json::Value, MvError> {
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

    Ok(parsed)
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
