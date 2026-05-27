//! Utilities for suggesting helper function names from sets of similar names.

/// Longest common prefix of two strings, split on `_` boundaries.
/// Returns the prefix *including* the trailing `_` if one exists.
///
/// ```text
/// common_prefix("call_left", "call_right") => "call_"
/// common_prefix("eval_all_predicate", "eval_any_predicate") => "eval_"
/// common_prefix("foo", "bar") => ""
/// ```
pub fn common_prefix<'a>(a: &'a str, b: &str) -> &'a str {
    let mut last_boundary = 0; // position after last shared `_`
    for (i, (ca, cb)) in a.bytes().zip(b.bytes()).enumerate() {
        if ca != cb {
            break;
        }
        if ca == b'_' {
            last_boundary = i + 1;
        }
    }
    // If the entire shorter string is a prefix and it ends at a boundary
    let min_len = a.len().min(b.len());
    if a.as_bytes()[..min_len] == b.as_bytes()[..min_len] && min_len > 0 {
        // Whole shorter string matches — check if it ends with `_`
        if a.as_bytes().get(min_len - 1) == Some(&b'_') {
            last_boundary = min_len;
        }
    }
    &a[..last_boundary]
}

/// Longest common suffix of two strings, split on `_` boundaries.
/// Returns the suffix *including* the leading `_`.
///
/// ```text
/// common_suffix("eval_all_predicate", "eval_any_predicate") => "_predicate"
/// common_suffix("call_left", "call_right") => ""
/// ```
pub fn common_suffix<'a>(a: &'a str, b: &str) -> &'a str {
    let mut last_boundary = a.len(); // position of last shared `_` from the end
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    let mut ai = a.len();
    let mut bi = b.len();
    while ai > 0 && bi > 0 {
        ai -= 1;
        bi -= 1;
        if a_bytes[ai] != b_bytes[bi] {
            break;
        }
        if a_bytes[ai] == b'_' {
            last_boundary = ai;
        }
    }
    // If we exhausted one string entirely and the match point is at a boundary
    if ai == 0 && bi == 0 && a_bytes[0] == b_bytes[0] && a_bytes[0] == b'_' {
        last_boundary = 0;
    }
    &a[last_boundary..]
}

/// Suggest a helper function name from a set of similar function names.
///
/// Uses longest common prefix and suffix on `_` boundaries.
/// Examples:
/// - `["call_left", "call_right"]` → `"call_op"`
/// - `["eval_all_predicate", "eval_any_predicate", "eval_none_predicate"]` → `"eval_predicate"`
/// - `["eval_and", "eval_or"]` → `"eval_op"`
/// - `["foo", "bar"]` → `"shared_helper"`
pub fn suggest_helper_name(names: &[&str]) -> String {
    if names.len() < 2 {
        return "shared_helper".into();
    }

    // Compute common prefix across all names.
    let mut prefix = common_prefix(names[0], names[1]);
    for name in &names[2..] {
        prefix = common_prefix(prefix, name);
    }

    // Compute common suffix across all names.
    let mut suffix = common_suffix(names[0], names[1]);
    for name in &names[2..] {
        let s = common_suffix(suffix, name);
        // s is a suffix of `suffix`, which is a suffix of names[0].
        // Find it in names[0].
        suffix = &names[0][names[0].len() - s.len()..];
    }

    // Strip leading `_` from suffix for combining.
    let suffix_trimmed = suffix.strip_prefix('_').unwrap_or(suffix);

    if !prefix.is_empty() && !suffix_trimmed.is_empty() {
        // Combine: "eval_" + "predicate" → "eval_predicate"
        format!("{prefix}{suffix_trimmed}")
    } else if !prefix.is_empty() {
        // Prefix only: "call_" → "call_op"
        let base = prefix.strip_suffix('_').unwrap_or(prefix);
        format!("{base}_op")
    } else if !suffix_trimmed.is_empty() {
        // Suffix only (rare): "_handler" → "shared_handler"
        format!("shared_{suffix_trimmed}")
    } else {
        "shared_helper".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_prefix() {
        assert_eq!(common_prefix("call_left", "call_right"), "call_");
        assert_eq!(
            common_prefix("eval_all_predicate", "eval_any_predicate"),
            "eval_"
        );
        assert_eq!(common_prefix("foo", "bar"), "");
        assert_eq!(common_prefix("eval_and", "eval_or"), "eval_");
        assert_eq!(
            common_prefix("set_node_properties", "set_rel_properties"),
            "set_"
        );
    }

    #[test]
    fn test_common_suffix() {
        assert_eq!(
            common_suffix("eval_all_predicate", "eval_any_predicate"),
            "_predicate"
        );
        assert_eq!(common_suffix("call_left", "call_right"), "");
        assert_eq!(
            common_suffix("set_node_properties", "set_rel_properties"),
            "_properties"
        );
    }

    #[test]
    fn test_suggest_helper_name() {
        assert_eq!(suggest_helper_name(&["call_left", "call_right"]), "call_op");
        assert_eq!(
            suggest_helper_name(&[
                "eval_all_predicate",
                "eval_any_predicate",
                "eval_none_predicate"
            ]),
            "eval_predicate"
        );
        assert_eq!(suggest_helper_name(&["eval_and", "eval_or"]), "eval_op");
        assert_eq!(
            suggest_helper_name(&["set_node_properties", "set_rel_properties"]),
            "set_properties"
        );
        assert_eq!(suggest_helper_name(&["foo", "bar"]), "shared_helper");
    }
}
