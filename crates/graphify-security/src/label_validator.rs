//! Label sanitization to prevent injection in HTML/GraphML output.

/// Maximum label length after sanitization.
const MAX_LABEL_LEN: usize = 256;

/// Sanitize a label for safe use in HTML/GraphML output.
///
/// - Strips control characters
/// - Truncates to 256 characters
/// - HTML-escapes `&`, `<`, `>`, `"`, and `'`
pub fn sanitize_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_LABEL_LEN)
        .collect();

    cleaned
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_unchanged() {
        assert_eq!(sanitize_label("Hello World"), "Hello World");
    }

    #[test]
    fn test_html_entities_escaped() {
        assert_eq!(
            sanitize_label("<script>alert(\"xss\")</script>"),
            "&lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;"
        );
    }

    #[test]
    fn test_ampersand_escaped() {
        assert_eq!(sanitize_label("A & B"), "A &amp; B");
    }

    #[test]
    fn test_single_quote_escaped() {
        assert_eq!(sanitize_label("it's"), "it&#x27;s");
    }

    #[test]
    fn test_control_chars_stripped() {
        assert_eq!(sanitize_label("hello\x00world\x07"), "helloworld");
    }

    #[test]
    fn test_newlines_stripped() {
        assert_eq!(sanitize_label("line1\nline2\r\n"), "line1line2");
    }

    #[test]
    fn test_tabs_stripped() {
        assert_eq!(sanitize_label("col1\tcol2"), "col1col2");
    }

    #[test]
    fn test_truncation() {
        let long = "a".repeat(300);
        let result = sanitize_label(&long);
        // After sanitization no escaping needed, so length == 256
        assert_eq!(result.len(), 256);
    }

    #[test]
    fn test_truncation_with_entities() {
        // 256 '<' characters → each becomes "&lt;" (4 bytes)
        let input = "<".repeat(300);
        let result = sanitize_label(&input);
        // Only 256 '<' taken, each → "&lt;" = 4 chars
        assert_eq!(result.len(), 256 * 4);
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(sanitize_label(""), "");
    }

    #[test]
    fn test_unicode_preserved() {
        assert_eq!(sanitize_label("你好世界"), "你好世界");
    }

    #[test]
    fn test_mixed_content() {
        assert_eq!(
            sanitize_label("Node <A> & \"B\""),
            "Node &lt;A&gt; &amp; &quot;B&quot;"
        );
    }

    #[test]
    fn test_backtick_and_braces() {
        // Backticks and braces pass through (not HTML-special)
        assert_eq!(sanitize_label("`{code}`"), "`{code}`");
    }
}
