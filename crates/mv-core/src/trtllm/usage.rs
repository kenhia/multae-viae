//! Token-usage helpers for the TRT-LLM proxy.
//!
//! TRT-LLM `/v1/chat/completions` and `/v1/completions` responses occasionally
//! emit token counts as JSON floats or strings instead of integers. The
//! [`Usage`] type here normalises those representations and provides a small
//! adapter for extracting counts from a `rig` response.

use serde::Deserialize;
use serde::de::{self, Deserializer};

use rig::completion::GetTokenUsage;

/// Normalised token-usage counts for a single completion or stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Deserialize)]
struct UsageRaw {
    #[serde(default, alias = "prompt_tokens", deserialize_with = "de_count")]
    input_tokens: u64,
    #[serde(default, alias = "completion_tokens", deserialize_with = "de_count")]
    output_tokens: u64,
}

impl<'de> Deserialize<'de> for Usage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UsageRaw::deserialize(deserializer)?;
        Ok(Usage {
            input_tokens: raw.input_tokens,
            output_tokens: raw.output_tokens,
        })
    }
}

/// Coerce integer, float, or numeric-string JSON values to `u64`.
fn de_count<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum AnyNum {
        U(u64),
        I(i64),
        F(f64),
        S(String),
    }

    match AnyNum::deserialize(deserializer)? {
        AnyNum::U(u) => Ok(u),
        AnyNum::I(i) => {
            if i < 0 {
                Err(de::Error::custom(format!("negative token count: {i}")))
            } else {
                Ok(i as u64)
            }
        }
        AnyNum::F(f) => {
            if !f.is_finite() || f < 0.0 {
                Err(de::Error::custom(format!("invalid token count: {f}")))
            } else {
                Ok(f.round() as u64)
            }
        }
        AnyNum::S(s) => s
            .trim()
            .parse::<u64>()
            .map_err(|e| de::Error::custom(format!("token count '{s}': {e}"))),
    }
}

impl Usage {
    /// Extract usage from any rig response that implements [`GetTokenUsage`].
    /// Returns `None` when the response did not report token usage.
    pub fn from_rig<R: GetTokenUsage + ?Sized>(response: &R) -> Option<Self> {
        let usage = response.token_usage()?;
        Some(Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        })
    }

    /// Build from raw counts (e.g. a rig `Usage` struct's fields).
    pub fn from_counts(input_tokens: u64, output_tokens: u64) -> Self {
        Usage {
            input_tokens,
            output_tokens,
        }
    }

    /// Record non-zero counts on `span` using the OpenTelemetry GenAI
    /// attribute names. The single recording path for buffered, streaming,
    /// and workflow completions — keep span-attribute naming here.
    pub fn record_on(&self, span: &tracing::Span) {
        if self.input_tokens != 0 || self.output_tokens != 0 {
            span.record("gen_ai.usage.input_tokens", self.input_tokens);
            span.record("gen_ai.usage.output_tokens", self.output_tokens);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialise_integer_counts() {
        let json = r#"{"input_tokens": 12, "output_tokens": 34}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.input_tokens, 12);
        assert_eq!(u.output_tokens, 34);
    }

    #[test]
    fn deserialise_float_counts() {
        let json = r#"{"input_tokens": 12.0, "output_tokens": 34.7}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.input_tokens, 12);
        assert_eq!(u.output_tokens, 35);
    }

    #[test]
    fn deserialise_string_counts() {
        let json = r#"{"input_tokens": "12", "output_tokens": "34"}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.input_tokens, 12);
        assert_eq!(u.output_tokens, 34);
    }

    #[test]
    fn deserialise_openai_alias_keys() {
        let json = r#"{"prompt_tokens": 5, "completion_tokens": 7}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.input_tokens, 5);
        assert_eq!(u.output_tokens, 7);
    }

    #[test]
    fn deserialise_missing_fields_defaults_to_zero() {
        let json = r#"{}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
    }

    struct NoUsage;
    impl GetTokenUsage for NoUsage {
        fn token_usage(&self) -> Option<rig::completion::Usage> {
            None
        }
    }

    struct WithUsage;
    impl GetTokenUsage for WithUsage {
        fn token_usage(&self) -> Option<rig::completion::Usage> {
            Some(rig::completion::Usage {
                input_tokens: 11,
                output_tokens: 22,
                total_tokens: 33,
                cached_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    #[test]
    fn from_rig_returns_none_when_missing() {
        assert!(Usage::from_rig(&NoUsage).is_none());
    }

    #[test]
    fn from_rig_extracts_counts() {
        let u = Usage::from_rig(&WithUsage).unwrap();
        assert_eq!(u.input_tokens, 11);
        assert_eq!(u.output_tokens, 22);
    }
}
