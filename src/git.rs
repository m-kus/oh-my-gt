//! The single chokepoint for talking to the `git` binary.
//!
//! Every `Command::new("git")` in the crate lives here. Higher layers call the
//! typed helpers and never construct git command lines themselves.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{GtError, Result};

/// Result of a git invocation that is allowed to exit non-zero.
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// Fully-qualified local branch ref.
pub fn head_ref(name: &str) -> String {
    format!("refs/heads/{name}")
}

/// Push refspec for updating exactly one branch with the same local/remote name.
pub fn head_refspec(name: &str) -> String {
    let r = head_ref(name);
    format!("{r}:{r}")
}

/// Run `git <args>`; error if the exit status is non-zero.
pub fn run(args: &[&str]) -> Result<String> {
    let out = run_allow_fail(args)?;
    if out.code != 0 {
        let detail = if out.stderr.is_empty() {
            &out.stdout
        } else {
            &out.stderr
        };
        return Err(GtError::Git(format!(
            "git {} failed:\n{detail}",
            args.join(" ")
        )));
    }
    Ok(out.stdout)
}

/// Run `git <args>`, capturing output and never erroring on a non-zero status.
pub fn run_allow_fail(args: &[&str]) -> Result<Output> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| GtError::Git(format!("failed to spawn git: {e}")))?;
    Ok(Output {
        stdout: String::from_utf8_lossy(&out.stdout)
            .trim_end_matches('\n')
            .to_string(),
        stderr: String::from_utf8_lossy(&out.stderr)
            .trim_end_matches('\n')
            .to_string(),
        code: out.status.code().unwrap_or(-1),
    })
}

/// Run `git <args>` feeding `input` on stdin; return raw stdout bytes.
pub fn run_with_stdin_raw(args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| GtError::Git(format!("failed to spawn git: {e}")))?;
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(input)
        .map_err(|e| GtError::Git(format!("failed writing git stdin: {e}")))?;
    let out = child
        .wait_with_output()
        .map_err(|e| GtError::Git(format!("git wait failed: {e}")))?;
    if !out.status.success() {
        return Err(GtError::Git(format!(
            "git {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(out.stdout)
}

/// Like [`run_with_stdin_raw`] but returns trimmed stdout as a string.
pub fn run_with_stdin(args: &[&str], input: &[u8]) -> Result<String> {
    let bytes = run_with_stdin_raw(args, input)?;
    Ok(String::from_utf8_lossy(&bytes)
        .trim_end_matches('\n')
        .to_string())
}

/// Run `git <args>` with inherited stdio (for rebase / editors). Returns the exit code.
pub fn run_interactive(args: &[&str]) -> Result<i32> {
    let status = Command::new("git")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| GtError::Git(format!("failed to spawn git: {e}")))?;
    Ok(status.code().unwrap_or(-1))
}

/// Ensure git is recent enough for `rebase --update-refs` (2.38+).
pub fn check_version() -> Result<()> {
    let out = run(&["--version"])?; // "git version 2.50.1"
    let ver = out
        .split_whitespace()
        .nth(2)
        .ok_or_else(|| GtError::Git(format!("cannot parse git version from `{out}`")))?;
    let mut parts = ver.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if (major, minor) < (2, 38) {
        return Err(GtError::Git(format!(
            "git >= 2.38 is required (found {ver}); needed for `git rebase --update-refs`"
        )));
    }
    Ok(())
}

/// Ensure we are inside a git work tree.
pub fn ensure_repo() -> Result<()> {
    let out = run_allow_fail(&["rev-parse", "--is-inside-work-tree"])?;
    if out.code != 0 || out.stdout != "true" {
        return Err(GtError::Git("not inside a git repository".into()));
    }
    Ok(())
}

/// Error out unless a remote is configured.
pub fn ensure_remote() -> Result<()> {
    if run(&["remote"])?.is_empty() {
        return Err(GtError::State("no git remote is configured".into()));
    }
    Ok(())
}

/// Error out unless the working tree is clean.
pub fn ensure_clean() -> Result<()> {
    if is_dirty()? {
        return Err(GtError::Precondition(
            "working tree has uncommitted changes; commit or stash them first".into(),
        ));
    }
    Ok(())
}

/// Absolute path of the `.git` directory (worktree-aware).
pub fn git_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(run(&["rev-parse", "--absolute-git-dir"])?))
}

/// Currently checked-out branch, or `None` if HEAD is detached.
pub fn current_branch() -> Result<Option<String>> {
    let out = run_allow_fail(&["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    Ok(if out.code == 0 {
        Some(out.stdout)
    } else {
        None
    })
}

fn verify(spec: &str) -> Result<Option<String>> {
    let out = run_allow_fail(&["rev-parse", "--verify", "--quiet", spec])?;
    Ok((out.code == 0 && !out.stdout.is_empty()).then_some(out.stdout))
}

/// Resolve a revision to a full commit SHA, or `None` if it does not exist.
pub fn rev_parse_opt(rev: &str) -> Result<Option<String>> {
    verify(&format!("{rev}^{{commit}}"))
}

/// Resolve a ref to the SHA it points at (e.g. a metadata blob), or `None`.
pub fn rev_parse_blob(reference: &str) -> Result<Option<String>> {
    verify(reference)
}

/// Whether a fully-qualified ref exists.
pub fn ref_exists(reference: &str) -> Result<bool> {
    Ok(run_allow_fail(&["show-ref", "--verify", "--quiet", reference])?.code == 0)
}

/// Whether a local branch exists.
pub fn branch_exists(name: &str) -> Result<bool> {
    ref_exists(&head_ref(name))
}

/// Tip SHA of a local branch.
pub fn branch_tip(name: &str) -> Result<String> {
    rev_parse_opt(&head_ref(name))?
        .ok_or_else(|| GtError::Git(format!("branch `{name}` does not exist")))
}

/// Tip SHA of the remote-tracking ref `refs/remotes/<remote>/<branch>`, or
/// `None` when it does not exist (the branch was never pushed/fetched). This is
/// the last position gt knows the remote held — the same ref `--force-with-lease`
/// leases against — so comparing a local tip to it tells us, without a network
/// round trip, whether a push would change anything.
pub fn remote_tracking_tip(remote: &str, branch: &str) -> Result<Option<String>> {
    rev_parse_opt(&format!("refs/remotes/{remote}/{branch}"))
}

/// Switch to `branch` if it exists; otherwise do nothing.
pub fn switch_if_exists(branch: &str) -> Result<()> {
    if branch_exists(branch)? {
        run(&["switch", "--", branch])?;
    }
    Ok(())
}

/// `git merge-base <a> <b>`.
pub fn merge_base(a: &str, b: &str) -> Result<String> {
    let a_ref = head_ref(a);
    let b_ref = head_ref(b);
    run(&["merge-base", &a_ref, &b_ref])
}

/// Whether `a` is an ancestor of `b`.
pub fn is_ancestor(a: &str, b: &str) -> Result<bool> {
    let out = run_allow_fail(&["merge-base", "--is-ancestor", a, b])?;
    Ok(out.code == 0)
}

/// Commit SHAs at the repository's shallow-clone boundaries — the contents of
/// `.git/shallow` — or an empty set when the clone is complete.
///
/// git presents a boundary commit as parentless: it hides the real parents even
/// when those objects are present locally (a stale graft left after the rest of
/// history was later fetched). Any ancestry query that crosses such a commit
/// silently returns the wrong answer, and a `rebase --update-refs` that replays
/// one emits a corrupt `update-ref grafted` todo line. gt reads this set so it
/// can flag and refuse those branches rather than trust git here.
pub fn shallow_boundaries() -> Result<HashSet<String>> {
    // `.git/shallow` lives in the common git dir, which linked worktrees share.
    let common = run(&["rev-parse", "--git-common-dir"])?;
    let mut path = PathBuf::from(&common);
    if path.is_relative() {
        path = std::env::current_dir()
            .map_err(|e| GtError::Git(format!("cannot resolve current dir: {e}")))?
            .join(path);
    }
    path.push("shallow");
    match std::fs::read_to_string(&path) {
        Ok(contents) => Ok(contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashSet::new()),
        Err(e) => Err(GtError::Git(format!("cannot read {}: {e}", path.display()))),
    }
}

/// The stable patch-id of a commit's diff, or `None` for a commit with no
/// textual diff (a merge, or an empty commit). Two commits with the same
/// patch-id introduce the same change — the equivalence `git rebase` uses to
/// recognise an already-applied commit, and how gt locates the commit an
/// external rebase replayed a stranded branch's tip as.
pub fn patch_id(commit: &str) -> Result<Option<String>> {
    let diff = run(&["show", "--no-color", commit])?;
    if diff.is_empty() {
        return Ok(None);
    }
    let out = run_with_stdin(&["patch-id", "--stable"], diff.as_bytes())?;
    Ok(out.split_whitespace().next().map(str::to_string))
}

/// `(patch_id, commit_sha)` for every non-merge commit in `base..tip`. Used to
/// find the commit equivalent to a stranded branch's tip among those an external
/// rebase replayed onto the new base.
pub fn patch_ids_in_range(base: &str, tip: &str) -> Result<Vec<(String, String)>> {
    let range = format!("{base}..{tip}");
    let log = run(&["log", "-p", "--no-color", "--no-merges", &range])?;
    if log.is_empty() {
        return Ok(Vec::new());
    }
    // `git patch-id` reads the diff stream and emits "<patch-id> <commit-sha>"
    // per commit, pairing each diff with the commit it belongs to.
    let out = run_with_stdin(&["patch-id", "--stable"], log.as_bytes())?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pid = it.next()?;
            let sha = it.next()?;
            Some((pid.to_string(), sha.to_string()))
        })
        .collect())
}

/// Porcelain working-tree status (empty string means clean).
pub fn status_porcelain() -> Result<String> {
    run(&["status", "--porcelain"])
}

/// Whether the working tree has uncommitted changes (staged or unstaged).
/// Untracked files don't count: rebase and switch never touch them, and git
/// itself refuses to overwrite one rather than discarding it.
pub fn is_dirty() -> Result<bool> {
    Ok(!run(&["status", "--porcelain", "--untracked-files=no"])?.is_empty())
}

/// Whether there are staged changes ready to commit.
pub fn has_staged_changes() -> Result<bool> {
    let out = run_allow_fail(&["diff", "--cached", "--quiet"])?;
    Ok(out.code != 0)
}

/// Whether the working tree has unstaged or untracked changes.
pub fn has_unstaged_changes() -> Result<bool> {
    Ok(status_porcelain()?.lines().any(|line| {
        line.starts_with("??") || line.as_bytes().get(1).map(|b| *b != b' ').unwrap_or(false)
    }))
}

/// Whether a rebase is currently in progress.
pub fn rebase_in_progress(git_dir: &Path) -> bool {
    git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists()
}

/// Files with unresolved merge conflicts.
pub fn conflicted_files() -> Result<Vec<String>> {
    let out = run(&["diff", "--name-only", "--diff-filter=U"])?;
    Ok(out
        .lines()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .collect())
}

/// Worktree that currently has `branch` checked out, if any.
///
/// Parses `git worktree list --porcelain`, which emits records of the form:
///
/// ```text
/// worktree /abs/path
/// HEAD <sha>
/// branch refs/heads/<name>
/// ```
///
/// separated by blank lines. The current process's worktree is included.
pub fn worktree_owner_of(branch: &str) -> Result<Option<PathBuf>> {
    let out = run(&["worktree", "list", "--porcelain"])?;
    let wanted = head_ref(branch);
    let mut current: Option<PathBuf> = None;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            current = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("branch ") {
            if rest == wanted {
                return Ok(current);
            }
        } else if line.is_empty() {
            current = None;
        }
    }
    Ok(None)
}

/// If `branch` is checked out in a worktree *other* than the current one,
/// return that worktree's path. A branch checked out elsewhere cannot be
/// rebased or switched to (`git` refuses, e.g. `'B' is already used by worktree
/// at ...`), so callers skip it rather than fail mid-operation.
pub fn branch_occupied_elsewhere(branch: &str) -> Result<Option<PathBuf>> {
    let Some(owner) = worktree_owner_of(branch)? else {
        return Ok(None);
    };
    let here = PathBuf::from(run(&["rev-parse", "--show-toplevel"])?);
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    Ok((canon(&owner) != canon(&here)).then_some(owner))
}

/// Whether the working tree rooted at `worktree` is clean.
pub fn is_clean_in(worktree: &Path) -> Result<bool> {
    let path = worktree
        .to_str()
        .ok_or_else(|| GtError::Git(format!("non-UTF8 worktree path: {worktree:?}")))?;
    let out = run(&["-C", path, "status", "--porcelain"])?;
    Ok(out.is_empty())
}

/// Point a ref at a new SHA via `git update-ref`.
///
/// Callers must enforce the AGENTS.md data-safety rules — in particular, this
/// must only be used to write `refs/heads/<trunk>` when trunk is not checked
/// out in any worktree (so no working tree can be silently dirtied).
pub fn update_ref(ref_name: &str, new_sha: &str) -> Result<()> {
    run(&["update-ref", ref_name, new_sha])?;
    Ok(())
}
