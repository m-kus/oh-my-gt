//! `gt up` — switch from the current branch to a child in the stack.

use crate::error::{GtError, Result};
use crate::graph::StackGraph;
use crate::style::OutputStyle;
use crate::{git, prompt};

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;
    let node = graph
        .get(&current)
        .ok_or_else(|| GtError::Precondition(format!("`{current}` is not a local branch")))?;

    // `link_children` only attaches `Valid` children, so this list already
    // excludes broken metadata.
    let children = node.children.clone();
    if children.is_empty() {
        return Err(GtError::Precondition(format!(
            "`{current}` has no valid child branch"
        )));
    }

    git::ensure_clean()?;
    let idx = prompt::select(&format!("child branch for `{current}`:"), &children, 0)?;
    let target = &children[idx];
    git::run(&["switch", "--", target])?;
    let style = OutputStyle::stdout();
    println!(
        "{} to `{}`",
        style.success("switched"),
        style.branch(target)
    );
    Ok(())
}
