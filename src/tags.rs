pub fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let rest = &text[start..];

    let mut cursor = 0;
    let mut depth = 1usize;
    while cursor < rest.len() {
        let next_open = rest[cursor..].find(&open).map(|index| cursor + index);
        let next_close = rest[cursor..].find(&close).map(|index| cursor + index);

        match (next_open, next_close) {
            (Some(open_index), Some(close_index)) if open_index < close_index => {
                depth += 1;
                cursor = open_index + open.len();
            }
            (_, Some(close_index)) => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..close_index].trim().to_string());
                }
                cursor = close_index + close.len();
            }
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_tag() {
        assert_eq!(
            extract_tag("<command-name>/code-review</command-name>", "command-name").as_deref(),
            Some("/code-review")
        );
    }

    #[test]
    fn balances_same_name_nested_tags() {
        assert_eq!(
            extract_tag(
                "<command-name>/foo<command-name>nested</command-name>bar</command-name>",
                "command-name"
            )
            .as_deref(),
            Some("/foo<command-name>nested</command-name>bar")
        );
    }
}
