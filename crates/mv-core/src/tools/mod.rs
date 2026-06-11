pub mod file_list;
pub mod file_read;
pub mod http_get;
pub mod shell_exec;

use std::sync::OnceLock;

/// Execution policy for built-in tools — the Phase 7 sandboxing seam.
///
/// Default-allow today. Phase 7 lands as deny rules (path roots, command
/// allowlists, URL filters) on this type without touching tool signatures:
/// the rig tool macro generates unit structs from free functions, so a
/// constructor-injected policy would mean abandoning the macro — a
/// process-global, set once at startup, is the seam instead.
#[derive(Debug, Default)]
pub struct ToolPolicy {}

impl ToolPolicy {
    /// May the tool read/list this filesystem path?
    pub fn check_path(&self, _path: &str) -> Result<(), String> {
        Ok(())
    }

    /// May the tool execute this shell command?
    pub fn check_command(&self, _command: &str) -> Result<(), String> {
        Ok(())
    }

    /// May the tool fetch this URL?
    pub fn check_url(&self, _url: &str) -> Result<(), String> {
        Ok(())
    }
}

static TOOL_POLICY: OnceLock<ToolPolicy> = OnceLock::new();

/// Install a process-wide tool policy before any tool runs. First call wins;
/// returns the rejected policy if one was already installed.
pub fn set_tool_policy(policy: ToolPolicy) -> Result<(), ToolPolicy> {
    TOOL_POLICY.set(policy)
}

/// The active tool policy (default-allow when none was installed).
pub fn tool_policy() -> &'static ToolPolicy {
    TOOL_POLICY.get_or_init(ToolPolicy::default)
}

/// Maximum characters in tool output before truncation.
pub const MAX_TOOL_OUTPUT_CHARS: usize = 10_000;

/// Shell command execution timeout in seconds.
pub const SHELL_TIMEOUT_SECS: u64 = 30;

/// HTTP request timeout in seconds.
pub const HTTP_TIMEOUT_SECS: u64 = 30;

/// Truncate a string to at most `max_chars` bytes, appending a notice if
/// truncated. The cut point is walked back to a UTF-8 char boundary so the
/// result is always valid (slicing at an arbitrary byte index panics).
pub fn truncate_output(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let mut end = max_chars;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = &s[..end];
        format!("{truncated}\n...[truncated at {max_chars} chars]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string_unchanged() {
        let input = "hello";
        assert_eq!(truncate_output(input, 100), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        let input = "abcde";
        assert_eq!(truncate_output(input, 5), "abcde");
    }

    #[test]
    fn truncate_long_string() {
        let input = "abcdefghij";
        let result = truncate_output(input, 5);
        assert!(result.starts_with("abcde"));
        assert!(result.contains("[truncated at 5 chars]"));
    }

    #[test]
    fn truncate_multibyte_boundary_does_not_panic() {
        // 'é' is 2 bytes; limit 5 falls mid-char after "abcd" + first byte of 'é'.
        let input = "abcdéfgh";
        let result = truncate_output(input, 5);
        assert!(result.starts_with("abcd"));
        assert!(!result.starts_with("abcdé"));
        assert!(result.contains("[truncated at 5 chars]"));
    }

    #[test]
    fn truncate_all_multibyte_input() {
        // Each '日' is 3 bytes; limit 7 lands inside the third char.
        let input = "日日日日";
        let result = truncate_output(input, 7);
        assert!(result.starts_with("日日"));
        assert!(result.contains("[truncated at 7 chars]"));
    }
}
