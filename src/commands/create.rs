//! `gt create` — create a new branch stacked on the current one and commit the
//! staged changes onto it.

use crate::error::{GtError, Result};
use crate::graph::{StackGraph, Validation};
use crate::meta::BranchMetadata;
use crate::style::OutputStyle;
use crate::{git, meta, prompt};

pub fn run() -> Result<()> {
    let (graph, parent) = StackGraph::load_current()?;

    let parent_ok = graph.is_trunk(&parent)
        || graph
            .get(&parent)
            .map(|n| n.validation == Validation::Valid)
            .unwrap_or(false);
    if !parent_ok {
        return Err(GtError::Precondition(format!(
            "`{parent}` is not tracked; run `gt track` before stacking on it"
        )));
    }

    if !git::has_staged_changes()? {
        return Err(GtError::Precondition(
            "no staged changes; stage something with `git add` first".into(),
        ));
    }

    let message = prompt::editor_message("commit message")?;
    // The branch-name default uses only the first line of the message, since
    // editor messages can be multi-line and the rest is body, not subject.
    let subject = message.lines().next().unwrap_or("");
    let default_name = slugify(subject);
    let name = prompt::input_branch_name("branch name", Some(&default_name))?;

    if git::branch_exists(&name)? {
        return Err(GtError::Usage(format!("branch `{name}` already exists")));
    }

    // The new branch forks from the parent's current tip.
    let parent_tip = graph.get(&parent).unwrap().tip.clone();

    git::run(&["branch", "--", &name])?;
    git::run(&["switch", "--", &name])?;
    git::run(&["commit", "-m", &message])?;

    meta::write(&name, &BranchMetadata::new(&parent, &parent_tip))?;
    let style = OutputStyle::stdout();
    println!(
        "{} `{}` on `{}`",
        style.success("created"),
        style.branch(&name),
        style.branch(&parent),
    );
    Ok(())
}

/// Turn a commit message into a plausible branch name.
fn slugify(message: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in message.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_end_matches('-');
    let trimmed: String = slug.chars().take(40).collect();
    if trimmed.is_empty() {
        "branch".to_string()
    } else {
        trimmed.trim_end_matches('-').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Add user login flow"), "add-user-login-flow");
        assert_eq!(slugify("  Fix: the thing!! "), "fix-the-thing");
        assert_eq!(slugify("***"), "branch");
    }
}
