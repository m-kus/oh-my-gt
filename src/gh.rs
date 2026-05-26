//! Thin wrapper around the GitHub CLI (`gh`). All PR creation/lookup goes
//! through here; there is no direct HTTP client.

use std::process::Command;

use serde::Deserialize;

use crate::error::{GtError, Result};

/// The subset of PR fields oh-my-gt cares about.
#[derive(Debug, Deserialize)]
pub struct PrView {
    pub number: u64,
    pub url: String,
    #[serde(rename = "baseRefName")]
    pub base: String,
    /// `OPEN` | `MERGED` | `CLOSED`.
    pub state: String,
    #[serde(default)]
    pub title: String,
    /// Current PR body on GitHub. May contain a previously-rendered stack
    /// overview section, plus any user edits made outside the markers.
    #[serde(default)]
    pub body: String,
}

fn spawn(args: &[&str]) -> Result<(i32, String, String)> {
    let out = Command::new("gh").args(args).output().map_err(|e| {
        GtError::Gh(format!(
            "failed to run `gh` (is the GitHub CLI installed?): {e}"
        ))
    })?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

/// Run `gh <args>`, erroring (tagged with `ctx`) on a non-zero exit.
fn checked(args: &[&str], ctx: &str) -> Result<()> {
    let (code, out, err) = spawn(args)?;
    if code != 0 {
        let detail = if err.is_empty() { out } else { err };
        return Err(GtError::Gh(format!("`gh {ctx}` failed: {detail}")));
    }
    Ok(())
}

/// Look up the pull request whose head is `branch`, if one exists.
pub fn view(branch: &str) -> Result<Option<PrView>> {
    let (code, stdout, stderr) = spawn(&[
        "pr",
        "view",
        "--json",
        "number,url,baseRefName,state,title,body",
        "--",
        branch,
    ])?;
    if code != 0 {
        let s = stderr.to_lowercase();
        if s.contains("no pull requests found") || s.contains("no open pull requests") {
            return Ok(None);
        }
        return Err(GtError::Gh(format!(
            "`gh pr view {branch}` failed: {stderr}"
        )));
    }
    Ok(Some(serde_json::from_str(&stdout)?))
}

/// Create a pull request for `head` against `base`.
///
/// New PRs are opened as drafts so a stacked-PR submitter can iterate on the
/// stack before requesting review; publishing is a one-click action on GitHub.
/// Update paths (`set_base`) intentionally leave draft state alone.
pub fn create(head: &str, base: &str, title: &str, body: &str) -> Result<()> {
    checked(
        &[
            "pr", "create", "--draft", "--head", head, "--base", base, "--title", title, "--body",
            body,
        ],
        &format!("pr create for `{head}`"),
    )
}

/// Re-target an existing pull request onto a new base branch.
pub fn set_base(number: u64, base: &str) -> Result<()> {
    checked(
        &["pr", "edit", &number.to_string(), "--base", base],
        "pr edit",
    )
}

/// Replace the body of an existing pull request.
pub fn set_body(number: u64, body: &str) -> Result<()> {
    checked(
        &["pr", "edit", &number.to_string(), "--body", body],
        "pr edit",
    )
}
