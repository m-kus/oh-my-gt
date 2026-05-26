//! `gt down` — switch from the current branch to its parent in the stack.

use crate::error::{GtError, Result};
use crate::git;
use crate::graph::{StackGraph, Validation};
use crate::style::OutputStyle;

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;
    let node = graph
        .get(&current)
        .ok_or_else(|| GtError::Precondition(format!("`{current}` is not a local branch")))?;

    if graph.is_trunk(&current) {
        return Err(GtError::Precondition(format!(
            "`{current}` is the trunk; nothing below it"
        )));
    }
    if node.validation != Validation::Valid {
        return Err(GtError::Precondition(format!(
            "`{current}` is not tracked; run `gt track` first"
        )));
    }

    let parent = node
        .parent
        .as_ref()
        .ok_or_else(|| GtError::Precondition(format!("`{current}` has no parent branch")))?;

    git::ensure_clean()?;
    git::run(&["switch", "--", parent])?;
    let style = OutputStyle::stdout();
    println!(
        "{} to `{}`",
        style.success("switched"),
        style.branch(parent)
    );
    Ok(())
}
