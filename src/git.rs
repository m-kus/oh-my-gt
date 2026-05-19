//! The single chokepoint for talking to the `git` binary.
//!
//! Every `Command::new("git")` in the crate lives here. Higher layers call the
//! typed helpers and never construct git command lines themselves.

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

/// Porcelain working-tree status (empty string means clean).
pub fn status_porcelain() -> Result<String> {
    run(&["status", "--porcelain"])
}

/// Whether the working tree has uncommitted changes (staged or unstaged).
pub fn is_dirty() -> Result<bool> {
    Ok(!status_porcelain()?.is_empty())
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
