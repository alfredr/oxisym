//! A "shape hasher" for HIR expressions that hashes the structural skeleton
//! but normalises away literal values and identifier names.
//!
//! This lets us detect that `check_arity("left", &args, 2)` and
//! `check_arity("right", &args, 2)` have the *same shape*, even though
//! `SpanlessHash` produces different hashes for them.

use rustc_hir::{Block, Expr, ExprKind, Stmt, StmtKind};
use std::hash::{Hash, Hasher};

/// Hash a statement's structural shape (ignoring literal values and names).
pub fn shape_hash_stmt(s: &Stmt<'_>) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    hash_stmt(&mut h, s);
    h.finish()
}

/// Hash an expression's shape into an existing hasher (for use by callers
/// that need to combine it with other data).
pub fn shape_hash_expr_into(h: &mut impl Hasher, e: &Expr<'_>) {
    hash_expr(h, e);
}

fn hash_stmt(h: &mut impl Hasher, s: &Stmt<'_>) {
    std::mem::discriminant(&s.kind).hash(h);
    match &s.kind {
        StmtKind::Let(local) => {
            // Hash the pattern shape (just that there's a pattern), type, init.
            local.ty.is_some().hash(h);
            if let Some(init) = local.init {
                hash_expr(h, init);
            }
            if let Some(els) = local.els {
                hash_block(h, els);
            }
        }
        StmtKind::Item(_) => {}
        StmtKind::Expr(e) | StmtKind::Semi(e) => {
            hash_expr(h, e);
        }
    }
}

fn hash_block(h: &mut impl Hasher, b: &Block<'_>) {
    b.stmts.len().hash(h);
    for s in b.stmts {
        hash_stmt(h, s);
    }
    if let Some(tail) = b.expr {
        hash_expr(h, tail);
    }
}

#[allow(clippy::cognitive_complexity)] // Inherently branchy match dispatcher; 9 vs threshold 8.
fn hash_expr(h: &mut impl Hasher, e: &Expr<'_>) {
    std::mem::discriminant(&e.kind).hash(h);
    match &e.kind {
        // Literals: hash only the discriminant of the literal kind,
        // NOT the actual value.  This is the key difference from SpanlessHash.
        ExprKind::Lit(lit) => {
            std::mem::discriminant(&lit.node).hash(h);
        }

        // Calls: hash the callee structure + arg count + each arg shape.
        ExprKind::Call(func, args) => {
            hash_expr(h, func);
            args.len().hash(h);
            for a in *args {
                hash_expr(h, a);
            }
        }
        ExprKind::MethodCall(path, receiver, args, _) => {
            // DO hash the method name — we want `set_property` == `set_property`
            // but `set_node_property` != `remove_property`.
            path.ident.name.hash(h);
            hash_expr(h, receiver);
            args.len().hash(h);
            for a in *args {
                hash_expr(h, a);
            }
        }

        ExprKind::Binary(op, l, r) => {
            std::mem::discriminant(&op.node).hash(h);
            hash_expr(h, l);
            hash_expr(h, r);
        }
        ExprKind::AssignOp(op, l, r) => {
            std::mem::discriminant(&op.node).hash(h);
            hash_expr(h, l);
            hash_expr(h, r);
        }
        ExprKind::Unary(op, e) => {
            std::mem::discriminant(op).hash(h);
            hash_expr(h, e);
        }
        ExprKind::Path(qpath) => {
            std::mem::discriminant(qpath).hash(h);
        }
        ExprKind::If(cond, then, els) => {
            hash_expr(h, cond);
            hash_expr(h, then);
            els.is_some().hash(h);
            if let Some(e) = els {
                hash_expr(h, e);
            }
        }
        ExprKind::Match(scrutinee, arms, _) => {
            hash_expr(h, scrutinee);
            arms.len().hash(h);
            for arm in *arms {
                if let Some(g) = arm.guard {
                    hash_expr(h, g);
                }
                hash_expr(h, arm.body);
            }
        }
        ExprKind::Block(b, _) | ExprKind::Loop(b, _, _, _) => hash_block(h, b),
        ExprKind::AddrOf(_, m, e) => {
            std::mem::discriminant(m).hash(h);
            hash_expr(h, e);
        }
        ExprKind::Field(e, field) => {
            hash_expr(h, e);
            field.name.hash(h);
        }
        ExprKind::Index(l, r, _) | ExprKind::Assign(l, r, _) => {
            hash_expr(h, l);
            hash_expr(h, r);
        }

        // Single-subexpression forms.
        ExprKind::Cast(e, _)
        | ExprKind::Type(e, _)
        | ExprKind::Ret(Some(e))
        | ExprKind::Break(_, Some(e))
        | ExprKind::Repeat(e, _)
        | ExprKind::DropTemps(e)
        | ExprKind::Yield(e, _)
        | ExprKind::Become(e) => {
            hash_expr(h, e);
        }

        // Struct literal.
        ExprKind::Struct(_, fields, _base) => {
            fields.len().hash(h);
            for f in *fields {
                hash_expr(h, f.expr);
            }
        }

        // Tuple / Array.
        ExprKind::Tup(exprs) | ExprKind::Array(exprs) => {
            exprs.len().hash(h);
            for e in *exprs {
                hash_expr(h, e);
            }
        }

        // Let-in-if (let ... = expr).
        ExprKind::Let(let_expr) => {
            hash_expr(h, let_expr.init);
        }

        // Everything else: just the discriminant is enough.
        _ => {}
    }
}
