//! `gt modify` — amend the current branch's commit and restack its descendants.

use crate::error::{GtError, Result};
use crate::graph::StackGraph;
use crate::{git, prompt, rebase};

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;
    let node = graph.require_tracked(&current)?;

    let base = node
        .meta
        .as_ref()
        .unwrap()
        .parent_branch_revision
        .clone()
        .unwrap();
    if node.tip == base {
        return Err(GtError::Precondition(format!(
            "`{current}` has no commit to amend"
        )));
    }

    if !git::status_porcelain()?.is_empty()
        && prompt::confirm("stage all changes before amending?", true)?
    {
        git::run(&["add", "-A"])?;
    }

    if !git::has_staged_changes()?
        && !prompt::confirm("nothing staged; amend just the commit message?", false)?
    {
        return Err(GtError::Aborted);
    }

    let args: &[&str] = if prompt::confirm("edit the commit message?", false)? {
        &["commit", "--amend"]
    } else {
        &["commit", "--amend", "--no-edit"]
    };
    if git::run_interactive(args)? != 0 {
        return Err(GtError::Git("`git commit --amend` failed".into()));
    }

    println!("amended `{current}`");

    // The amend moved this branch's tip, so every descendant must restack.
    rebase::restack(std::slice::from_ref(&current), "modify")
}
