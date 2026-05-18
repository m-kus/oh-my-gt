//! The rebase engine.
//!
//! A restack is planned as a list of linear *chains*; each chain is rebased by
//! a single `git rebase --update-refs --onto` call, which moves every
//! intermediate branch ref in one pass. Chains are processed parent-first, so
//! by the time a chain runs its parent's tip is already final.
//!
//! Conflicts pause the operation: state is persisted to `.git/oh-my-gt/` so
//! `gt continue` resumes mid-plan and `gt abort` rolls every branch back.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::error::{GtError, Result};
use crate::graph::{StackGraph, Validation};
use crate::state::{self, BranchSnapshot, Chain, OpState};
use crate::{git, meta};

/// A planned restack: the chains to rebase and a rollback snapshot.
pub struct RestackPlan {
    pub operation: String,
    pub chains: Vec<Chain>,
    pub snapshot: Vec<BranchSnapshot>,
}

/// Plan a restack of the subtrees rooted at `roots`.
pub fn plan(graph: &StackGraph, roots: &[String], operation: &str) -> Result<RestackPlan> {
    // Scope: every branch at or below a root (the trunk never moves).
    let mut scope: BTreeSet<String> = BTreeSet::new();
    for r in roots {
        for b in graph.subtree(r) {
            scope.insert(b);
        }
    }
    scope.remove(&graph.trunk);

    // Process parents before children.
    let mut ordered: Vec<String> = scope.iter().cloned().collect();
    ordered.sort_by_key(|b| graph.path_from_trunk(b).len());

    // A branch is rebased if its own fork point is stale, or an ancestor in
    // scope is being rebased (its tip is about to move).
    let mut to_rebase: HashSet<String> = HashSet::new();
    let mut heads_order: Vec<String> = Vec::new();
    for b in &ordered {
        let Some(node) = graph.get(b) else { continue };
        if node.validation != Validation::Valid {
            continue;
        }
        let parent = node.parent.clone().unwrap();
        if graph.needs_restack(b) || to_rebase.contains(&parent) {
            to_rebase.insert(b.clone());
            heads_order.push(b.clone());
        }
    }

    // Decompose into maximal linear chains. A chain extends through a branch
    // while that branch has exactly one rebased child; a branch point ends it.
    let mut chains: Vec<Chain> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    for head in &heads_order {
        if visited.contains(head) {
            continue;
        }
        let mut branches = vec![head.clone()];
        visited.insert(head.clone());
        let mut cur = head.clone();
        loop {
            let rebased_children: Vec<String> = graph
                .get(&cur)
                .unwrap()
                .children
                .iter()
                .filter(|c| to_rebase.contains(*c))
                .cloned()
                .collect();
            if rebased_children.len() == 1 {
                cur = rebased_children.into_iter().next().unwrap();
                branches.push(cur.clone());
                visited.insert(cur.clone());
            } else {
                break;
            }
        }
        let head_node = graph.get(head).unwrap();
        chains.push(Chain {
            parent: head_node.parent.clone().unwrap(),
            old_base: head_node
                .meta
                .as_ref()
                .unwrap()
                .parent_branch_revision
                .clone()
                .unwrap(),
            branches,
            done: false,
        });
    }

    // Snapshot every branch a chain will touch, for `abort`.
    let mut snap_names: Vec<String> = Vec::new();
    for c in &chains {
        for b in &c.branches {
            snap_names.push(b.clone());
        }
    }
    let snapshot = snapshot_branches(graph, &snap_names)?;

    Ok(RestackPlan {
        operation: operation.to_string(),
        chains,
        snapshot,
    })
}

/// Snapshot branch refs and metadata for later rollback.
pub fn snapshot_branches(graph: &StackGraph, branches: &[String]) -> Result<Vec<BranchSnapshot>> {
    let mut snap_names: BTreeSet<String> = BTreeSet::new();
    for b in branches {
        snap_names.insert(b.clone());
    }

    let mut snapshot = Vec::new();
    for b in snap_names {
        let Some(node) = graph.get(&b) else {
            continue;
        };
        snapshot.push(BranchSnapshot {
            tip: node.tip.clone(),
            metadata_blob: meta::blob_sha(&b)?,
            branch: b,
        });
    }
    Ok(snapshot)
}

/// Load the graph, plan a restack of `roots`, and execute it.
pub fn restack(roots: &[String], operation: &str) -> Result<()> {
    let graph = StackGraph::load()?;
    let plan = plan(&graph, roots, operation)?;
    if plan.chains.is_empty() {
        println!("the stack is already up to date");
        return Ok(());
    }
    start(&graph, plan)
}

/// Begin executing a freshly-built plan.
pub fn start(graph: &StackGraph, plan: RestackPlan) -> Result<()> {
    let current = graph.current.clone().ok_or_else(|| {
        GtError::Precondition("HEAD is detached; check out a branch first".into())
    })?;
    git::ensure_clean()?;
    if git::rebase_in_progress(&git::git_dir()?) {
        return Err(GtError::Precondition(
            "a git rebase is already in progress".into(),
        ));
    }

    let st = OpState {
        version: 1,
        operation: plan.operation,
        trunk: graph.trunk.clone(),
        return_branch: Some(current),
        snapshot: plan.snapshot,
        chains: plan.chains,
        current_chain: 0,
    };
    state::save(&st)?;
    drive(st)
}

/// Resume an operation paused on conflicts (`gt continue`).
pub fn resume() -> Result<()> {
    let mut st = state::load()?
        .ok_or_else(|| GtError::Usage("no operation in progress to continue".into()))?;

    let gd = git::git_dir()?;
    if git::rebase_in_progress(&gd) {
        if !git::conflicted_files()?.is_empty() {
            return Err(GtError::Precondition(
                "unresolved conflicts remain; resolve them and `git add` the files".into(),
            ));
        }
        let code = git::run_interactive(&["rebase", "--continue"])?;
        if code != 0 {
            return handle_rebase_exit(&mut st, &gd);
        }
        finish_chain(&mut st)?;
    }
    drive(st)
}

/// Abort an in-progress operation (`gt abort`), restoring the prior state.
pub fn abort() -> Result<()> {
    let st =
        state::load()?.ok_or_else(|| GtError::Usage("no operation in progress to abort".into()))?;

    if git::rebase_in_progress(&git::git_dir()?) {
        git::run(&["rebase", "--abort"])?;
    }

    // Roll every touched branch back to its pre-operation tip and metadata.
    for s in &st.snapshot {
        git::run(&["update-ref", &git::head_ref(&s.branch), &s.tip])?;
        match &s.metadata_blob {
            Some(blob) => meta::restore_ref(&s.branch, blob)?,
            None => meta::delete(&s.branch)?,
        }
    }

    state::clear()?;
    println!("aborted; the stack was restored to its previous state");

    // Best-effort return to the original branch. A plain (non-force) switch is
    // used so it can never discard uncommitted work.
    if let Some(rb) = &st.return_branch {
        if git::branch_exists(rb)? {
            let _ = git::run(&["switch", "--", rb]);
        }
    }
    Ok(())
}

// ── internals ───────────────────────────────────────────────────────────────

/// Run chains from `current_chain` onward.
fn drive(mut st: OpState) -> Result<()> {
    while st.current_chain < st.chains.len() {
        if st.chains[st.current_chain].done {
            st.current_chain += 1;
            continue;
        }
        let chain = st.chains[st.current_chain].clone();
        // The parent's tip is final now (its chain ran earlier).
        let new_base = git::branch_tip(&chain.parent)?;
        let code = git::run_interactive(&[
            "rebase",
            "--update-refs",
            "--committer-date-is-author-date",
            "--empty=drop",
            "--onto",
            &new_base,
            &chain.old_base,
            "--",
            chain.tip(),
        ])?;
        if code != 0 {
            let gd = git::git_dir()?;
            return handle_rebase_exit(&mut st, &gd);
        }
        finish_chain(&mut st)?;
    }
    complete(st)
}

/// Refresh metadata for the just-rebased chain and advance the cursor.
fn finish_chain(st: &mut OpState) -> Result<()> {
    let idx = st.current_chain;
    let chain = st.chains[idx].clone();
    for (i, branch) in chain.branches.iter().enumerate() {
        let parent = if i == 0 {
            &chain.parent
        } else {
            &chain.branches[i - 1]
        };
        let parent_tip = git::branch_tip(parent)?;
        let mut m = meta::read(branch)?.unwrap_or_default();
        m.parent_branch_name = Some(parent.clone());
        m.parent_branch_revision = Some(parent_tip);
        meta::write(branch, &m)?;
    }
    st.chains[idx].done = true;
    st.current_chain = idx + 1;
    state::save(st)?;
    Ok(())
}

/// React to a non-zero `git rebase` exit: pause on a conflict, else error.
fn handle_rebase_exit(st: &mut OpState, git_dir: &Path) -> Result<()> {
    if !git::rebase_in_progress(git_dir) {
        return Err(GtError::Git("git rebase failed unexpectedly".into()));
    }
    state::save(st)?; // current_chain still points at the failing chain
    let chain = &st.chains[st.current_chain];
    println!();
    println!("\x1b[33mCONFLICT\x1b[0m while restacking `{}`", chain.tip());
    let files = git::conflicted_files().unwrap_or_default();
    if !files.is_empty() {
        println!("conflicted files:");
        for f in &files {
            println!("  {f}");
        }
    }
    println!("resolve the conflicts, `git add` them, then run `gt continue`");
    println!("or run `gt abort` to undo and restore the previous state");
    Err(GtError::Paused)
}

/// Finish a fully-applied operation.
fn complete(st: OpState) -> Result<()> {
    state::clear()?;
    if let Some(rb) = &st.return_branch {
        git::switch_if_exists(rb)?;
    }
    let n: usize = st.chains.iter().map(|c| c.branches.len()).sum();
    println!("restacked {n} branch{}", if n == 1 { "" } else { "es" });
    Ok(())
}
