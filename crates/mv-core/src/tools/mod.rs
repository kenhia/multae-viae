pub mod file_list;
pub mod file_read;
pub mod http_get;
pub mod shell_exec;

/// Maximum characters in tool output before truncation.
pub const MAX_TOOL_OUTPUT_CHARS: usize = 10_000;

/// Shell command execution timeout in seconds.
pub const SHELL_TIMEOUT_SECS: u64 = 30;

/// HTTP request timeout in seconds.
pub const HTTP_TIMEOUT_SECS: u64 = 30;

/// Truncate a string to `max_chars`, appending a notice if truncated.
pub fn truncate_output(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let truncated = &s[..max_chars];
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
}
