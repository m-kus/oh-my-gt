//! `gt restack` — rebase the current stack so each branch sits on its parent.
//!
//! Only attempts branches whose existing fork point is still in the parent's
//! history (i.e. a clean rebase is plausible) plus the currently checked-out
//! branch. Stale branches off the current path are reported as skipped. A
//! conflict off the current path reports the branch (check it out and restack
//! manually); a conflict on the current path pauses for manual resolution
//! (`gt continue`, or `gt abort` to undo) once everything else is done.

use crate::error::Result;
use crate::graph::StackGraph;
use crate::rebase;
use crate::style::OutputStyle;

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;
    let style = OutputStyle::stdout();

    // An untracked branch has no place in the stack; "already up to date"
    // would be misleading. Tell the user how to bring it in instead.
    if !graph.is_trunk(&current) {
        graph.require_tracked(&current)?;
    }

    // Restack the whole stack that contains the current branch.
    let stack = graph.stack_of(&current);
    let root = stack.into_iter().next().unwrap_or_else(|| current.clone());

    let selection = rebase::select_branches_for_clean_restack(&graph, &current);
    selection.print_stale(&style);
    let mut plan = rebase::plan_clean(&graph, &[root], "restack", &selection.clean)?;
    if plan.chains.is_empty() {
        if selection.stale.is_empty() {
            println!("the stack is already up to date");
        }
        return Ok(());
    }
    // The common case: a single linear chain (the current stack, no branch
    // points) whose refs really do form a line down to the tip. Replay it as one
    // `git rebase --update-refs` with no resume state of its own — git owns the
    // paused state, so `git status` and gt agree and native `git rebase
    // --continue` / `--abort` work as expected.
    if plan.chains.len() == 1 && rebase::chain_is_natively_restackable(&graph, &plan.chains[0])? {
        return rebase::restack_native(plan.chains.remove(0), &current);
    }
    // Branch points — or a chain whose parent carries a commit its children
    // never absorbed, so the parent's tip is not an ancestor of the chain tip
    // and a single `--update-refs` pass would strand it (see
    // [`rebase::chain_is_natively_restackable`]). Fall back to the state-backed
    // engine with per-branch chains: each branch is rebased onto its parent using
    // its own fork point, so every commit is carried and a conflict on one branch
    // leaves earlier branches in the same chain restacked cleanly — the same
    // trade-off `gt sync` makes.
    rebase::split_chains_per_branch(&graph, &mut plan);
    rebase::start_best_effort(&graph, plan, &selection.on_path)
}
