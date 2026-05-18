//! `gt move` — re-parent the current branch onto another branch and restack
//! its descendants. No manual `git rebase --onto` is ever required.

use std::collections::HashSet;

use crate::error::{GtError, Result};
use crate::graph::{StackGraph, Validation};
use crate::{git, meta, prompt, rebase};

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;
    let node = graph.require_tracked(&current)?;
    let old_parent = node.parent.clone().unwrap();

    // Valid parent candidates: the trunk or any tracked branch that is not the
    // current branch and not one of its descendants (which would form a cycle).
    let blocked: HashSet<String> = graph
        .descendants(&current)
        .into_iter()
        .chain(std::iter::once(current.clone()))
        .collect();
    let mut candidates: Vec<String> = graph
        .nodes
        .values()
        .filter(|n| !blocked.contains(&n.name))
        .filter(|n| graph.is_trunk(&n.name) || n.validation == Validation::Valid)
        .map(|n| n.name.clone())
        .collect();
    candidates.sort();
    if candidates.is_empty() {
        return Err(GtError::State("no valid branch to move onto".into()));
    }

    let default = candidates
        .iter()
        .position(|b| *b == graph.trunk)
        .unwrap_or(0);
    let idx = prompt::select(&format!("move `{current}` onto:"), &candidates, default)?;
    let new_parent = candidates.swap_remove(idx);

    if new_parent == old_parent {
        println!("`{current}` is already on `{new_parent}`");
        return Ok(());
    }
    if graph.would_cycle(&current, &new_parent) {
        return Err(GtError::Usage(format!(
            "cannot move `{current}` onto `{new_parent}`: that would create a cycle"
        )));
    }

    // Fail before mutating metadata if the restack could not run.
    git::ensure_clean()?;
    if git::rebase_in_progress(&git::git_dir()?) {
        return Err(GtError::Precondition(
            "a git rebase is already in progress".into(),
        ));
    }
    let rollback_branches = graph.subtree(&current);
    let rollback_snapshot = rebase::snapshot_branches(&graph, &rollback_branches)?;

    // Re-parent in metadata; the fork point stays put so the restack replays
    // exactly this branch's own commits onto the new parent.
    let mut m = node.meta.clone().unwrap();
    m.parent_branch_name = Some(new_parent.clone());
    meta::write(&current, &m)?;

    println!("re-parented `{current}` from `{old_parent}` onto `{new_parent}`");
    let updated = StackGraph::load()?;
    let mut plan = rebase::plan(&updated, std::slice::from_ref(&current), "move")?;
    if plan.chains.is_empty() {
        println!("the stack is already up to date");
        return Ok(());
    }
    plan.snapshot = rollback_snapshot;
    rebase::start(&updated, plan)
}
