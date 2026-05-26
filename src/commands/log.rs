//! `gt log` — print the stack as a tree (a debugging / orientation aid).
//!
//! Shares its traversal and rendering with `gt tree` via `crate::tree`;
//! the only difference is that this view also shows each branch's tip SHA
//! and any `(needs restack)` marker.

use crate::error::Result;
use crate::graph::StackGraph;
use crate::tree;

pub fn run() -> Result<()> {
    let graph = StackGraph::load()?;
    tree::print_detailed_tree(&graph);
    Ok(())
}
