use std::collections::HashMap;

/// Returns true if `body` contains a `<@uuid>` tag matching the given UUID.
pub fn mentions_user(body: &str, uuid: &str) -> bool {
    let needle = format!("<@{}>", uuid);
    body.contains(&needle)
}

/// Replace all `<@uuid>name</@>` tags in `body` with `@resolved_name`,
/// resolving UUIDs via `name_cache` and falling back to the embedded name.
pub fn render_body(body: &str, name_cache: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(body.len());
    let mut rest = body;

    while let Some(open_start) = rest.find("<@") {
        result.push_str(&rest[..open_start]);
        let after_open = &rest[open_start + 2..];

        // Find the closing '>' of the opening tag.
        if let Some(close_gt) = after_open.find('>') {
            let uuid_candidate = &after_open[..close_gt];
            if is_uuid_like(uuid_candidate) {
                let after_tag = &after_open[close_gt + 1..];
                if let Some(close_start) = after_tag.find("</@>") {
                    let embedded_name = &after_tag[..close_start];
                    let resolved = name_cache
                        .get(uuid_candidate)
                        .map(|s| s.as_str())
                        .unwrap_or(embedded_name);
                    result.push('@');
                    result.push_str(resolved);
                    rest = &after_tag[close_start + 4..];
                    continue;
                }
            }
        }

        // Not a valid mention tag — emit `<@` literally and advance past it.
        result.push_str("<@");
        rest = after_open; // = &rest[open_start + 2..]
    }

    result.push_str(rest);
    result
}

/// Extract the list of screennames mentioned in the raw wire-format `body`
/// (i.e. before tag-stripping), resolving via `name_cache`.
pub fn extract_mentioned_names(body: &str, name_cache: &HashMap<String, String>) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = body;
    while let Some(open_start) = rest.find("<@") {
        let after_open = &rest[open_start + 2..];
        if let Some(close_gt) = after_open.find('>') {
            let uuid_candidate = &after_open[..close_gt];
            if is_uuid_like(uuid_candidate) {
                let after_tag = &after_open[close_gt + 1..];
                if let Some(close_start) = after_tag.find("</@>") {
                    let embedded_name = &after_tag[..close_start];
                    let resolved = name_cache
                        .get(uuid_candidate)
                        .map(|s| s.as_str())
                        .unwrap_or(embedded_name);
                    if !resolved.is_empty() {
                        names.push(resolved.to_string());
                    }
                    rest = &after_tag[close_start + 4..];
                    continue;
                }
            }
        }
        rest = after_open; // skip past the invalid "<@"
    }
    names
}

/// Split a rendered body line into `(text, is_mention)` pairs so the caller
/// can apply different styles to `@name` tokens.  Matches are case-insensitive.
/// Returns a single no-mention pair if `mentioned_names` is empty.
pub fn split_body_spans(line: &str, mentioned_names: &[String]) -> Vec<(String, bool)> {
    if mentioned_names.is_empty() {
        return vec![(line.to_string(), false)];
    }

    let mut result: Vec<(String, bool)> = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut segment_start = 0;

    while i < chars.len() {
        if chars[i] == '@' {
            // Try each known name; prefer longest match.
            let tail: String = chars[i + 1..].iter().collect();
            let matched = mentioned_names
                .iter()
                .filter(|n| tail.to_lowercase().starts_with(&n.to_lowercase()))
                .max_by_key(|n| n.len());
            if let Some(name) = matched {
                let word_end = i + 1 + name.chars().count();
                // Push the plain text before this mention.
                if i > segment_start {
                    result.push((chars[segment_start..i].iter().collect(), false));
                }
                // Push the @name as a mention span.
                result.push((chars[i..word_end].iter().collect(), true));
                segment_start = word_end;
                i = word_end;
                continue;
            }
        }
        i += 1;
    }

    // Remaining plain text.
    if segment_start < chars.len() {
        result.push((chars[segment_start..].iter().collect(), false));
    }

    if result.is_empty() {
        result.push((line.to_string(), false));
    }
    result
}

fn is_uuid_like(s: &str) -> bool {
    if s.len() != 36 {
        return false;
    }
    for (i, b) in s.bytes().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return false;
                }
            }
            _ => {
                if !b.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mentions_user() {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let body = format!("hey <@{}>Alice</@> how are you", uuid);
        assert!(mentions_user(&body, uuid));
        assert!(!mentions_user(
            &body,
            "00000000-0000-0000-0000-000000000000"
        ));
    }

    #[test]
    fn test_render_body_resolves_name() {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let mut cache = HashMap::new();
        cache.insert(uuid.to_string(), "Alice".to_string());
        let body = format!("hey <@{}>fallback</@> how are you", uuid);
        let rendered = render_body(&body, &cache);
        assert_eq!(rendered, "hey @Alice how are you");
    }

    #[test]
    fn test_render_body_fallback_name() {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let body = format!("hey <@{}>Alice</@>!", uuid);
        let rendered = render_body(&body, &HashMap::new());
        assert_eq!(rendered, "hey @Alice!");
    }

    #[test]
    fn test_render_body_passthrough() {
        let body = "no mentions here <@notauuid> or </@> this";
        let rendered = render_body(body, &HashMap::new());
        assert_eq!(rendered, body);
    }

    #[test]
    fn test_split_body_spans_slash_in_name() {
        let names = vec!["foo/bar".to_string()];
        let spans = split_body_spans("hey @foo/bar!", &names);
        assert_eq!(
            spans,
            vec![
                ("hey ".to_string(), false),
                ("@foo/bar".to_string(), true),
                ("!".to_string(), false),
            ]
        );
    }

    #[test]
    fn test_is_uuid_like() {
        assert!(is_uuid_like("a1b2c3d4-e5f6-7890-abcd-ef1234567890"));
        assert!(!is_uuid_like("not-a-uuid"));
        assert!(!is_uuid_like("a1b2c3d4-e5f6-7890-abcd-ef123456789")); // 35 chars
        assert!(!is_uuid_like("a1b2c3d4-e5f6-7890-abcd-ef12345678901")); // 37 chars
    }
}
