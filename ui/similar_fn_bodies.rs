// Tests for the similar_fn_bodies lint.

// --- Should warn: near-identical, differ only in string literals ---

fn call_left(s: &str, n: usize) -> String {
    if s.is_empty() {
        return String::new();
    }
    let _name = "left";
    let chars: Vec<char> = s.chars().collect();
    let end = n.min(chars.len());
    chars[..end].iter().collect()
}

fn call_right(s: &str, n: usize) -> String {
    if s.is_empty() {
        return String::new();
    }
    let _name = "right";
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    chars[start..].iter().collect()
}

// --- Should NOT warn: completely different ---

fn unrelated(x: i32) -> bool {
    let a = x * 2;
    let b = a + 1;
    let c = b % 3;
    c == 0
}

fn main() {}
