//! `gt tree` — print the tracked stack as a focused branch tree.
//!
//! Unlike `gt log`, this view drops commit SHAs and restack markers so the
//! shape of the stack is easy to read at a glance. Untracked or broken
//! branches are summarized below the tree rather than mixed into it.

use crate::error::Result;
use crate::graph::StackGraph;
use crate::tree;

pub fn run() -> Result<()> {
    let graph = StackGraph::load()?;
    tree::print_tree(&graph);
    Ok(())
}
