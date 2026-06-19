//! The rebase engine.
//!
//! A restack is planned as a list of linear *chains*; each chain is rebased by
//! a single `git rebase --update-refs --onto` call, which moves every
//! intermediate branch ref in one pass. Chains are processed parent-first, so
//! by the time a chain runs its parent's tip is already final.
//!
//! Conflicts pause the operation: state is persisted to `.git/oh-my-gt/` so
//! `gt continue` resumes mid-plan and `gt abort` rolls every branch back. In
//! best-effort mode (`gt sync` / `gt restack`) only a conflict on the path to
//! the user's checked-out branch pauses — and only after every other chain
//! has been attempted; conflicts elsewhere just report the branch.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::error::{GtError, Result};
use crate::graph::{StackGraph, Validation};
use crate::state::{self, BranchSnapshot, Chain, OpState};
use crate::style::OutputStyle;
use crate::{git, meta};

/// A planned restack: the chains to rebase and a rollback snapshot.
pub struct RestackPlan {
    pub operation: String,
    pub chains: Vec<Chain>,
    pub snapshot: Vec<BranchSnapshot>,
}

/// Up-front partition of branches into "worth attempting" and "stale".
///
/// `gt sync` and `gt restack` use this to avoid pulling unrelated stale
/// branches into the rebase. See [`select_branches_for_clean_restack`].
pub struct CleanSelection {
    /// Branches eligible to be attempted in this restack pass.
    pub clean: HashSet<String>,
    /// Branches deliberately skipped up-front: their recorded fork point is
    /// no longer in their parent's history, so a clean rebase is unlikely.
    /// Stable order, suitable for user-facing output.
    pub stale: Vec<String>,
    /// Branches on the path from trunk to the current branch (inclusive) —
    /// the user's active line of work. Only a conflict on one of these pauses
    /// for manual resolution (see [`start_best_effort`]).
    pub on_path: HashSet<String>,
}

impl CleanSelection {
    /// Print the up-front skip notice for stale branches, if any.
    pub fn print_stale(&self, style: &OutputStyle) {
        if self.stale.is_empty() {
            return;
        }
        let names = self
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
}

/// Partition tracked branches into a clean (attempt) set and a stale (skip)
/// set, with the current branch and everything on its path from trunk always
/// in the clean set.
///
/// Heuristic:
/// * `current` (if tracked) and every branch on the path from trunk to
///   `current` are always included — the user is actively working there and
///   may legitimately want to absorb a rewrite, even if it conflicts.
/// * Every other tracked branch is included only if its recorded
///   `parent_branch_revision` is still an ancestor of its parent's current
///   tip. That is the "parent advanced cleanly" case where `git rebase
///   --onto parent.tip recorded-rev` has a straightforward upstream.
/// * Tracked branches whose recorded fork point is NOT an ancestor of their
///   parent's tip are stale: the parent was rewritten under them, so a
///   replay is likely to conflict. They are reported separately and left
///   untouched.
pub fn select_branches_for_clean_restack(graph: &StackGraph, current: &str) -> CleanSelection {
    // Path from trunk to current (inclusive). These branches are always
    // attempted, regardless of whether their fork point looks fresh.
    let on_path: HashSet<String> = if graph.get(current).is_some() {
        graph.path_from_trunk(current).into_iter().collect()
    } else {
        HashSet::new()
    };

    let mut clean: HashSet<String> = HashSet::new();
    let mut stale_set: HashSet<String> = HashSet::new();

    for name in graph.tracked() {
        if on_path.contains(name) {
            clean.insert(name.to_string());
            continue;
        }
        if parent_revision_is_in_history(graph, name) {
            clean.insert(name.to_string());
        } else {
            stale_set.insert(name.to_string());
        }
    }

    // Always include the current branch itself if it is a tracked, non-trunk
    // branch (covered by `on_path` above, but guard against an untracked
    // current — in which case there's nothing to add).
    if let Some(node) = graph.get(current) {
        if !graph.is_trunk(current) && node.validation == Validation::Valid {
            clean.insert(current.to_string());
            stale_set.remove(current);
        }
    }

    // Deterministic order for the user-facing list: lexical, matching the
    // way other commands render branch names.
    let mut stale: Vec<String> = stale_set.into_iter().collect();
    stale.sort();
    CleanSelection {
        clean,
        stale,
        on_path,
    }
}

/// Whether `branch`'s recorded `parent_branch_revision` is reachable from its
/// parent's current tip. A tracked-but-unrecorded fork point counts as stale.
fn parent_revision_is_in_history(graph: &StackGraph, branch: &str) -> bool {
    let Some(node) = graph.get(branch) else {
        return false;
    };
    if node.validation != Validation::Valid {
        return false;
    }
    let parent = match node.parent.as_deref() {
        Some(p) => p,
        None => return false,
    };
    let recorded = match node
        .meta
        .as_ref()
        .and_then(|m| m.parent_branch_revision.as_deref())
    {
        Some(r) => r,
        None => return false,
    };
    let parent_tip = match graph.get(parent) {
        Some(p) => p.tip.as_str(),
        None => return false,
    };
    // The simplest case: the parent hasn't moved at all since this branch
    // was stacked. No need to spawn git.
    if recorded == parent_tip {
        return true;
    }
    git::is_ancestor(recorded, parent_tip).unwrap_or(false)
}

/// Split each chain into one chain per branch.
///
/// `gt sync` uses this so that a conflict on one branch leaves earlier
/// branches in the same chain restacked cleanly. The trade-off — one `git
/// rebase` invocation per branch instead of one per chain — is acceptable for
/// sync (which is interactive and uncommon).
pub fn split_chains_per_branch(graph: &StackGraph, plan: &mut RestackPlan) {
    let mut out: Vec<Chain> = Vec::new();
    for chain in plan.chains.drain(..) {
        for (i, branch) in chain.branches.iter().enumerate() {
            // For the first sub-chain, the parent and old_base match the
            // original chain. For later sub-chains, the parent is the previous
            // branch in the chain and the old_base is that branch's pre-rebase
            // tip — which is exactly what the next branch's metadata records
            // as its parent revision.
            let (parent, old_base) = if i == 0 {
                (chain.parent.clone(), chain.old_base.clone())
            } else {
                let prev = chain.branches[i - 1].clone();
                let recorded = graph
                    .get(branch)
                    .and_then(|n| n.meta.as_ref())
                    .and_then(|m| m.parent_branch_revision.clone())
                    .expect("a chain branch is always tracked with a fork point");
                (prev, recorded)
            };
            out.push(Chain {
                parent,
                old_base,
                branches: vec![branch.clone()],
                done: false,
                deferred: false,
            });
        }
    }
    plan.chains = out;
}

/// Plan a restack of the subtrees rooted at `roots`.
pub fn plan(graph: &StackGraph, roots: &[String], operation: &str) -> Result<RestackPlan> {
    plan_inner(graph, roots, operation, None)
}

/// Plan a restack of the subtrees rooted at `roots`, but only include
/// branches in `allow_set`. Branches outside the set are not added to any
/// chain — even if their parent is being rebased. Used by best-effort
/// callers (sync, restack) to skip stale branches up-front.
pub fn plan_clean(
    graph: &StackGraph,
    roots: &[String],
    operation: &str,
    allow_set: &HashSet<String>,
) -> Result<RestackPlan> {
    plan_inner(graph, roots, operation, Some(allow_set))
}

fn plan_inner(
    graph: &StackGraph,
    roots: &[String],
    operation: &str,
    allow_set: Option<&HashSet<String>>,
) -> Result<RestackPlan> {
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
    // scope is being rebased (its tip is about to move). With an allow-set,
    // a branch is only considered if it appears in the set.
    let mut to_rebase: HashSet<String> = HashSet::new();
    let mut heads_order: Vec<String> = Vec::new();
    for b in &ordered {
        let Some(node) = graph.get(b) else { continue };
        if node.validation != Validation::Valid {
            continue;
        }
        if let Some(allow) = allow_set {
            if !allow.contains(b) {
                continue;
            }
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
            deferred: false,
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
    let st = prepare(graph, plan)?;
    state::save(&st)?;
    drive(st)
}

/// Like [`start`], but in best-effort mode (used by `gt sync` and
/// `gt restack`): every chain is attempted, and only a conflict on a branch
/// in `pause_on` — the path to the user's checked-out branch — pauses for
/// manual resolution, after every cleanly-rebaseable chain has been done.
/// A conflict anywhere else just reports the branch and moves on.
pub fn start_best_effort(
    graph: &StackGraph,
    plan: RestackPlan,
    pause_on: &HashSet<String>,
) -> Result<()> {
    let mut st = prepare(graph, plan)?;
    st.best_effort = true;
    st.pause_branches = pause_on.iter().cloned().collect();
    st.pause_branches.sort();
    state::save(&st)?;
    drive_best_effort(st)
}

/// Common preflight for [`start`] and [`start_best_effort`].
fn prepare(graph: &StackGraph, plan: RestackPlan) -> Result<OpState> {
    let current = graph.current.clone().ok_or_else(|| {
        GtError::Precondition("HEAD is detached; check out a branch first".into())
    })?;
    git::ensure_clean()?;
    if git::rebase_in_progress(&git::git_dir()?) {
        return Err(GtError::Precondition(
            "a git rebase is already in progress".into(),
        ));
    }
    Ok(OpState {
        version: 1,
        operation: plan.operation,
        trunk: graph.trunk.clone(),
        return_branch: Some(current),
        snapshot: plan.snapshot,
        chains: plan.chains,
        current_chain: 0,
        best_effort: false,
        pause_branches: Vec::new(),
    })
}

/// Resume an operation paused on conflicts (`gt continue`).
pub fn resume() -> Result<()> {
    let mut st = state::load()?
        .ok_or_else(|| GtError::Usage("no operation in progress to continue".into()))?;

    let gd = git::git_dir()?;
    let style = OutputStyle::stdout();
    if git::rebase_in_progress(&gd) {
        if !git::conflicted_files()?.is_empty() {
            return Err(GtError::Precondition(
                "unresolved conflicts remain; resolve them and `git add` the files".into(),
            ));
        }
        let code = git::run_interactive(&["rebase", "--continue"])?;
        if code != 0 {
            return handle_rebase_exit(&mut st, &gd, "");
        }
        let chain = st.chains[st.current_chain].clone();
        finish_chain(&mut st)?;
        for b in &chain.branches {
            println!("restacked {}", style.branch(b));
        }
    } else {
        // gt has a paused restack recorded, but git is not mid-rebase: the
        // rebase gt started was finished or aborted out of band with native
        // `git rebase` commands. Re-driving the plan from the current branch
        // tips reconciles either case — a chain the user already replayed
        // becomes a no-op (its commits are dropped as empty), and one they
        // undid simply runs again. But only when the tree is clean: a stray
        // change would otherwise be lost to, or block, the replay.
        if git::is_dirty()? {
            return Err(GtError::Precondition(
                "the rebase gt paused is no longer in progress, and the working tree has \
                 uncommitted changes. Commit or stash them, then run `gt continue` — or run \
                 `gt abort` to discard the in-progress restack."
                    .into(),
            ));
        }
        println!(
            "{} the git rebase ended outside gt (with native `git rebase`); reconciling and \
             continuing the restack",
            style.warning("note:")
        );
    }
    if st.best_effort {
        drive_best_effort(st)
    } else {
        drive(st)
    }
}

/// Abort an in-progress operation (`gt abort`), restoring the prior state.
pub fn abort() -> Result<()> {
    let st =
        state::load()?.ok_or_else(|| GtError::Usage("no operation in progress to abort".into()))?;

    let style = OutputStyle::stdout();
    if git::rebase_in_progress(&git::git_dir()?) {
        git::run(&["rebase", "--abort"])?;
    } else {
        // The rebase gt paused is already gone — finished or aborted out of
        // band with native `git rebase`. We still roll the recorded branches
        // back to their pre-operation tips (that is what `abort` promises),
        // but say so: if the user resolved the rebase by hand, this rewinds
        // that result. The rewound commits stay reachable through git's reflog,
        // so this is recoverable, not destructive.
        println!(
            "{} no git rebase is in progress (it was ended outside gt); restoring the \
             pre-operation state anyway",
            style.warning("note:")
        );
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

/// Best-effort variant of [`drive`], used by `gt sync` and `gt restack`.
///
/// Scans the chains repeatedly until nothing is left, printing one status
/// line per branch:
/// * a clean rebase completes the chain;
/// * a conflict off the user's path reports the branch (check it out and
///   restack manually) and gives up on everything stacked on it;
/// * a conflict on the user's path is deferred so every cleanly-rebaseable
///   chain finishes first; when retried it pauses for manual resolution
///   (`gt continue` / `gt abort`), exactly like [`drive`].
fn drive_best_effort(mut st: OpState) -> Result<()> {
    let style = OutputStyle::stdout();
    loop {
        let mut all_done = true;
        for idx in 0..st.chains.len() {
            if st.chains[idx].done {
                continue;
            }
            // A deferred ancestor means this chain's base is not final yet;
            // leave it for a later scan.
            let parent = st.chains[idx].parent.clone();
            let parent_pending = st
                .chains
                .iter()
                .enumerate()
                .any(|(j, c)| j != idx && !c.done && c.branches.contains(&parent));
            if parent_pending {
                all_done = false;
                continue;
            }

            st.current_chain = idx;
            let chain = st.chains[idx].clone();
            let new_base = git::branch_tip(&chain.parent)?;
            let out = git::run_allow_fail(&[
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
            if out.code == 0 {
                finish_chain(&mut st)?;
                for b in &chain.branches {
                    println!("restacked {}", style.branch(b));
                }
                continue;
            }

            let on_path = chain.branches.iter().any(|b| st.pause_branches.contains(b));
            if on_path && chain.deferred {
                // Second attempt: everything else is done, pause here.
                let gd = git::git_dir()?;
                return handle_rebase_exit(&mut st, &gd, &out.stderr);
            }
            if git::rebase_in_progress(&git::git_dir()?) {
                let _ = git::run(&["rebase", "--abort"]);
            }
            if on_path {
                st.chains[idx].deferred = true;
                all_done = false;
            } else {
                give_up_on_chain(&mut st, idx, &style);
            }
            state::save(&st)?;
        }
        if all_done {
            break;
        }
    }

    state::clear()?;
    if let Some(rb) = &st.return_branch {
        git::switch_if_exists(rb)?;
    }
    Ok(())
}

/// Report a chain that cannot be restacked automatically and mark it — plus
/// every chain stacked on it — as done so it is never retried this run.
fn give_up_on_chain(st: &mut OpState, idx: usize, style: &OutputStyle) {
    let mut given_up: HashSet<String> = HashSet::new();
    for b in &st.chains[idx].branches {
        given_up.insert(b.clone());
        println!(
            "{} {} cannot be restacked automatically; check it out and run `gt restack` to \
             resolve manually",
            style.warning("conflict:"),
            style.branch(b)
        );
    }
    st.chains[idx].done = true;
    // Chains are planned parent-first, so a single forward sweep reaches
    // every chain transitively stacked on the conflicted one.
    for chain in st.chains.iter_mut().skip(idx + 1) {
        if chain.done || !given_up.contains(&chain.parent) {
            continue;
        }
        for b in &chain.branches {
            given_up.insert(b.clone());
            println!(
                "{} {} (parent not restacked)",
                style.warning("skipped:"),
                style.branch(b)
            );
        }
        chain.done = true;
    }
}

/// Run chains from `current_chain` onward, printing one status line per
/// restacked branch. The raw `git rebase` output is captured, not shown.
fn drive(mut st: OpState) -> Result<()> {
    let style = OutputStyle::stdout();
    while st.current_chain < st.chains.len() {
        if st.chains[st.current_chain].done {
            st.current_chain += 1;
            continue;
        }
        let chain = st.chains[st.current_chain].clone();
        // The parent's tip is final now (its chain ran earlier).
        let new_base = git::branch_tip(&chain.parent)?;
        let out = git::run_allow_fail(&[
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
        if out.code != 0 {
            let gd = git::git_dir()?;
            return handle_rebase_exit(&mut st, &gd, &out.stderr);
        }
        finish_chain(&mut st)?;
        for b in &chain.branches {
            println!("restacked {}", style.branch(b));
        }
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
/// `detail` is the captured stderr, surfaced on an unexpected failure.
fn handle_rebase_exit(st: &mut OpState, git_dir: &Path, detail: &str) -> Result<()> {
    if !git::rebase_in_progress(git_dir) {
        let msg = if detail.is_empty() {
            "git rebase failed unexpectedly".to_string()
        } else {
            format!("git rebase failed unexpectedly:\n{detail}")
        };
        return Err(GtError::Git(msg));
    }
    state::save(st)?; // current_chain still points at the failing chain
    let chain = &st.chains[st.current_chain];
    let style = OutputStyle::stdout();
    println!();
    println!(
        "{} `{}` cannot be restacked automatically",
        style.warning("conflict:"),
        chain.tip()
    );
    let files = git::conflicted_files().unwrap_or_default();
    if !files.is_empty() {
        println!("conflicted files:");
        for f in &files {
            println!("  {f}");
        }
    }
    println!("resolve the conflicts manually, `git add` them, then run `gt continue`");
    println!("or run `gt abort` to undo the rebase and restore the previous state");
    println!(
        "{} drive this with gt, not `git rebase --continue` / `git rebase --abort` — \
         resolving the rebase with git directly leaves gt out of sync",
        style.warning("note:")
    );
    Err(GtError::Paused)
}

/// Finish a fully-applied operation. Each branch already got its own
/// `restacked` line, so there is nothing left to report.
fn complete(st: OpState) -> Result<()> {
    state::clear()?;
    if let Some(rb) = &st.return_branch {
        git::switch_if_exists(rb)?;
    }
    Ok(())
}
