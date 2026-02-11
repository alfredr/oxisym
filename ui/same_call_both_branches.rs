// Tests for the same_call_both_branches lint.

fn paint(c: i32) -> i32 {
    c
}

fn other(c: i32) -> i32 {
    c + 1
}

struct Canvas;
impl Canvas {
    fn draw(&self, x: i32) -> i32 {
        x
    }
}

// --- Should warn ---

fn warn_if_else(dark: bool) -> i32 {
    if dark { paint(0) } else { paint(255) }
}

fn warn_match(x: bool) -> i32 {
    match x {
        true => paint(1),
        false => paint(2),
    }
}

fn warn_method(x: bool) -> i32 {
    let c = Canvas;
    match x {
        true => c.draw(10),
        false => c.draw(20),
    }
}

// --- Should NOT warn ---

fn ok_different_fns(dark: bool) -> i32 {
    if dark { paint(0) } else { other(255) }
}

fn ok_single_branch(dark: bool) -> i32 {
    if dark { paint(0) } else { 42 }
}

fn main() {}
