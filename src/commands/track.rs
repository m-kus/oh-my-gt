//! `gt track` — start tracking the current branch by recording its parent.

use crate::error::{GtError, Result};
use crate::graph::{StackGraph, Validation};
use crate::style::OutputStyle;
use crate::{git, meta, prompt};

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;

    if graph.is_trunk(&current) {
        return Err(GtError::Usage(format!(
            "`{current}` is the trunk and cannot be tracked"
        )));
    }

    if let Some(node) = graph.get(&current) {
        if node.validation == Validation::Valid {
            let parent = node.parent.clone().unwrap();
            let q =
                format!("`{current}` is already tracked (parent `{parent}`); re-pick its parent?");
            if !prompt::confirm(&q, false)? {
                return Ok(());
            }
        }
    }

    let parent = choose_parent(&graph, &current)?;
    let base = git::merge_base(&parent, &current)?;

    let mut m = graph
        .get(&current)
        .and_then(|n| n.meta.clone())
        .unwrap_or_default();
    m.parent_branch_name = Some(parent.clone());
    m.parent_branch_revision = Some(base);
    meta::write(&current, &m)?;

    let style = OutputStyle::stdout();
    println!(
        "{} `{}` onto `{}`",
        style.success("tracked"),
        style.branch(&current),
        style.branch(&parent),
    );
    Ok(())
}

/// Offer the ancestor branches of `current` as parent candidates, best guess first.
fn choose_parent(graph: &StackGraph, current: &str) -> Result<String> {
    let current_tip = &graph.get(current).unwrap().tip;

    let mut candidates: Vec<String> = Vec::new();
    for (name, node) in &graph.nodes {
        if name == current {
            continue;
        }
        if git::is_ancestor(&node.tip, current_tip)? {
            candidates.push(name.clone());
        }
    }
    if candidates.is_empty() {
        candidates.push(graph.trunk.clone());
    }
    candidates.sort();

    // Best guess: the candidate that has the most other candidates as ancestors
    // (i.e. the one deepest in the stack, closest to `current`).
    let mut best = 0usize;
    let mut best_depth = -1i32;
    for (i, c) in candidates.iter().enumerate() {
        let c_tip = &graph.nodes[c].tip;
        let mut depth = 0i32;
        for other in &candidates {
            if other != c && git::is_ancestor(&graph.nodes[other].tip, c_tip)? {
                depth += 1;
            }
        }
        if depth > best_depth {
            best_depth = depth;
            best = i;
        }
    }

    let idx = prompt::select(
        &format!("parent branch for `{current}`:"),
        &candidates,
        best,
    )?;
    Ok(candidates.swap_remove(idx))
}
