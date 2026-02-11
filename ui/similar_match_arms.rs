// Tests for the similar_match_arms lint.

#[derive(PartialEq, PartialOrd)]
struct V;

enum Op { A, B, C, D }

// --- Should warn: 4 arms with same body structure (exact shape match) ---

fn compare(op: Op, left: &V, right: &V) -> Option<bool> {
    match op {
        Op::A => match left.partial_cmp(right) {
            Some(std::cmp::Ordering::Less) => Some(true),
            Some(_) => Some(false),
            None => None,
        },
        Op::B => match left.partial_cmp(right) {
            Some(std::cmp::Ordering::Greater) => Some(true),
            Some(_) => Some(false),
            None => None,
        },
        Op::C => match left.partial_cmp(right) {
            Some(std::cmp::Ordering::Less) => Some(true),
            Some(_) => Some(false),
            None => None,
        },
        Op::D => match left.partial_cmp(right) {
            Some(std::cmp::Ordering::Greater) => Some(true),
            Some(_) => Some(false),
            None => None,
        },
    }
}

// --- Should warn: 2 arms with similar block bodies (pairwise LCS) ---

use std::fmt;

enum Container {
    List(Vec<i32>),
    Map(Vec<(String, i32)>),
}

impl fmt::Display for Container {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::List(l) => {
                write!(f, "[")?;
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Self::Map(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
        }
    }
}

// --- Should NOT warn: only 2 arms, trivial ---

fn small(x: bool) -> i32 {
    match x {
        true => 1,
        false => 2,
    }
}

fn main() {}
