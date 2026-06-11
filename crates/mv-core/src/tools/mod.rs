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
