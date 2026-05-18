//! `gt untrack` — stop tracking the current branch, reparenting any children.

use crate::error::{GtError, Result};
use crate::graph::StackGraph;
use crate::{meta, prompt};

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;

    if graph.is_trunk(&current) {
        return Err(GtError::Usage("the trunk is not tracked".into()));
    }

    let node = graph
        .get(&current)
        .filter(|n| n.meta.is_some())
        .ok_or_else(|| GtError::Usage(format!("`{current}` is not tracked")))?;

    let parent = node.parent.clone();
    let children = node.children.clone();

    if !children.is_empty() {
        let onto = parent.as_deref().unwrap_or("its parent");
        let q = format!(
            "`{current}` has tracked children ({}); reparent them onto `{onto}`?",
            children.join(", ")
        );
        if !prompt::confirm(&q, true)? {
            return Err(GtError::Aborted);
        }
        if let Some(p) = &parent {
            for child in &children {
                if let Some(mut cm) = graph.get(child).and_then(|n| n.meta.clone()) {
                    cm.parent_branch_name = Some(p.clone());
                    meta::write(child, &cm)?;
                }
            }
        }
    }

    meta::delete(&current)?;
    println!("untracked `{current}`");
    Ok(())
}
