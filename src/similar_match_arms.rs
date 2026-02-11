use crate::shape_hash::{shape_hash_expr_into, shape_hash_stmt};
use clippy_utils::diagnostics::span_lint_and_then;
use rustc_hir::{Arm, Block, Expr, ExprKind, MatchSource};
use rustc_lint::{LateContext, LateLintPass};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Span;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

declare_lint! {
    /// ### What it does
    /// Detects `match` expressions where multiple arms have structurally
    /// similar bodies — either identical shape (3+ arms) or high LCS-based
    /// similarity (2+ arms with block bodies).
    ///
    /// ### Why is this bad?
    /// Repeated structure across match arms usually indicates the common
    /// logic can be extracted into a helper function, with the varying part
    /// passed as a parameter or closure.
    pub SIMILAR_MATCH_ARMS,
    Warn,
    "multiple match arms with structurally similar bodies"
}

#[derive(Copy, Clone)]
pub struct SimilarMatchArms;

impl_lint_pass!(SimilarMatchArms => [SIMILAR_MATCH_ARMS]);

fn shape_hash_expr(e: &Expr<'_>) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    shape_hash_expr_into(&mut h, e);
    h.finish()
}

/// Minimum number of arms that must share an exact shape to trigger.
const MIN_SIMILAR_ARMS: usize = 3;

/// Minimum similarity ratio for pairwise LCS comparison of arm bodies.
const ARM_SIMILARITY_THRESHOLD: f64 = 0.55;

/// Minimum number of statements in an arm body to be eligible for pairwise
/// LCS comparison (avoids noise from tiny arms).
const MIN_ARM_STMTS: usize = 3;

/// Hash the statements + tail of a block into a Vec of per-statement hashes,
/// same approach as `similar_fn_bodies`.
fn hash_block_stmts(block: &Block<'_>) -> Vec<u64> {
    let mut hashes = Vec::with_capacity(block.stmts.len() + 1);
    for stmt in block.stmts {
        hashes.push(shape_hash_stmt(stmt));
    }
    if let Some(tail) = block.expr {
        let mut h = std::hash::DefaultHasher::new();
        0xCAFE_u64.hash(&mut h);
        shape_hash_expr_into(&mut h, tail);
        hashes.push(h.finish());
    }
    hashes
}

/// LCS length between two hash sequences.
fn lcs_len(a: &[u64], b: &[u64]) -> usize {
    let m = a.len();
    let n = b.len();
    let mut prev = vec![0usize; n + 1];
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        for j in 1..=n {
            curr[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1] + 1
            } else {
                prev[j].max(curr[j - 1])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.fill(0);
    }
    prev[n]
}

/// Count how many leaf nodes differ between two expressions with the same shape.
fn count_differing_leaves(a: &Expr<'_>, b: &Expr<'_>) -> Option<usize> {
    diff_expr(a, b)
}

#[allow(clippy::cognitive_complexity)] // Match dispatcher over ExprKind variants; inherently branchy.
fn diff_expr(a: &Expr<'_>, b: &Expr<'_>) -> Option<usize> {
    match (&a.kind, &b.kind) {
        // Leaves.
        (ExprKind::Lit(la), ExprKind::Lit(lb))
            if std::mem::discriminant(&la.node) == std::mem::discriminant(&lb.node) =>
        {
            Some(usize::from(la.span != lb.span))
        }
        (ExprKind::Path(_), ExprKind::Path(_)) => Some(usize::from(a.span != b.span)),
        (ExprKind::Ret(None), ExprKind::Ret(None))
        | (ExprKind::Break(_, None), ExprKind::Break(_, None))
        | (ExprKind::Continue(_), ExprKind::Continue(_)) => Some(0),

        // Calls: diff head + argument list.
        (ExprKind::Call(fa, args_a), ExprKind::Call(fb, args_b))
            if args_a.len() == args_b.len() =>
        {
            diff_head_and_args(fa, fb, args_a, args_b)
        }
        (ExprKind::MethodCall(sa, ra, args_a, _), ExprKind::MethodCall(sb, rb, args_b, _))
            if sa.ident.name == sb.ident.name && args_a.len() == args_b.len() =>
        {
            diff_head_and_args(ra, rb, args_a, args_b)
        }

        // Two-subexpression forms.
        (ExprKind::Binary(oa, la, ra), ExprKind::Binary(ob, lb, rb))
            if std::mem::discriminant(&oa.node) == std::mem::discriminant(&ob.node) =>
        {
            Some(diff_expr(la, lb)? + diff_expr(ra, rb)?)
        }
        (ExprKind::Index(la, ra, _), ExprKind::Index(lb, rb, _))
        | (ExprKind::Assign(la, ra, _), ExprKind::Assign(lb, rb, _)) => {
            Some(diff_expr(la, lb)? + diff_expr(ra, rb)?)
        }

        // Single-subexpression forms.
        (ExprKind::Unary(oa, ea), ExprKind::Unary(ob, eb))
            if std::mem::discriminant(oa) == std::mem::discriminant(ob) =>
        {
            diff_expr(ea, eb)
        }
        (ExprKind::AddrOf(_, _, ea), ExprKind::AddrOf(_, _, eb))
        | (ExprKind::Ret(Some(ea)), ExprKind::Ret(Some(eb)))
        | (ExprKind::Break(_, Some(ea)), ExprKind::Break(_, Some(eb)))
        | (ExprKind::Field(ea, _), ExprKind::Field(eb, _))
        | (ExprKind::DropTemps(ea), ExprKind::DropTemps(eb)) => diff_expr(ea, eb),

        // Compound: control flow + blocks.
        (ExprKind::If(ca, ta, ea), ExprKind::If(cb, tb, eb)) => diff_if(ca, ta, *ea, cb, tb, *eb),
        (ExprKind::Match(sa, arms_a, _), ExprKind::Match(sb, arms_b, _))
            if arms_a.len() == arms_b.len() =>
        {
            diff_match(sa, sb, arms_a, arms_b)
        }
        (ExprKind::Block(ba, _), ExprKind::Block(bb, _))
            if ba.stmts.len() == bb.stmts.len() =>
        {
            diff_block(ba, bb)
        }
        (ExprKind::Tup(as_), ExprKind::Tup(bs)) if as_.len() == bs.len() => diff_slice(as_, bs),

        _ => None,
    }
}

/// Diff a head expression followed by a zipped argument list.
fn diff_head_and_args(
    head_a: &Expr<'_>, head_b: &Expr<'_>,
    args_a: &[Expr<'_>], args_b: &[Expr<'_>],
) -> Option<usize> {
    let mut total = diff_expr(head_a, head_b)?;
    for (a, b) in args_a.iter().zip(args_b.iter()) {
        total += diff_expr(a, b)?;
    }
    Some(total)
}

fn diff_if(
    ca: &Expr<'_>, ta: &Expr<'_>, ea: Option<&Expr<'_>>,
    cb: &Expr<'_>, tb: &Expr<'_>, eb: Option<&Expr<'_>>,
) -> Option<usize> {
    let mut total = diff_expr(ca, cb)? + diff_expr(ta, tb)?;
    match (ea, eb) {
        (Some(a), Some(b)) => total += diff_expr(a, b)?,
        (None, None) => {}
        _ => return None,
    }
    Some(total)
}

fn diff_match(
    sa: &Expr<'_>, sb: &Expr<'_>,
    arms_a: &[Arm<'_>], arms_b: &[Arm<'_>],
) -> Option<usize> {
    let mut total = diff_expr(sa, sb)?;
    for (aa, ab) in arms_a.iter().zip(arms_b.iter()) {
        total += diff_expr(aa.body, ab.body)?;
        if aa.pat.span != ab.pat.span {
            total += 1;
        }
    }
    Some(total)
}

fn diff_block(ba: &Block<'_>, bb: &Block<'_>) -> Option<usize> {
    let mut total = 0;
    for (sa, sb) in ba.stmts.iter().zip(bb.stmts.iter()) {
        total += diff_stmt(sa, sb)?;
    }
    match (ba.expr, bb.expr) {
        (Some(a), Some(b)) => total += diff_expr(a, b)?,
        (None, None) => {}
        _ => return None,
    }
    Some(total)
}

fn diff_slice(as_: &[Expr<'_>], bs: &[Expr<'_>]) -> Option<usize> {
    let mut total = 0;
    for (a, b) in as_.iter().zip(bs.iter()) {
        total += diff_expr(a, b)?;
    }
    Some(total)
}

fn diff_stmt(a: &rustc_hir::Stmt<'_>, b: &rustc_hir::Stmt<'_>) -> Option<usize> {
    use rustc_hir::StmtKind;
    match (&a.kind, &b.kind) {
        (StmtKind::Let(la), StmtKind::Let(lb)) => {
            match (la.init, lb.init) {
                (Some(a), Some(b)) => diff_expr(a, b),
                (None, None) => Some(0),
                _ => None,
            }
        }
        (StmtKind::Expr(a), StmtKind::Expr(b))
        | (StmtKind::Semi(a), StmtKind::Semi(b)) => diff_expr(a, b),
        _ => None,
    }
}

/// Try to derive a name for the helper from the enclosing function context.
fn suggest_name_from_context<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &'tcx Expr<'tcx>,
) -> String {
    let mut id = expr.hir_id;
    loop {
        let parent = cx.tcx.parent_hir_id(id);
        if parent == id {
            break;
        }
        id = parent;
        match cx.tcx.hir_node(id) {
            rustc_hir::Node::Item(item)
                if matches!(item.kind, rustc_hir::ItemKind::Fn { .. }) =>
            {
                let fn_name = cx.tcx.item_name(item.owner_id.def_id);
                let name_str = fn_name.as_str();
                if !name_str.is_empty() {
                    return format!("{name_str}_helper");
                }
            }
            rustc_hir::Node::ImplItem(item)
                if matches!(item.kind, rustc_hir::ImplItemKind::Fn(..)) =>
            {
                let name_str = item.ident.name.as_str();
                if !name_str.is_empty() {
                    return format!("{name_str}_helper");
                }
            }
            _ => continue,
        }
        break;
    }
    "apply_arm".into()
}

/// Check if an expression is "simple" — a path, literal, or flat call with
/// only path/literal arguments.
fn is_simple_expr(expr: &Expr<'_>) -> bool {
    match &expr.kind {
        ExprKind::Path(_) | ExprKind::Lit(_) => true,
        ExprKind::Call(_, args) => {
            args.iter()
                .all(|a| matches!(a.kind, ExprKind::Path(_) | ExprKind::Lit(_)))
        }
        ExprKind::MethodCall(_, receiver, args, _) => {
            is_simple_expr(receiver)
                && args
                    .iter()
                    .all(|a| matches!(a.kind, ExprKind::Path(_) | ExprKind::Lit(_)))
        }
        ExprKind::Block(block, _) if block.stmts.is_empty() => {
            block.expr.is_some_and(is_simple_expr)
        }
        _ => false,
    }
}

fn is_trivial_group(group: &[&Arm<'_>]) -> bool {
    group.iter().all(|arm| is_simple_expr(arm.body))
}

/// Extract the block from an arm body (peeling single-expr wrappers).
fn arm_body_block<'a>(arm: &'a Arm<'a>) -> Option<&'a Block<'a>> {
    match &arm.body.kind {
        ExprKind::Block(block, _) => Some(block),
        _ => None,
    }
}

impl<'tcx> LateLintPass<'tcx> for SimilarMatchArms {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        let ExprKind::Match(_, arms, MatchSource::Normal) = &expr.kind else {
            return;
        };
        if arms.len() < 2 {
            return;
        }

        let mut reported: Vec<bool> = vec![false; arms.len()];
        check_exact_shape_groups(cx, expr, arms, &mut reported);
        check_pairwise_lcs(cx, expr, arms, &mut reported);
    }
}

/// Pass 1: Exact shape-hash groups (3+ arms).
fn check_exact_shape_groups<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &'tcx Expr<'tcx>,
    arms: &'tcx [Arm<'tcx>],
    reported: &mut [bool],
) {
    if arms.len() < MIN_SIMILAR_ARMS {
        return;
    }
    let mut groups: HashMap<u64, Vec<(usize, &Arm<'_>)>> = HashMap::new();
    for (idx, arm) in arms.iter().enumerate() {
        let hash = shape_hash_expr(arm.body);
        groups.entry(hash).or_default().push((idx, arm));
    }

    for group in groups.values() {
        if group.len() < MIN_SIMILAR_ARMS
            || is_trivial_group(
                &group.iter().map(|(_, arm)| *arm).collect::<Vec<_>>(),
            )
        {
            continue;
        }
        for &(idx, _) in group {
            reported[idx] = true;
        }

        let spans: Vec<Span> = group.iter().map(|(_, a)| a.span).collect();
        let diff_count = count_differing_leaves(group[0].1.body, group[1].1.body);

        let advice = match diff_count {
            Some(1) => "the arms differ in exactly 1 leaf — \
                        pass the varying value as a parameter",
            Some(n) if n > 1 => "the arms differ in multiple leaves — \
                                 pass a closure for the varying behavior",
            _ => "extract the shared pattern into a function and pass \
                  the differing part as a parameter",
        };

        let suggested_name = suggest_name_from_context(cx, expr);

        span_lint_and_then(
            cx,
            SIMILAR_MATCH_ARMS,
            spans[0],
            format!(
                "{} match arms have the same body structure — \
                 extract into a helper",
                group.len(),
            ),
            |diag| {
                for arm_span in &spans[1..] {
                    diag.span_note(*arm_span, "similar arm here");
                }
                diag.help(format!(
                    "suggested helper name: `{suggested_name}`\n\
                     transform: T2 — extract predicate / collapse arms\n\
                     {advice}",
                ));
            },
        );
    }
}

/// Pass 2: Pairwise LCS similarity for block-bodied arms.
fn check_pairwise_lcs<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &'tcx Expr<'tcx>,
    arms: &'tcx [Arm<'tcx>],
    reported: &mut [bool],
) {
    let arm_hashes: Vec<Option<Vec<u64>>> = arms
        .iter()
        .map(|arm| arm_body_block(arm).map(hash_block_stmts))
        .collect();

    for i in 0..arms.len() {
        if reported[i] {
            continue;
        }
        let Some(ref hashes_a) = arm_hashes[i] else {
            continue;
        };
        if hashes_a.len() < MIN_ARM_STMTS {
            continue;
        }

        for j in (i + 1)..arms.len() {
            if reported[j] {
                continue;
            }
            let Some(ref hashes_b) = arm_hashes[j] else {
                continue;
            };
            if hashes_b.len() < MIN_ARM_STMTS {
                continue;
            }

            if compare_arm_pair(cx, expr, arms, i, j, hashes_a, hashes_b) {
                reported[i] = true;
                reported[j] = true;
            }
        }
    }
}

/// Compare two arm bodies by LCS similarity. Returns `true` if a warning was emitted.
#[allow(clippy::cast_precision_loss)]
fn compare_arm_pair<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &'tcx Expr<'tcx>,
    arms: &'tcx [Arm<'tcx>],
    i: usize,
    j: usize,
    hashes_a: &[u64],
    hashes_b: &[u64],
) -> bool {
    let max_len = hashes_a.len().max(hashes_b.len());
    let min_len = hashes_a.len().min(hashes_b.len());

    if (min_len as f64 / max_len as f64) < ARM_SIMILARITY_THRESHOLD {
        return false;
    }

    let lcs = lcs_len(hashes_a, hashes_b);
    let similarity = lcs as f64 / max_len as f64;

    if similarity < ARM_SIMILARITY_THRESHOLD {
        return false;
    }
    if is_simple_expr(arms[i].body) && is_simple_expr(arms[j].body) {
        return false;
    }

    let suggested_name = suggest_name_from_context(cx, expr);

    span_lint_and_then(
        cx,
        SIMILAR_MATCH_ARMS,
        arms[i].span,
        format!(
            "2 match arms have {:.0}% structural similarity — \
             consider extracting shared logic into a helper",
            similarity * 100.0,
        ),
        |diag| {
            diag.span_note(arms[j].span, "similar arm here");
            diag.help(format!(
                "suggested helper name: `{suggested_name}`\n\
                 transform: T3 — extract shared preamble\n\
                 extract the common logic into a helper \
                 taking a closure or parameter for the differing parts",
            ));
        },
    );
    true
}
