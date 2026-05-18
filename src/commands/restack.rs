//! `gt restack` — rebase the current stack so each branch sits on its parent.

use crate::error::Result;
use crate::graph::StackGraph;
use crate::rebase;

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;

    // Restack the whole stack that contains the current branch.
    let stack = graph.stack_of(&current);
    let root = stack.into_iter().next().unwrap_or(current);
    rebase::restack(&[root], "restack")
}
