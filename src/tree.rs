//! Shared rendering for the tracked branch tree.
//!
//! Both `gt log` (with commit SHAs and restack markers) and `gt tree`
//! (branch shape only) walk the same `StackGraph` traversal — they only
//! differ in how each line is formatted.

use std::collections::HashSet;

use crate::graph::{StackGraph, Validation};
use crate::style::OutputStyle;

/// A rendered line with its source branch, useful for picker prompts that
/// want to display the tree shape but map each entry back to a branch name.
#[derive(Debug, Clone)]
pub(crate) struct TreeLine {
    // Read by `gt move`'s future picker (issue #4) and by the unit tests
    // here; the binary build of `gt log`/`gt tree` only renders `text`.
    #[allow(dead_code)]
    pub(crate) branch: String,
    pub(crate) text: String,
}

/// Print the detailed tree (used by `gt log`): glyph, branch name, short
/// SHA, and a `(needs restack)` marker where applicable.
pub(crate) fn print_detailed_tree(graph: &StackGraph) {
    print_with_format(graph, LineFormat::Detailed);
}

/// Print the tree-only view (used by `gt tree`): glyph + branch name only.
pub(crate) fn print_tree(graph: &StackGraph) {
    print_with_format(graph, LineFormat::TreeOnly);
}

/// Tree of branch names for picker prompts. `included` filters which
/// branches participate in the tree; the depth and ordering match the
/// printed views so the prompt mirrors what the user sees in `gt tree`.
#[allow(dead_code)] // wired by a future `gt move` change (issue #4)
pub(crate) fn branch_lines(graph: &StackGraph, included: &HashSet<String>) -> Vec<TreeLine> {
    let style = OutputStyle::stdout();
    let mut out = Vec::new();
    render_node(
        graph,
        &graph.trunk,
        0,
        Some(included),
        LineFormat::BranchOnly,
        &style,
        &mut out,
    );
    out
}

/// Lines summarizing branches that are not part of the tree itself
/// (untracked locals, broken metadata). Shared by `gt log` and `gt tree`.
pub(crate) fn summary_lines(graph: &StackGraph) -> Vec<String> {
    let style = OutputStyle::stdout();
    let mut lines = Vec::new();

    let mut untracked: Vec<&str> = graph
        .nodes
        .values()
        .filter(|n| n.validation == Validation::Untracked && n.name != graph.trunk)
        .map(|n| n.name.as_str())
        .collect();
    untracked.sort();
    if !untracked.is_empty() {
        lines.push(format!(
            "{} {}",
            style.warning("untracked:"),
            untracked.join(", ")
        ));
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
        lines.push(format!(
            "{} {}",
            style.warning("needs repair (run `gt track`):"),
            broken.join(", ")
        ));
    }

    lines
}

#[derive(Clone, Copy)]
enum LineFormat {
    /// `gt log`: glyph + branch + short SHA + optional restack marker.
    Detailed,
    /// `gt tree`: glyph + branch only.
    TreeOnly,
    /// Picker prompts: indent + branch + current marker, no glyph column.
    BranchOnly,
}

fn print_with_format(graph: &StackGraph, format: LineFormat) {
    let style = OutputStyle::stdout();
    let mut lines = Vec::new();
    render_node(graph, &graph.trunk, 0, None, format, &style, &mut lines);
    for line in &lines {
        println!("{}", line.text);
    }
    for line in summary_lines(graph) {
        println!();
        println!("{line}");
    }
}

fn render_node(
    graph: &StackGraph,
    name: &str,
    depth: usize,
    included: Option<&HashSet<String>>,
    format: LineFormat,
    style: &OutputStyle,
    out: &mut Vec<TreeLine>,
) {
    let node = &graph.nodes[name];
    if included.map(|set| set.contains(name)).unwrap_or(true) {
        out.push(TreeLine {
            branch: name.to_string(),
            text: format_line(graph, name, depth, format, style),
        });
    }
    // `children` is sorted in `StackGraph::link_children`, so iteration here
    // is deterministic across runs and matches the picker ordering.
    for child in &node.children {
        render_node(graph, child, depth + 1, included, format, style, out);
    }
}

fn format_line(
    graph: &StackGraph,
    name: &str,
    depth: usize,
    format: LineFormat,
    style: &OutputStyle,
) -> String {
    let node = &graph.nodes[name];
    let indent = "  ".repeat(depth);
    let is_current = graph.current.as_deref() == Some(name);

    match format {
        LineFormat::BranchOnly => {
            let branch: Box<dyn std::fmt::Display> = if is_current {
                Box::new(style.branch(name))
            } else {
                Box::new(name.to_string())
            };
            let marker = if is_current {
                format!(" {}", style.glyph("(current)"))
            } else {
                String::new()
            };
            format!("{indent}{branch}{marker}")
        }
        LineFormat::TreeOnly | LineFormat::Detailed => {
            // Match charcoal's glyphs: filled circle for the current branch,
            // hollow circle for the rest.
            let glyph = if is_current { "\u{25c9}" } else { "\u{25cb}" };
            let branch: Box<dyn std::fmt::Display> = if is_current {
                Box::new(style.branch(name))
            } else {
                Box::new(name.to_string())
            };
            let mut line = format!("{indent}{} {}", style.glyph(glyph), branch);
            if matches!(format, LineFormat::Detailed) {
                let short = &node.tip[..node.tip.len().min(8)];
                line.push_str(&format!("  {}", style.glyph(short)));
                if name != graph.trunk && graph.needs_restack(name) {
                    line.push_str(&format!("  {}", style.restack_marker("(needs restack)")));
                }
            }
            line
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{BranchNode, StackGraph};
    use crate::meta::BranchMetadata;
    use std::collections::HashMap;

    /// Build a graph with the shape:
    ///
    ///   main
    ///     alpha
    ///       beta
    ///       gamma
    ///
    /// The two children of `alpha` are sorted; `gamma` is checked out.
    fn branching_graph() -> StackGraph {
        let mut nodes: HashMap<String, BranchNode> = HashMap::new();
        nodes.insert(
            "main".to_string(),
            BranchNode {
                name: "main".into(),
                tip: "1111111111111111".into(),
                meta: None,
                validation: Validation::Trunk,
                parent: None,
                children: vec!["alpha".into()],
            },
        );
        nodes.insert(
            "alpha".to_string(),
            BranchNode {
                name: "alpha".into(),
                tip: "2222222222222222".into(),
                meta: Some(BranchMetadata::new("main", "1111111111111111")),
                validation: Validation::Valid,
                parent: Some("main".into()),
                children: vec!["beta".into(), "gamma".into()],
            },
        );
        nodes.insert(
            "beta".to_string(),
            BranchNode {
                name: "beta".into(),
                tip: "3333333333333333".into(),
                meta: Some(BranchMetadata::new("alpha", "2222222222222222")),
                validation: Validation::Valid,
                parent: Some("alpha".into()),
                children: Vec::new(),
            },
        );
        nodes.insert(
            "gamma".to_string(),
            BranchNode {
                name: "gamma".into(),
                tip: "4444444444444444".into(),
                meta: Some(BranchMetadata::new("alpha", "2222222222222222")),
                validation: Validation::Valid,
                parent: Some("alpha".into()),
                children: Vec::new(),
            },
        );
        StackGraph {
            nodes,
            trunk: "main".into(),
            current: Some("gamma".into()),
        }
    }

    fn collect(graph: &StackGraph, format: LineFormat) -> Vec<TreeLine> {
        let style = OutputStyle::stdout();
        let mut out = Vec::new();
        render_node(graph, &graph.trunk, 0, None, format, &style, &mut out);
        out
    }

    #[test]
    fn branching_tree_renders_children_in_sorted_order() {
        let graph = branching_graph();
        let lines = collect(&graph, LineFormat::TreeOnly);

        let branches: Vec<&str> = lines.iter().map(|l| l.branch.as_str()).collect();
        assert_eq!(branches, vec!["main", "alpha", "beta", "gamma"]);

        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "\u{25cb} main",
                "  \u{25cb} alpha",
                "    \u{25cb} beta",
                "    \u{25c9} gamma",
            ]
        );
    }

    #[test]
    fn branching_tree_render_is_deterministic() {
        let graph = branching_graph();
        let first = collect(&graph, LineFormat::TreeOnly);
        let second = collect(&graph, LineFormat::TreeOnly);
        let first_texts: Vec<&str> = first.iter().map(|l| l.text.as_str()).collect();
        let second_texts: Vec<&str> = second.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(first_texts, second_texts);
    }

    #[test]
    fn detailed_format_includes_short_sha() {
        let graph = branching_graph();
        let lines = collect(&graph, LineFormat::Detailed);
        // beta's tip starts with 33333333; the line must contain that short SHA.
        let beta = lines.iter().find(|l| l.branch == "beta").unwrap();
        assert!(
            beta.text.contains("33333333"),
            "expected short SHA in detailed line, got `{}`",
            beta.text
        );
    }

    #[test]
    fn branch_only_format_marks_current() {
        let graph = branching_graph();
        let mut included = HashSet::new();
        for n in graph.nodes.keys() {
            included.insert(n.clone());
        }
        let lines = branch_lines(&graph, &included);
        let gamma = lines.iter().find(|l| l.branch == "gamma").unwrap();
        assert!(
            gamma.text.contains("(current)"),
            "expected current marker on `gamma`, got `{}`",
            gamma.text
        );
        let beta = lines.iter().find(|l| l.branch == "beta").unwrap();
        assert!(
            !beta.text.contains("(current)"),
            "non-current branch should not be marked, got `{}`",
            beta.text
        );
    }
}
