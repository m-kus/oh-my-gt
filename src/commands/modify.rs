//! `gt modify` — amend the current branch's commit and restack its descendants.

use crate::error::{GtError, Result};
use crate::graph::StackGraph;
use crate::style::OutputStyle;
use crate::{git, rebase};

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

    if !git::has_staged_changes()? {
        return Err(GtError::Precondition(
            "no staged changes; stage something with `git add` first".into(),
        ));
    }

    let style = OutputStyle::stdout();
    if git::has_unstaged_changes()? {
        println!(
            "{} unstaged changes will not be included in the amend",
            style.warning("warning:")
        );
    }

    if git::run_interactive(&["commit", "--amend", "--no-edit"])? != 0 {
        return Err(GtError::Git("`git commit --amend` failed".into()));
    }

    println!("{} `{}`", style.success("amended"), style.branch(&current),);

    // The amend moved this branch's tip, so every descendant must restack.
    rebase::restack(std::slice::from_ref(&current), "modify")
}
