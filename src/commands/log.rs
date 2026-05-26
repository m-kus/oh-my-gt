//! `gt log` — print the stack as a tree (a debugging / orientation aid).

use crate::error::Result;
use crate::graph::{StackGraph, Validation};
use crate::style::OutputStyle;

pub fn run() -> Result<()> {
    let graph = StackGraph::load()?;
    let trunk = graph.trunk.clone();
    let style = OutputStyle::stdout();
    print_node(&graph, &trunk, 0, &style);

    let mut untracked: Vec<&str> = graph
        .nodes
        .values()
        .filter(|n| n.validation == Validation::Untracked && n.name != trunk)
        .map(|n| n.name.as_str())
        .collect();
    untracked.sort();
    if !untracked.is_empty() {
        println!("\n{} {}", style.warning("untracked:"), untracked.join(", "));
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
        println!(
            "\n{} {}",
            style.warning("needs repair (run `gt track`):"),
            broken.join(", ")
        );
    }
    Ok(())
}

fn print_node(graph: &StackGraph, name: &str, depth: usize, style: &OutputStyle) {
    let node = &graph.nodes[name];
    let indent = "  ".repeat(depth);
    let is_current = graph.current.as_deref() == Some(name);
    // Match charcoal's glyphs: filled circle for the current branch,
    // hollow circle for the rest.
    let glyph = if is_current { "\u{25c9}" } else { "\u{25cb}" };
    let short = &node.tip[..node.tip.len().min(8)];
    let needs_restack = name != graph.trunk && graph.needs_restack(name);

    let branch_label: Box<dyn std::fmt::Display> = if is_current {
        Box::new(style.branch(name))
    } else {
        Box::new(name.to_string())
    };

    print!(
        "{indent}{} {}  {}",
        style.glyph(glyph),
        branch_label,
        style.glyph(short),
    );
    if needs_restack {
        print!("  {}", style.restack_marker("(needs restack)"));
    }
    println!();

    for child in &node.children {
        print_node(graph, child, depth + 1, style);
    }
}
