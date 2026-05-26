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

    // Capture submitted descendants before the amend: the restack will move
    // their local tips past whatever the remote PR branch currently points at.
    let submitted_descendants: Vec<String> = graph
        .descendants(&current)
        .into_iter()
        .filter(|b| {
            graph
                .get(b)
                .and_then(|n| n.meta.as_ref())
                .and_then(|m| m.pr_info.as_ref())
                .and_then(|p| p.number)
                .is_some()
        })
        .collect();

    if git::run_interactive(&["commit", "--amend", "--no-edit"])? != 0 {
        return Err(GtError::Git("`git commit --amend` failed".into()));
    }

    println!("{} `{}`", style.success("amended"), style.branch(&current),);

    // The amend moved this branch's tip, so every descendant must restack.
    rebase::restack(std::slice::from_ref(&current), "modify")?;

    // After a clean restack, any descendant whose PR was already submitted
    // now has a local tip ahead of its remote PR branch. Nudge the user, but
    // do not touch refs, metadata, or the working tree.
    if !submitted_descendants.is_empty() {
        let branches: Vec<String> = submitted_descendants
            .iter()
            .map(|b| style.branch(b).to_string())
            .collect();
        println!(
            "{} re-submit affected PRs: {}",
            style.status("hint:"),
            branches.join(" ")
        );
    }
    Ok(())
}
