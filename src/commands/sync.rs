//! `gt sync` — fetch, fast-forward the trunk, drop merged/closed branches, and
//! restack whatever survives.

use std::collections::HashSet;

use crate::error::Result;
use crate::graph::StackGraph;
use crate::{gh, git, meta, prompt, rebase};

pub fn run() -> Result<()> {
    git::ensure_remote()?;
    let original = git::current_branch()?;

    println!("fetching...");
    let fetched = git::run_allow_fail(&["fetch", "--prune"])?;
    if fetched.code != 0 {
        println!("warning: `git fetch` failed; continuing with local state");
    }

    let trunk = StackGraph::load()?.trunk;
    fast_forward_trunk(&trunk)?;

    // Reload after the trunk moved.
    let graph = StackGraph::load()?;
    let mut rollback_branches: Vec<String> =
        graph.tracked().into_iter().map(str::to_string).collect();
    rollback_branches.push(trunk.clone());
    let rollback_snapshot = rebase::snapshot_branches(&graph, &rollback_branches)?;

    // Find merged / closed branches and confirm each deletion.
    let mut to_delete: Vec<String> = Vec::new();
    let mut tracked: Vec<&str> = graph.tracked();
    tracked.sort();
    for b in tracked {
        if let Some(reason) = merged_reason(&graph, b)? {
            if prompt::confirm(&format!("delete `{b}` ({reason})?"), true)? {
                to_delete.push(b.to_string());
            }
        }
    }

    if to_delete.is_empty() {
        println!("no merged or closed branches to clean up");
    } else {
        delete_branches(&graph, &to_delete, &trunk)?;
    }

    // Return to a real branch before restacking survivors.
    let landing = match &original {
        Some(b) if git::branch_exists(b)? => b.clone(),
        _ => trunk.clone(),
    };
    git::run(&["switch", "--", &landing])?;

    let survivors = StackGraph::load()?;
    let roots: Vec<String> = survivors
        .get(&trunk)
        .map(|n| n.children.clone())
        .unwrap_or_default();
    if roots.is_empty() {
        println!("nothing left to restack");
        return Ok(());
    }
    let mut plan = rebase::plan(&survivors, &roots, "sync")?;
    if plan.chains.is_empty() {
        println!("the stack is already up to date");
        return Ok(());
    }
    plan.snapshot = rollback_snapshot;
    rebase::start(&survivors, plan)
}

/// Fast-forward the trunk branch to its remote tip, if possible.
fn fast_forward_trunk(trunk: &str) -> Result<()> {
    let remote_ref = format!("refs/remotes/origin/{trunk}");
    if git::rev_parse_opt(&remote_ref)?.is_none() {
        println!("no `{remote_ref}`; skipping trunk update");
        return Ok(());
    }
    let before = git::current_branch()?;
    git::run(&["switch", "--", trunk])?;
    let res = git::run_allow_fail(&["merge", "--ff-only", &remote_ref])?;
    if res.code == 0 {
        println!("updated `{trunk}`");
    } else {
        println!("warning: `{trunk}` could not be fast-forwarded (it diverged from {remote_ref})");
    }
    if let Some(b) = before {
        if b != trunk {
            let _ = git::run(&["switch", "--", &b]);
        }
    }
    Ok(())
}

/// Why a branch should be cleaned up, if it should.
fn merged_reason(graph: &StackGraph, branch: &str) -> Result<Option<String>> {
    let node = graph.get(branch).unwrap();

    // Primary signal: the pull request's state on GitHub.
    if let Some(num) = node
        .meta
        .as_ref()
        .and_then(|m| m.pr_info.as_ref())
        .and_then(|p| p.number)
    {
        if let Some(v) = gh::view(branch)? {
            match v.state.as_str() {
                "MERGED" => return Ok(Some(format!("PR #{num} merged"))),
                "CLOSED" => return Ok(Some(format!("PR #{num} closed"))),
                _ => {}
            }
        }
    }

    // Fallback: every commit is already present on the trunk (patch-id match).
    let trunk_ref = git::head_ref(&graph.trunk);
    let branch_ref = git::head_ref(branch);
    let cherry = git::run_allow_fail(&["cherry", &trunk_ref, &branch_ref])?;
    if cherry.code == 0 && !cherry.stdout.lines().any(|l| l.starts_with('+')) {
        return Ok(Some("commits already on trunk".into()));
    }
    Ok(None)
}

/// Delete the confirmed branches, reparenting any surviving children.
fn delete_branches(graph: &StackGraph, to_delete: &[String], trunk: &str) -> Result<()> {
    let del: HashSet<&str> = to_delete.iter().map(String::as_str).collect();

    // Reparent each surviving child onto the nearest non-deleted ancestor.
    for b in to_delete {
        let node = graph.get(b).unwrap();
        let mut ancestor = node.parent.clone().unwrap();
        while ancestor != *trunk && del.contains(ancestor.as_str()) {
            ancestor = graph
                .get(&ancestor)
                .and_then(|n| n.parent.clone())
                .unwrap_or_else(|| trunk.to_string());
        }
        for child in &node.children {
            if del.contains(child.as_str()) {
                continue;
            }
            if let Some(mut cm) = graph.get(child).and_then(|n| n.meta.clone()) {
                cm.parent_branch_name = Some(ancestor.clone());
                meta::write(child, &cm)?;
            }
        }
    }

    // Never delete the checked-out branch.
    if let Some(cur) = git::current_branch()? {
        if del.contains(cur.as_str()) {
            git::run(&["switch", "--", trunk])?;
        }
    }

    for b in to_delete {
        let Some(node) = graph.get(b) else { continue };
        // Keep the deleted branch recoverable: a backup ref under
        // refs/oh-my-gt/deleted/ holds its tip and survives `git gc`.
        let _ = git::run(&[
            "update-ref",
            &format!("refs/oh-my-gt/deleted/{b}"),
            &node.tip,
        ]);
        git::run(&["branch", "-D", "--", b])?;
        meta::delete(b)?;
        println!("deleted `{b}` (restore: git branch -- {b} {})", node.tip);
    }
    Ok(())
}
