/// Generate a deterministic ID from string parts.
///
/// Matches the Python `_make_id` implementation exactly:
/// ```python
/// def _make_id(*parts: str) -> str:
///     combined = "_".join(p.strip("_.") for p in parts if p)
///     cleaned = re.sub(r"[^a-zA-Z0-9]+", "_", combined)
///     return cleaned.strip("_").lower()
/// ```
pub fn make_id(parts: &[&str]) -> String {
    let combined = parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.trim_matches(&['_', '.'][..]))
        .collect::<Vec<_>>()
        .join("_");

    let mut cleaned = String::with_capacity(combined.len());
    let mut prev_was_sep = false;
    for ch in combined.chars() {
        if ch.is_alphanumeric() {
            cleaned.push(ch);
            prev_was_sep = false;
        } else if !prev_was_sep {
            cleaned.push('_');
            prev_was_sep = true;
        }
    }

    cleaned.trim_matches('_').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_parts() {
        assert_eq!(make_id(&["Hello", "World"]), "hello_world");
    }

    #[test]
    fn strips_dots_and_underscores() {
        assert_eq!(make_id(&["__foo__", "..bar.."]), "foo_bar");
    }

    #[test]
    fn replaces_special_chars() {
        assert_eq!(make_id(&["my-class", "method()"]), "my_class_method");
    }

    #[test]
    fn filters_empty_parts() {
        assert_eq!(make_id(&["a", "", "b", ""]), "a_b");
    }

    #[test]
    fn single_part() {
        assert_eq!(make_id(&["SomeClass"]), "someclass");
    }

    #[test]
    fn all_empty() {
        assert_eq!(make_id(&["", ""]), "");
    }

    #[test]
    fn special_only() {
        assert_eq!(make_id(&["---"]), "");
    }

    #[test]
    fn mixed_unicode_and_ascii() {
        assert_eq!(make_id(&["foo::bar"]), "foo_bar");
    }

    #[test]
    fn consecutive_separators_collapsed() {
        assert_eq!(make_id(&["a!!!b"]), "a_b");
    }

    #[test]
    fn python_compat_complex() {
        assert_eq!(make_id(&["__init__", "MyClass"]), "init_myclass");
    }

    #[test]
    fn cjk_identifiers_preserved() {
        assert_eq!(make_id(&["类名"]), "类名");
        assert_eq!(make_id(&["関数", "Helper"]), "関数_helper");
        assert_eq!(make_id(&["모듈", "클래스"]), "모듈_클래스");
    }

    #[test]
    fn mixed_cjk_and_special_chars() {
        assert_eq!(make_id(&["类名::方法"]), "类名_方法");
        assert_eq!(make_id(&["my-类"]), "my_类");
    }
}
