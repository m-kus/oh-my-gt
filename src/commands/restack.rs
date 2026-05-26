//! `gt restack` — rebase the current stack so each branch sits on its parent.
//!
//! Only attempts branches whose existing fork point is still in the parent's
//! history (i.e. a clean rebase is plausible) plus the currently checked-out
//! branch. Stale branches off the current path are reported as skipped, and
//! any branch that fails to rebase cleanly is marked outdated instead of
//! pausing the operation.

use crate::error::Result;
use crate::graph::StackGraph;
use crate::rebase::{self, CleanSelection};
use crate::style::OutputStyle;

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;

    // Restack the whole stack that contains the current branch.
    let stack = graph.stack_of(&current);
    let root = stack.into_iter().next().unwrap_or_else(|| current.clone());

    let selection = rebase::select_branches_for_clean_restack(&graph, &current);
    let mut plan = rebase::plan_clean(&graph, &[root], "restack", &selection.clean)?;
    if plan.chains.is_empty() {
        print_outcome(
            &rebase::BestEffortOutcome {
                restacked: Vec::new(),
                outdated: Vec::new(),
            },
            &selection,
        );
        return Ok(());
    }
    // Per-branch chains so that a conflict on one branch leaves earlier
    // branches in the same chain restacked cleanly — same trade-off `gt sync`
    // makes.
    rebase::split_chains_per_branch(&graph, &mut plan);
    let outcome = rebase::start_best_effort(&graph, plan)?;
    print_outcome(&outcome, &selection);
    Ok(())
}

/// Summarize: which branches moved, which were left for manual attention.
fn print_outcome(outcome: &rebase::BestEffortOutcome, selection: &CleanSelection) {
    let style = OutputStyle::stdout();
    if !outcome.restacked.is_empty() {
        println!(
            "restacked: {}",
            outcome
                .restacked
                .iter()
                .map(|b| style.branch(b).to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    if !selection.stale.is_empty() {
        let names = selection
            .stale
            .iter()
            .map(|b| style.branch(b).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{} {} (run `gt restack` manually to retry)",
            style.warning("skipped (stale):"),
            names
        );
    }
    if !outcome.outdated.is_empty() {
        let names = outcome
            .outdated
            .iter()
            .map(|b| style.branch(b).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{} {} (conflicts; needs manual restack)",
            style.warning("outdated:"),
            names
        );
    }
    if outcome.restacked.is_empty() && outcome.outdated.is_empty() && selection.stale.is_empty() {
        println!("the stack is already up to date");
    }
}
