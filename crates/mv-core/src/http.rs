//! Shared `reqwest::Client`.
//!
//! A process-wide client built once and reused, so connection pools and TLS
//! state are shared across every outbound HTTP call (TRT-LLM health/preflight,
//! Ollama preflight, the `http_get` tool). Building a fresh client per request
//! — the pre-013 pattern — threw away the pool on every call, which is wasteful
//! in the CLI and a steady leak of connections in the long-running `mv-server`
//! daemon (fable finding F23).
//!
//! The shared client carries **no default timeout**: callers that need one
//! apply it per request with [`reqwest::RequestBuilder::timeout`], so the same
//! pooled client serves a 2-second preflight probe and a 30-second tool fetch
//! without either constraining the other. MCP servers keep their own clients
//! where per-server auth headers require a distinct configuration.

use std::sync::OnceLock;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// The process-wide shared HTTP client. Built on first use; every later call
/// returns the same instance (and therefore the same connection pool).
pub fn client() -> &'static reqwest::Client {
    // `reqwest::Client::new()` only fails if the TLS backend cannot initialize,
    // which is a process-level fault, not a per-call condition — falling back
    // to a second attempt keeps this infallible for callers.
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_is_a_shared_singleton() {
        // Same instance across calls — proves we pool rather than rebuild.
        let a = client();
        let b = client();
        assert!(std::ptr::eq(a, b), "shared client must be a singleton");
    }
}
