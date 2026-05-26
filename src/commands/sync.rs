//! `gt sync` — fetch, fast-forward the trunk, drop merged/closed branches, and
//! restack whatever survives.

use std::collections::HashSet;

use crate::error::Result;
use crate::graph::StackGraph;
use crate::style::OutputStyle;
use crate::{gh, git, meta, prompt, rebase};

pub fn run() -> Result<()> {
    git::ensure_remote()?;
    let original = git::current_branch()?;
    let style = OutputStyle::stdout();

    println!("{}", style.status("fetching..."));
    let fetched = git::run_allow_fail(&["fetch", "--prune"])?;
    if fetched.code != 0 {
        println!(
            "{} `git fetch` failed; continuing with local state",
            style.warning("warning:")
        );
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
    // Sync uses per-branch chains so a conflict on one branch does not
    // discard work already done on an earlier branch in the same chain.
    rebase::split_chains_per_branch(&survivors, &mut plan);
    let outcome = rebase::start_best_effort(&survivors, plan)?;
    print_outcome(&outcome, &style);
    Ok(())
}

/// Summarize a best-effort restack: which branches moved, which were left
/// outdated and need manual attention.
fn print_outcome(outcome: &rebase::BestEffortOutcome, style: &OutputStyle) {
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
    if !outcome.outdated.is_empty() {
        let names = outcome
            .outdated
            .iter()
            .map(|b| style.branch(b).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{} {} (run `gt restack` to retry)",
            style.warning("outdated:"),
            names
        );
    }
    if outcome.restacked.is_empty() && outcome.outdated.is_empty() {
        println!("the stack is already up to date");
    }
}

/// Fast-forward the trunk branch to its remote tip, if possible.
///
/// Trunk may legitimately be checked out in a different worktree (e.g. the
/// primary worktree while the user is working from a feature worktree). In
/// that case `git switch` refuses, so we route the fast-forward through the
/// owning worktree — and only after it is clean, so we never silently dirty
/// someone else's working tree. When trunk is not checked out anywhere, we
/// fast-forward the ref directly with `update-ref`; that is safe precisely
/// because no working tree can be affected.
fn fast_forward_trunk(trunk: &str) -> Result<()> {
    let remote_ref = format!("refs/remotes/origin/{trunk}");
    let style = OutputStyle::stdout();
    let Some(remote_sha) = git::rev_parse_opt(&remote_ref)? else {
        println!("no `{remote_ref}`; skipping trunk update");
        return Ok(());
    };

    let local_ref = git::head_ref(trunk);
    let local_sha = git::rev_parse_opt(&local_ref)?
        .ok_or_else(|| crate::error::GtError::Git(format!("branch `{trunk}` does not exist")))?;
    if local_sha == remote_sha {
        return Ok(());
    }
    // Diverged trunk: never rewrite history, just warn (matches prior behavior).
    if !git::is_ancestor(&local_sha, &remote_sha)? {
        println!(
            "{} `{trunk}` could not be fast-forwarded (it diverged from {remote_ref})",
            style.warning("warning:")
        );
        return Ok(());
    }

    let owner = git::worktree_owner_of(trunk)?;
    match owner {
        // Case A: trunk is checked out in the current worktree. Use the
        // ordinary `merge --ff-only` flow.
        Some(path) if is_current_worktree(&path)? => {
            let before = git::current_branch()?;
            git::run(&["switch", "--", trunk])?;
            git::run(&["merge", "--ff-only", &remote_ref])?;
            println!("updated `{trunk}`");
            if let Some(b) = before {
                if b != trunk {
                    let _ = git::run(&["switch", "--", &b]);
                }
            }
        }
        // Case B: trunk is checked out elsewhere. Fast-forward inside the
        // owning worktree, but only if it is clean — otherwise we would be
        // dirtying someone else's tree, which the no-data-loss rule forbids.
        Some(path) => {
            if !git::is_clean_in(&path)? {
                return Err(crate::error::GtError::Precondition(format!(
                    "trunk `{trunk}` is checked out at {} with uncommitted changes; \
                     commit or stash them before running `gt sync`",
                    path.display()
                )));
            }
            let path_str = path.to_str().ok_or_else(|| {
                crate::error::GtError::Git(format!("non-UTF8 worktree path: {path:?}"))
            })?;
            git::run(&["-C", path_str, "merge", "--ff-only", &remote_ref])?;
            println!("updated `{trunk}` in {}", path.display());
        }
        // Case C: trunk is not checked out anywhere. Safe to advance the ref
        // directly — no working tree can be affected. We already verified
        // ancestry above, so this is a true fast-forward.
        None => {
            git::update_ref(&local_ref, &remote_sha)?;
            println!("updated `{trunk}`");
        }
    }
    Ok(())
}

/// Whether `path` is the same worktree as the current process's cwd.
fn is_current_worktree(path: &std::path::Path) -> Result<bool> {
    let here = git::run(&["rev-parse", "--show-toplevel"])?;
    let here = std::path::PathBuf::from(here);
    Ok(canonical(&here) == canonical(path))
}

fn canonical(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
