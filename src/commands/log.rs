//! `gt log` — print the stack as a tree (a debugging / orientation aid).

use crate::error::Result;
use crate::graph::{StackGraph, Validation};

pub fn run() -> Result<()> {
    let graph = StackGraph::load()?;
    let trunk = graph.trunk.clone();
    print_node(&graph, &trunk, 0);

    let mut untracked: Vec<&str> = graph
        .nodes
        .values()
        .filter(|n| n.validation == Validation::Untracked && n.name != trunk)
        .map(|n| n.name.as_str())
        .collect();
    untracked.sort();
    if !untracked.is_empty() {
        println!("\nuntracked: {}", untracked.join(", "));
    }

    let mut broken: Vec<String> = graph
        .nodes
        .values()
        .filter(|n| {
            matches!(
                n.validation,
                Validation::BadParentName
                    | Validation::BadParentRevision
                    | Validation::InvalidParent
            )
        })
        .map(|n| format!("{} ({:?})", n.name, n.validation))
        .collect();
    broken.sort();
    if !broken.is_empty() {
        println!("\nneeds repair (run `gt track`): {}", broken.join(", "));
    }
    Ok(())
}

fn print_node(graph: &StackGraph, name: &str, depth: usize) {
    let node = &graph.nodes[name];
    let indent = "  ".repeat(depth);
    let here = if graph.current.as_deref() == Some(name) {
        " *"
    } else {
        ""
    };
    let short = &node.tip[..node.tip.len().min(8)];
    let flag = if name != graph.trunk && graph.needs_restack(name) {
        "  (needs restack)"
    } else {
        ""
    };
    println!("{indent}{name}{here}  {short}{flag}");
    for child in &node.children {
        print_node(graph, child, depth + 1);
    }
}
