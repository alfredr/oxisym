use clippy_utils::SpanlessEq;
use clippy_utils::diagnostics::span_lint_and_then;
use clippy_utils::higher;
use clippy_utils::source::snippet;
use rustc_hir::{Arm, Expr, ExprKind, MatchSource, PathSegment, StmtKind};
use rustc_lint::{LateContext, LateLintPass};
use rustc_session::{declare_lint, impl_lint_pass};

declare_lint! {
    /// ### What it does
    /// Detects `match` or `if/else` expressions where every branch calls the
    /// same function or method, differing only in arguments.
    ///
    /// ### Why is this bad?
    /// The branch condition can often be factored into the call arguments,
    /// eliminating duplication.
    ///
    /// ### Example
    /// ```rust,ignore
    /// if dark { paint(Color::Black) } else { paint(Color::White) }
    /// // becomes:
    /// paint(if dark { Color::Black } else { Color::White })
    /// ```
    pub SAME_CALL_BOTH_BRANCHES,
    Warn,
    "all branches call the same function/method with different arguments"
}

#[derive(Copy, Clone)]
pub struct SameCallBothBranches;

impl_lint_pass!(SameCallBothBranches => [SAME_CALL_BOTH_BRANCHES]);

// What kind of call we found at the tail of a branch.
enum Callee<'tcx> {
    Free(&'tcx Expr<'tcx>),
    Method(&'tcx Expr<'tcx>, &'tcx PathSegment<'tcx>),
}

// Full call info: callee + arguments.
struct CallInfo<'tcx> {
    callee: Callee<'tcx>,
    args: &'tcx [Expr<'tcx>],
}

/// Peel through single-expr blocks to the terminal expression.
fn peel_blocks<'tcx>(expr: &'tcx Expr<'tcx>) -> &'tcx Expr<'tcx> {
    match &expr.kind {
        ExprKind::Block(block, _) => {
            if let Some(tail) = block.expr
                && block.stmts.is_empty()
            {
                return peel_blocks(tail);
            }
            if block.stmts.len() == 1
                && block.expr.is_none()
                && let StmtKind::Semi(e) | StmtKind::Expr(e) = block.stmts[0].kind
            {
                return peel_blocks(e);
            }
            expr
        }
        _ => expr,
    }
}

fn extract_callee<'tcx>(expr: &'tcx Expr<'tcx>) -> Option<Callee<'tcx>> {
    let peeled = peel_blocks(expr);
    match &peeled.kind {
        ExprKind::Call(func, _) => Some(Callee::Free(func)),
        ExprKind::MethodCall(seg, receiver, _, _) => Some(Callee::Method(receiver, seg)),
        _ => None,
    }
}

fn extract_call_info<'tcx>(expr: &'tcx Expr<'tcx>) -> Option<CallInfo<'tcx>> {
    let peeled = peel_blocks(expr);
    match &peeled.kind {
        ExprKind::Call(func, args) => Some(CallInfo {
            callee: Callee::Free(func),
            args,
        }),
        ExprKind::MethodCall(seg, receiver, args, _) => Some(CallInfo {
            callee: Callee::Method(receiver, seg),
            args,
        }),
        _ => None,
    }
}

fn callees_eq<'tcx>(cx: &LateContext<'tcx>, a: &Callee<'tcx>, b: &Callee<'tcx>) -> bool {
    let mut eq = SpanlessEq::new(cx).deny_side_effects();
    match (a, b) {
        (Callee::Free(fa), Callee::Free(fb)) => eq.eq_expr(fa, fb),
        (Callee::Method(ra, sa), Callee::Method(rb, sb)) => {
            sa.ident.name == sb.ident.name && eq.eq_expr(ra, rb)
        }
        _ => false,
    }
}

fn all_same_callee<'tcx>(cx: &LateContext<'tcx>, bodies: &[&'tcx Expr<'tcx>]) -> bool {
    let callees: Vec<_> = bodies.iter().filter_map(|b| extract_callee(b)).collect();
    if callees.len() != bodies.len() || callees.len() < 2 {
        return false;
    }
    let first = &callees[0];
    callees[1..].iter().all(|c| callees_eq(cx, first, c))
}

/// Flatten an if/else chain into a list of branch bodies.
fn collect_if_else_branches<'tcx>(expr: &'tcx Expr<'tcx>) -> Vec<&'tcx Expr<'tcx>> {
    let mut branches = Vec::new();
    let mut cur = expr;
    loop {
        if let Some(if_expr) = higher::If::hir(cur) {
            branches.push(if_expr.then);
            match if_expr.r#else {
                Some(else_expr) => cur = else_expr,
                None => break,
            }
        } else {
            branches.push(cur);
            break;
        }
    }
    branches
}

/// Find which argument positions differ across calls in all branches.
/// Returns `None` if calls have different arities.
fn find_differing_args<'tcx>(
    cx: &LateContext<'tcx>,
    calls: &[CallInfo<'tcx>],
) -> Option<Vec<usize>> {
    let arity = calls[0].args.len();
    if !calls[1..].iter().all(|c| c.args.len() == arity) {
        return None;
    }

    let mut differ = Vec::new();
    for i in 0..arity {
        let first = &calls[0].args[i];
        let mut eq = SpanlessEq::new(cx).deny_side_effects();
        if !calls[1..].iter().all(|c| eq.eq_expr(first, &c.args[i])) {
            differ.push(i);
        }
    }
    Some(differ)
}

/// Build a hoisted suggestion for an if/else expression (2 branches only).
fn build_if_suggestion<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &'tcx Expr<'tcx>,
    calls: &[CallInfo<'tcx>],
    differ: &[usize],
) -> Option<String> {
    let if_expr = higher::If::hir(expr)?;
    // Only handle simple if/else (no else-if chains) for now.
    if higher::If::hir(if_expr.r#else?).is_some() {
        return None;
    }

    let cond_str = snippet(cx, if_expr.cond.span, "..");
    let callee_str = callee_snippet(cx, &calls[0]);

    let arity = calls[0].args.len();
    let mut args_parts = Vec::with_capacity(arity);
    for i in 0..arity {
        if differ.contains(&i) {
            let then_arg = snippet(cx, calls[0].args[i].span, "..");
            let else_arg = snippet(cx, calls[1].args[i].span, "..");
            args_parts.push(format!(
                "if {cond_str} {{ {then_arg} }} else {{ {else_arg} }}"
            ));
        } else {
            args_parts.push(snippet(cx, calls[0].args[i].span, "..").into_owned());
        }
    }

    Some(format!("{}({})", callee_str, args_parts.join(", ")))
}

/// Build a hoisted suggestion for a match expression.
fn build_match_suggestion<'tcx>(
    cx: &LateContext<'tcx>,
    scrutinee: &'tcx Expr<'tcx>,
    arms: &'tcx [Arm<'tcx>],
    calls: &[CallInfo<'tcx>],
    differ: &[usize],
) -> String {
    let scrutinee_str = snippet(cx, scrutinee.span, "..");
    let callee_str = callee_snippet(cx, &calls[0]);

    let arity = calls[0].args.len();
    let mut args_parts = Vec::with_capacity(arity);
    for i in 0..arity {
        if differ.contains(&i) {
            // Build a match expression for this arg.
            let mut match_arms = Vec::new();
            for (j, arm) in arms.iter().enumerate() {
                let pat_str = snippet(cx, arm.pat.span, "..");
                let arg_str = snippet(cx, calls[j].args[i].span, "..");
                match_arms.push(format!("{pat_str} => {arg_str}"));
            }
            args_parts.push(format!(
                "match {scrutinee_str} {{ {} }}",
                match_arms.join(", ")
            ));
        } else {
            args_parts.push(snippet(cx, calls[0].args[i].span, "..").into_owned());
        }
    }

    format!("{}({})", callee_str, args_parts.join(", "))
}

/// Get a snippet for the callee (function path or receiver.method).
fn callee_snippet<'tcx>(cx: &LateContext<'tcx>, call: &CallInfo<'tcx>) -> String {
    match &call.callee {
        Callee::Free(func) => snippet(cx, func.span, "..").into_owned(),
        Callee::Method(receiver, seg) => {
            format!("{}.{}", snippet(cx, receiver.span, ".."), seg.ident)
        }
    }
}

impl<'tcx> LateLintPass<'tcx> for SameCallBothBranches {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        match &expr.kind {
            ExprKind::Match(scrutinee, arms, MatchSource::Normal) if arms.len() >= 2 => {
                check_match(cx, expr, scrutinee, arms);
            }
            _ if higher::If::hir(expr).is_some() => {
                let branches = collect_if_else_branches(expr);
                if branches.len() >= 2 && all_same_callee(cx, &branches) {
                    check_if_else(cx, expr, &branches);
                }
            }
            _ => {}
        }
    }
}

fn check_if_else<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &'tcx Expr<'tcx>,
    branches: &[&'tcx Expr<'tcx>],
) {
    let calls: Vec<_> = branches
        .iter()
        .filter_map(|b| extract_call_info(b))
        .collect();
    if calls.len() != branches.len() {
        return;
    }

    let differ = find_differing_args(cx, &calls);

    span_lint_and_then(
        cx,
        SAME_CALL_BOTH_BRANCHES,
        expr.span,
        "all if/else branches call the same function — \
         hoist the call and factor the condition into the arguments",
        |diag| {
            // Try to build a concrete suggestion for simple cases.
            if let Some(ref diff) = differ
                && branches.len() == 2
                && !diff.is_empty()
                && let Some(suggestion) = build_if_suggestion(cx, expr, &calls, diff)
            {
                diag.span_suggestion(
                    expr.span,
                    "try",
                    suggestion,
                    rustc_errors::Applicability::MaybeIncorrect,
                );
                return;
            }
            // Fallback to generic help.
            diag.help(
                "e.g. `f(if cond { a } else { b })` instead of \
                 `if cond { f(a) } else { f(b) }`",
            );
        },
    );
}

fn check_match<'tcx>(
    cx: &LateContext<'tcx>,
    match_expr: &'tcx Expr<'tcx>,
    scrutinee: &'tcx Expr<'tcx>,
    arms: &'tcx [Arm<'tcx>],
) {
    let bodies: Vec<&Expr<'_>> = arms.iter().map(|a| a.body).collect();
    if !all_same_callee(cx, &bodies) {
        return;
    }

    let calls: Vec<_> = bodies.iter().filter_map(|b| extract_call_info(b)).collect();
    if calls.len() != bodies.len() {
        return;
    }

    let differ = find_differing_args(cx, &calls);

    span_lint_and_then(
        cx,
        SAME_CALL_BOTH_BRANCHES,
        match_expr.span,
        "all match arms call the same function — \
         hoist the call and factor the match into the arguments",
        |diag| {
            if let Some(ref diff) = differ
                && !diff.is_empty()
            {
                let suggestion = build_match_suggestion(cx, scrutinee, arms, &calls, diff);
                diag.span_suggestion(
                    match_expr.span,
                    "try",
                    suggestion,
                    rustc_errors::Applicability::MaybeIncorrect,
                );
                return;
            }
            diag.help(
                "e.g. `f(match x { A => a, B => b })` instead of \
                 `match x { A => f(a), B => f(b) }`",
            );
        },
    );
}
