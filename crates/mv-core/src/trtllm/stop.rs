//! Stop-sequence helpers for the TRT-LLM provider.

use serde_json::{Value, json};

use crate::ModelEntry;

/// Provider-level default stop sequences applied when a TRT-LLM model entry
/// does not declare its own. Covers role terminators common to llama/qwen
/// chat-tuned checkpoints.
pub fn default_stop_sequences() -> Vec<String> {
    vec![
        "</s>".to_string(),
        "<|im_end|>".to_string(),
        "<|eot_id|>".to_string(),
    ]
}

/// Build the `{"stop": [...]}` JSON snippet for `additional_params` based on
/// the effective stop sequences for `entry`. Returns `None` when no stop
/// sequences apply (e.g. a non-trtllm entry with no explicit list).
pub fn request_stop_value(entry: &ModelEntry) -> Option<Value> {
    let seq = entry.effective_stop_sequences()?;
    if seq.is_empty() {
        return None;
    }
    Some(json!({ "stop": seq }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trtllm_entry(stop: Option<Vec<String>>) -> ModelEntry {
        ModelEntry {
            id: "llama-fp8".to_string(),
            provider: crate::Provider::Trtllm,
            locality: None,
            api_key_env: None,
            endpoint: None,
            default: false,
            served_name: None,
            architecture: None,
            quant: None,
            expected_vram_gb: None,
            stop_sequences: stop,
            max_turns: None,
        }
    }

    #[test]
    fn default_returned_when_entry_has_no_stop_sequences() {
        let entry = trtllm_entry(None);
        let value = request_stop_value(&entry).expect("default applied");
        assert_eq!(value, json!({ "stop": default_stop_sequences() }));
    }

    #[test]
    fn explicit_list_preserved_verbatim() {
        let entry = trtllm_entry(Some(vec!["<|eot_id|>".to_string()]));
        let value = request_stop_value(&entry).expect("explicit applied");
        assert_eq!(value, json!({ "stop": ["<|eot_id|>"] }));
    }

    #[test]
    fn request_stop_value_shape_exact() {
        let entry = trtllm_entry(Some(vec!["X".to_string(), "Y".to_string()]));
        let value = request_stop_value(&entry).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        let stop = obj.get("stop").unwrap().as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0], "X");
        assert_eq!(stop[1], "Y");
    }

    #[test]
    fn non_trtllm_entry_with_no_stop_returns_none() {
        let mut entry = trtllm_entry(None);
        entry.provider = crate::Provider::Ollama;
        assert!(request_stop_value(&entry).is_none());
    }
}
