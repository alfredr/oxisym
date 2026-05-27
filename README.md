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

## Algorithm

All three lints share a single foundation: a **shape hash** (`src/shape_hash.rs`) that walks the HIR and hashes structural skeletons while ignoring literal values and identifier names. So `paint("left", 0)` and `paint("right", 255)` collapse to the same hash.

### Shape hashing: strip literals and names

```text
Source:  paint("left", 0)              Source:  paint("right", 255)

   Call                                    Call
   / \                                     / \
 Path  Args                              Path  Args
("paint")|                             ("paint") |
        / \                                     / \
      Lit  Lit                                Lit  Lit
   ("left") (0)                            ("right") (255)

Shape hash walks the tree, hashing only the discriminants:
   Call -> hash(Path-disc) + len(2) + hash(Lit-disc) + hash(Lit-disc)
   (literal values and the strings "left"/"right" are NOT fed in)

Both trees -> 0xA1B2  (same hash)
```

Identifier names *are* hashed for method calls (so `x.set_property` is distinguished from `x.remove_property`), but bare paths and literals are reduced to their kind only.

### `same_call_both_branches` (local, within one `if`/`match`)

```text
  fn warn(dark: bool) -> i32 {
      if dark { paint(0) } else { paint(255) }
  }

          If
         / | \
     Cond  Then  Else
      |     |     |
    Path  Call   Call
   (dark) / \   / \
        Path Lit Path Lit
       (paint)(0)(paint)(255)
                 ^^^^^^^^^^^
                 same callee Path "paint"

  -> identify differing arg positions:
     position 0: Lit(0) vs Lit(255)   <-- differs
  -> hoist: paint(if dark { 0 } else { 255 })
```

Callees are compared by **HIR path equality**, not by shape hash -- we don't want to hoist when one branch calls `foo` and another `bar` just because both are 1-arg calls.

### `similar_fn_bodies` (cross-function LCS over statement hashes)

```text
  fn call_left(s, n)              fn call_right(s, n)
  +-------------------+           +--------------------+
  | if s.is_empty()   | h1        | if s.is_empty()    | h1
  | let _ = "left"    | h2        | let _ = "right"    | h2 (same shape!)
  | let chars = ...   | h3        | let chars = ...    | h3
  | let end = n.min() | h4        | let start = .sat_sub() | h5
  | chars[..end]...   | h6        | chars[start..]...  | h7 (similar)
  +-------------------+           +--------------------+

  Hash vectors:
     A = [h1, h2, h3, h4, h6]
     B = [h1, h2, h3, h5, h7]

  LCS alignment:
       A:  h1  h2  h3  h4  h6
            |   |   |   x   x
       B:  h1  h2  h3  h5  h7

     LCS length = 3
     similarity = 2 * 3 / (5 + 5) = 0.60   -> above threshold

  -> warn: 60% structural similarity; suggest helper `call_`
     derived from the common prefix "call_" + differing suffixes
```

### `similar_match_arms` (two complementary passes per `match`)

**Pass A -- exact-shape groups** (whole-body shape hash, group identical):

```text
  match kind {
      A(x) => { hash(x); push(x); }   shape -> g1
      B(y) => { hash(y); push(y); }   shape -> g1  (same!)
      C(z) => { hash(z); push(z); }   shape -> g1
      D    => { return; }             shape -> g2
  }

  Groups: { g1: [A, B, C],  g2: [D] }
  g1 has 3 arms >= MIN_SIMILAR_ARMS  -> warn, suggest collapse
```

**Pass B -- pairwise LCS + leaf-diff** (for arms not already in an exact group):

```text
  Arm A body stmts: [h1, h2, h3]
  Arm B body stmts: [h1, h2, h4]

  LCS = 2,  similarity = 4 / 6 = 0.67

  Then walk both ASTs with diff_expr() to count differing LEAVES:

     stmt h3:    foo(x, 1)
     stmt h4:    foo(x, 2)
                       ^
                  1 differing leaf

     -> suggestion text: "pass the varying value as a parameter"
        (vs "pass a closure for the varying behavior" if many leaves differ)
```

The two passes are complementary: Pass A catches groups of >= 3 perfectly-shaped arms cheaply; Pass B catches near-misses by structure and uses the leaf-count to decide whether the difference is "one literal" (parameter) or "whole sub-expression" (closure).

## Tests

```sh
cargo test
```
