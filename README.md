# oxisym

[Dylint](https://github.com/trailofbits/dylint) lints for detecting structural duplication in Rust code.

## Lints

| Lint | What it detects |
|------|----------------|
| `same_call_both_branches` | `if`/`match` where every branch calls the same function. Can be refactored to call once with a conditional argument. |
| `similar_fn_bodies` | Function pairs in the same module with high structural similarity. Candidates for extraction into a shared helper. |
| `similar_match_arms` | Match arms with near-identical bodies. Common logic can be collapsed or extracted. |

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

`same_call_both_branches`: the call can be hoisted out of the branch.

```rust
fn warn(dark: bool) -> i32 {
    if dark { paint(0) } else { paint(255) }
    // suggestion: paint(if dark { 0 } else { 255 })
}
```

`similar_fn_bodies`: two functions that differ only in literals.

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

All three lints share a single foundation: a shape hash (`src/shape_hash.rs`) that walks the HIR and hashes structural skeletons while ignoring literal values and identifier names. So `paint("left", 0)` and `paint("right", 255)` collapse to the same hash.

### Shape hashing: strip literals and names

Two source expressions:

```rust
paint("left", 0)
paint("right", 255)
```

Their HIR trees:

```text
   Call                                Call
   / \                                 / \
 Path  Args                          Path  Args
("paint")|                         ("paint") |
        / \                                 / \
      Lit  Lit                            Lit  Lit
   ("left") (0)                        ("right") (255)

Shape hash walks the tree, hashing only the discriminants:
   Call -> hash(Path-disc) + len(2) + hash(Lit-disc) + hash(Lit-disc)
   (literal values and the strings "left"/"right" are NOT fed in)

Both trees -> 0xA1B2  (same hash)
```

Identifier names *are* hashed for method calls (so `x.set_property` is distinguished from `x.remove_property`), but bare paths and literals are reduced to their kind only.

### `same_call_both_branches` (local, within one `if`/`match`)

Source:

```rust
fn warn(dark: bool) -> i32 {
    if dark { paint(0) } else { paint(255) }
}
```

HIR:

```text
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

Callees are compared by HIR path equality, not by shape hash. We don't want to hoist when one branch calls `foo` and another `bar` just because both are 1-arg calls.

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

  -> warn: 60% structural similarity, suggest helper `call_`
     derived from the common prefix "call_" + differing suffixes
```

### `similar_match_arms` (two complementary passes per `match`)

Pass A (exact-shape groups): whole-body shape hash, group identical.

```rust
match kind {
    A(x) => { hash(x); push(x); }
    B(y) => { hash(y); push(y); }
    C(z) => { hash(z); push(z); }
    D    => { return; }
}
```

```text
  Shapes:
     A(x) -> g1
     B(y) -> g1   (same!)
     C(z) -> g1
     D    -> g2

  Groups: { g1: [A, B, C],  g2: [D] }
  g1 has 3 arms >= MIN_SIMILAR_ARMS  -> warn, suggest collapse
```

Pass B (pairwise LCS + leaf-diff): for arms not already in an exact group.

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

The two passes are complementary. Pass A catches groups of >= 3 perfectly-shaped arms cheaply. Pass B catches near-misses by structure, and uses the leaf-count to decide whether the difference is one literal (parameter) or a whole sub-expression (closure).

## Real-world examples

The following cases came from running oxisym on a working Rust codebase.

### Three build-glue functions at 100% structural similarity

oxisym output:

```text
warning: `build_shared` and `build_host` have 100% structural similarity
warning: `build_shared` and `test_shared` have 100% structural similarity
warning: `build_host` and `test_shared` have 100% structural similarity
```

Before:

```rust
fn build_shared(sh: &Shell) -> Result<()> {
    build(sh)?;
    let _g = sh.push_dir("pkg/foo");
    cmd!(sh, "swift build").run()?;
    Ok(())
}

fn build_host(sh: &Shell) -> Result<()> {
    build(sh)?;
    let _g = sh.push_dir("pkg/bar");
    cmd!(sh, "swift build").run()?;
    Ok(())
}

fn test_shared(sh: &Shell) -> Result<()> {
    build(sh)?;
    let _g = sh.push_dir("pkg/foo");
    cmd!(sh, "swift test").run()?;
    Ok(())
}
```

After:

```rust
fn swift_in(sh: &Shell, package_dir: &str, subcmd: &str) -> Result<()> {
    build(sh)?;
    let _g = sh.push_dir(package_dir);
    cmd!(sh, "swift {subcmd}").run()?;
    Ok(())
}

fn build_shared(sh: &Shell) -> Result<()> { swift_in(sh, "pkg/foo", "build") }
fn build_host(sh: &Shell)   -> Result<()> { swift_in(sh, "pkg/bar", "build") }
fn test_shared(sh: &Shell)  -> Result<()> { swift_in(sh, "pkg/foo", "test")  }
```

### Match arms with a shared `unsafe` preamble (100%)

oxisym flagged two arms of a `match` over a value-kind tag as 100% identical in shape: both wrote the same 8-byte little-endian copy boilerplate.

Before:

```rust
let value = unsafe {
    match value_kind {
        0 => {
            let bytes = std::slice::from_raw_parts(value_ptr, value_len);
            AttrValue::Str(String::from_utf8_lossy(bytes).into_owned())
        }
        1 if value_len == 8 => {
            let mut b = [0u8; 8];
            std::ptr::copy_nonoverlapping(value_ptr, b.as_mut_ptr(), 8);
            AttrValue::I64(i64::from_le_bytes(b))
        }
        2 if value_len == 8 => {
            let mut b = [0u8; 8];
            std::ptr::copy_nonoverlapping(value_ptr, b.as_mut_ptr(), 8);
            AttrValue::F64(f64::from_le_bytes(b))
        }
        3 if value_len == 1 => AttrValue::Bool(*value_ptr != 0),
        _ => return,
    }
};
```

After:

```rust
let value = unsafe {
    let read_le8 = |ptr: *const u8| -> [u8; 8] {
        let mut b = [0u8; 8];
        std::ptr::copy_nonoverlapping(ptr, b.as_mut_ptr(), 8);
        b
    };
    match value_kind {
        0 => {
            let bytes = std::slice::from_raw_parts(value_ptr, value_len);
            AttrValue::Str(String::from_utf8_lossy(bytes).into_owned())
        }
        1 if value_len == 8 => AttrValue::I64(i64::from_le_bytes(read_le8(value_ptr))),
        2 if value_len == 8 => AttrValue::F64(f64::from_le_bytes(read_le8(value_ptr))),
        3 if value_len == 1 => AttrValue::Bool(*value_ptr != 0),
        _ => return,
    }
};
```

## Tests

```sh
cargo test
```
