# oxisym

[Dylint](https://github.com/trailofbits/dylint) lints for detecting structural duplication in Rust code.

## Lints

| Lint | What it detects |
|------|----------------|
| `same_call_both_branches` | `if`/`match` where every branch calls the same function — can be refactored to call once with a conditional argument |
| `similar_fn_bodies` | Function pairs in the same module with high structural similarity — candidates for extraction into a shared helper |
| `similar_match_arms` | Match arms with near-identical bodies — common logic can be collapsed or extracted |

All three use **shape hashing**, which ignores literal values and identifier names so that functions differing only in string/numeric constants are correctly flagged.

## Setup

```sh
cargo install cargo-dylint dylint-link
```

## Usage

Add oxisym to your project's `Cargo.toml`:

```toml
[workspace.metadata.dylint]
libraries = [
    { git = "https://github.com/alfredr/oxisym" },
]
```

Then run:

```sh
cargo dylint --all
```

Warnings appear inline just like clippy, with suggested helper names and refactoring transforms.

## Examples

**`same_call_both_branches`** — the call can be hoisted out of the branch:

```rust
fn warn(dark: bool) -> i32 {
    if dark { paint(0) } else { paint(255) }
    // suggestion: paint(if dark { 0 } else { 255 })
}
```

**`similar_fn_bodies`** — two functions that differ only in literals:

```rust
fn call_left(s: &str, n: usize) -> String {
    if s.is_empty() { return String::new(); }
    let _name = "left";
    let chars: Vec<char> = s.chars().collect();
    let end = n.min(chars.len());
    chars[..end].iter().collect()
}

fn call_right(s: &str, n: usize) -> String {
    if s.is_empty() { return String::new(); }
    let _name = "right";
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    chars[start..].iter().collect()
}
// warning: `call_left` and `call_right` have 83% structural similarity
// help: extract into `call_` parameterized on the differing constants
```

For one-off runs without modifying `Cargo.toml`, use `DYLINT_LIBRARY_PATH`:

```sh
DYLINT_LIBRARY_PATH=/path/to/oxisym/target/release cargo dylint --all --workspace
```

## Tests

```sh
cargo test
```
