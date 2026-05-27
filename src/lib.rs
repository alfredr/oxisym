#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_session;
extern crate rustc_span;

mod naming;
mod same_call_both_branches;
mod shape_hash;
mod similar_fn_bodies;
mod similar_match_arms;

#[doc(hidden)]
#[unsafe(no_mangle)]
pub extern "C" fn register_lints(
    _sess: &rustc_session::Session,
    lint_store: &mut rustc_lint::LintStore,
) {
    lint_store.register_lints(&[
        same_call_both_branches::SAME_CALL_BOTH_BRANCHES,
        similar_fn_bodies::SIMILAR_FN_BODIES,
        similar_match_arms::SIMILAR_MATCH_ARMS,
    ]);
    lint_store.register_late_pass(|_| Box::new(same_call_both_branches::SameCallBothBranches));
    lint_store.register_late_pass(|_| Box::new(similar_fn_bodies::SimilarFnBodies::default()));
    lint_store.register_late_pass(|_| Box::new(similar_match_arms::SimilarMatchArms));
}

dylint_linting::dylint_library!();

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
