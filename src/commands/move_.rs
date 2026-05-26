//! `gt move` — re-parent the current branch onto another branch and restack
//! its descendants. No manual `git rebase --onto` is ever required.

use std::collections::HashSet;

use crate::error::{GtError, Result};
use crate::graph::{StackGraph, Validation};
use crate::{git, meta, prompt, rebase, tree};

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;
    let node = graph.require_tracked(&current)?;
    let old_parent = node.parent.clone().unwrap();

    let selectable = selectable_parents(&graph, &current);
    if selectable.is_empty() {
        return Err(GtError::State("no valid branch to move onto".into()));
    }
    let lines = tree::branch_lines(&graph, &selectable);

    // Default to trunk when it is itself a valid target; otherwise fall back
    // to the first selectable branch in tree order.
    let default = if selectable.contains(&graph.trunk) {
        graph.trunk.clone()
    } else {
        lines
            .iter()
            .find(|l| l.selectable)
            .map(|l| l.branch.clone())
            .unwrap()
    };
    let new_parent = prompt::select_tree(&format!("move `{current}` onto:"), &lines, &default)?;

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

/// Branches the user may pick as a new parent: trunk plus every `Valid`
/// tracked branch, minus the current branch and any of its descendants
/// (re-parenting onto a descendant would form a cycle).
fn selectable_parents(graph: &StackGraph, current: &str) -> HashSet<String> {
    let blocked: HashSet<String> = graph
        .descendants(current)
        .into_iter()
        .chain(std::iter::once(current.to_string()))
        .collect();
    graph
        .nodes
        .values()
        .filter(|n| !blocked.contains(&n.name))
        .filter(|n| graph.is_trunk(&n.name) || n.validation == Validation::Valid)
        .map(|n| n.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{BranchNode, StackGraph};
    use crate::meta::BranchMetadata;
    use std::collections::HashMap;

    /// Stack shape:
    ///
    ///   main
    ///     alpha
    ///       beta
    ///         delta
    ///       gamma
    ///
    /// `beta` is checked out; valid destinations are everything except beta
    /// itself and beta's descendant `delta`.
    fn multi_branch_graph() -> StackGraph {
        let mut nodes: HashMap<String, BranchNode> = HashMap::new();
        let insert = |nodes: &mut HashMap<String, BranchNode>,
                      name: &str,
                      tip: &str,
                      parent: Option<(&str, &str)>,
                      validation: Validation,
                      children: Vec<&str>| {
            nodes.insert(
                name.to_string(),
                BranchNode {
                    name: name.into(),
                    tip: tip.into(),
                    meta: parent.map(|(p, r)| BranchMetadata::new(p, r)),
                    validation,
                    parent: parent.map(|(p, _)| p.to_string()),
                    children: children.iter().map(|c| (*c).to_string()).collect(),
                },
            );
        };
        insert(
            &mut nodes,
            "main",
            "1111111111111111",
            None,
            Validation::Trunk,
            vec!["alpha"],
        );
        insert(
            &mut nodes,
            "alpha",
            "2222222222222222",
            Some(("main", "1111111111111111")),
            Validation::Valid,
            vec!["beta", "gamma"],
        );
        insert(
            &mut nodes,
            "beta",
            "3333333333333333",
            Some(("alpha", "2222222222222222")),
            Validation::Valid,
            vec!["delta"],
        );
        insert(
            &mut nodes,
            "delta",
            "4444444444444444",
            Some(("beta", "3333333333333333")),
            Validation::Valid,
            vec![],
        );
        insert(
            &mut nodes,
            "gamma",
            "5555555555555555",
            Some(("alpha", "2222222222222222")),
            Validation::Valid,
            vec![],
        );
        StackGraph {
            nodes,
            trunk: "main".into(),
            current: Some("beta".into()),
        }
    }

    #[test]
    fn selectable_parents_excludes_current_and_descendants() {
        let graph = multi_branch_graph();
        let set = selectable_parents(&graph, "beta");
        let mut names: Vec<&String> = set.iter().collect();
        names.sort();
        // beta is the current branch, delta is its descendant — both blocked.
        assert_eq!(names, vec!["alpha", "gamma", "main"]);
    }

    #[test]
    fn picker_shows_full_tree_with_only_valid_lines_selectable() {
        let graph = multi_branch_graph();
        let selectable = selectable_parents(&graph, "beta");
        let lines = tree::branch_lines(&graph, &selectable);

        // The picker must reflect the entire stack shape — including beta
        // (current) and delta (descendant) — even though they are blocked.
        let names: Vec<&str> = lines.iter().map(|l| l.branch.as_str()).collect();
        assert_eq!(names, vec!["main", "alpha", "beta", "delta", "gamma"]);

        // Only the parent candidates are selectable.
        let pickable: Vec<&str> = lines
            .iter()
            .filter(|l| l.selectable)
            .map(|l| l.branch.as_str())
            .collect();
        assert_eq!(pickable, vec!["main", "alpha", "gamma"]);

        // Sanity-check the rendered shape: indents grow with depth and the
        // current branch carries the `(current)` marker.
        let by_branch: HashMap<&str, &str> = lines
            .iter()
            .map(|l| (l.branch.as_str(), l.text.as_str()))
            .collect();
        assert!(by_branch["main"].starts_with("\u{25cb} main"));
        assert!(by_branch["alpha"].starts_with("  \u{25cb} alpha"));
        assert!(by_branch["beta"].starts_with("    \u{25c9} beta"));
        assert!(by_branch["beta"].contains("(current)"));
        assert!(by_branch["delta"].starts_with("      \u{25cb} delta"));
        assert!(by_branch["gamma"].starts_with("    \u{25cb} gamma"));
    }
}
