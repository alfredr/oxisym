use crate::naming::suggest_helper_name;
use crate::shape_hash::shape_hash_stmt;
use clippy_utils::diagnostics::span_lint_and_then;
use rustc_hir::def_id::LocalDefId;
use rustc_hir::intravisit::FnKind;
use rustc_hir::{Body, ExprKind, FnDecl, Stmt};
use rustc_lint::{LateContext, LateLintPass};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::{Span, Symbol};
use std::collections::HashMap;

declare_lint! {
    /// ### What it does
    /// Detects pairs of functions in the same module whose bodies share a
    /// high proportion of structurally-identical statements.
    ///
    /// Uses "shape hashing" which ignores literal values, so functions like
    /// `call_left` and `call_right` that differ only in string arguments
    /// are correctly identified as near-duplicates.
    ///
    /// ### Why is this bad?
    /// Near-duplicate function bodies increase maintenance burden. The common
    /// logic can usually be extracted into a shared helper parameterised on
    /// the part that differs.
    pub SIMILAR_FN_BODIES,
    Warn,
    "functions in the same module with structurally similar bodies"
}

struct FnInfo {
    name: Symbol,
    span: Span,
    /// Shape hashes per statement — ignores literal values.
    shape_hashes: Vec<u64>,
}

#[derive(Default)]
pub struct SimilarFnBodies {
    modules: HashMap<LocalDefId, Vec<FnInfo>>,
}

impl_lint_pass!(SimilarFnBodies => [SIMILAR_FN_BODIES]);

const THRESHOLD: f64 = 0.55;
const HIGH_SIMILARITY: f64 = 0.95;
const MIN_STMTS: usize = 3;
/// If two functions share at least this many prefix statements, flag them
/// regardless of the LCS ratio. A 4-statement shared preamble is substantial.
const MIN_SHARED_PREFIX: usize = 4;

fn hash_stmts_and_tail<'tcx>(
    stmts: &'tcx [Stmt<'tcx>],
    tail: Option<&'tcx rustc_hir::Expr<'tcx>>,
) -> Vec<u64> {
    let mut hashes = Vec::with_capacity(stmts.len() + 1);
    for stmt in stmts {
        hashes.push(shape_hash_stmt(stmt));
    }
    if let Some(tail) = tail {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        0xCAFE_u64.hash(&mut h);
        crate::shape_hash::shape_hash_expr_into(&mut h, tail);
        hashes.push(h.finish());
    }
    hashes
}

/// Longest common subsequence length (O(n*m), fine for function-sized inputs).
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

/// Count how many statements at the start of both hash sequences match.
fn shared_prefix_len(a: &[u64], b: &[u64]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

impl<'tcx> LateLintPass<'tcx> for SimilarFnBodies {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        kind: FnKind<'tcx>,
        _decl: &'tcx FnDecl<'tcx>,
        body: &'tcx Body<'tcx>,
        span: Span,
        local_def_id: LocalDefId,
    ) {
        let name = match kind {
            FnKind::ItemFn(ident, ..) | FnKind::Method(ident, ..) => ident.name,
            FnKind::Closure => return,
        };

        let body_expr = body.value;
        let shape_hashes = if let ExprKind::Block(block, _) = &body_expr.kind {
            hash_stmts_and_tail(block.stmts, block.expr)
        } else {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            0xCAFE_u64.hash(&mut h);
            crate::shape_hash::shape_hash_expr_into(&mut h, body_expr);
            vec![h.finish()]
        };

        if shape_hashes.len() < MIN_STMTS {
            return;
        }

        let hir_id = cx.tcx.local_def_id_to_hir_id(local_def_id);
        let parent_mod = cx.tcx.parent_module(hir_id);

        self.modules
            .entry(parent_mod.to_local_def_id())
            .or_default()
            .push(FnInfo {
                name,
                span,
                shape_hashes,
            });
    }

    fn check_crate_post(&mut self, cx: &LateContext<'tcx>) {
        for fns in self.modules.values() {
            compare_all_pairs(cx, fns);
        }
    }
}

fn compare_all_pairs(cx: &LateContext<'_>, fns: &[FnInfo]) {
    for i in 0..fns.len() {
        for j in (i + 1)..fns.len() {
            compare_fn_pair(cx, &fns[i], &fns[j]);
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn compare_fn_pair(cx: &LateContext<'_>, a: &FnInfo, b: &FnInfo) {
    let max_len = a.shape_hashes.len().max(b.shape_hashes.len());
    let prefix_len = shared_prefix_len(&a.shape_hashes, &b.shape_hashes);

    // Quick reject on length ratio (unless prefix is long enough).
    let min_len = a.shape_hashes.len().min(b.shape_hashes.len());
    if (min_len as f64 / max_len as f64) < THRESHOLD && prefix_len < MIN_SHARED_PREFIX {
        return;
    }

    // LCS-based similarity.
    let lcs = lcs_len(&a.shape_hashes, &b.shape_hashes);
    let similarity = lcs as f64 / max_len as f64;

    // Trigger conditions:
    // - T4 (near-identical): always flag
    // - T3 (shared preamble): require a meaningful prefix to extract
    let is_actionable = if similarity >= HIGH_SIMILARITY {
        true
    } else {
        (similarity >= THRESHOLD && prefix_len >= 2) || prefix_len >= MIN_SHARED_PREFIX
    };
    if !is_actionable {
        return;
    }

    let name_a = a.name.as_str();
    let name_b = b.name.as_str();
    let suggested = suggest_helper_name(&[name_a, name_b]);

    let (transform, advice) = if similarity >= HIGH_SIMILARITY {
        (
            "T4: merge and parameterize",
            "these functions are near-identical — merge into \
             one function parameterized on the differing constants",
        )
    } else {
        (
            "T3: extract shared preamble",
            "extract the common prefix into a helper \
             taking a closure for the differing tail",
        )
    };

    span_lint_and_then(
        cx,
        SIMILAR_FN_BODIES,
        a.span,
        format!(
            "`{}` and `{}` have {:.0}% structural similarity",
            a.name,
            b.name,
            similarity * 100.0,
        ),
        |diag| {
            diag.span_note(b.span, format!("`{}` defined here", b.name));
            let mut help = format!(
                "suggested helper name: `{suggested}`\n\
                 transform: {transform}\n\
                 {advice}"
            );
            if similarity < HIGH_SIMILARITY {
                use std::fmt::Write;
                let _ = write!(
                    help,
                    "\nshared preamble: {prefix_len} of {max_len} statements",
                );
            }
            diag.help(help);
        },
    );
}
