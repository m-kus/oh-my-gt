//! `gt submit` — push the downstack and create or update its pull requests.

use crate::error::{GtError, Result};
use crate::graph::StackGraph;
use crate::meta::PrInfo;
use crate::{gh, git, meta};

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;
    if graph.is_trunk(&current) {
        return Err(GtError::Usage("the trunk has no pull request to submit".into()));
    }

    // The downstack path trunk..current; submit everything but the trunk.
    let path = graph.path_from_trunk(&current);
    let to_submit: Vec<String> = path.into_iter().filter(|b| *b != graph.trunk).collect();
    if to_submit.is_empty() {
        return Err(GtError::Usage(format!(
            "`{current}` is not tracked; run `gt track` first"
        )));
    }

    // Every branch must be valid and already restacked — never submit a stale stack.
    for b in &to_submit {
        graph.require_tracked(b)?;
        if graph.needs_restack(b) {
            return Err(GtError::State(
                "the stack is out of date; run `gt restack` before submitting".into(),
            ));
        }
    }
    git::ensure_remote()?;

    // Submit bottom-up so each PR's base branch already exists on the remote.
    for (i, branch) in to_submit.iter().enumerate() {
        let base = if i == 0 {
            graph.trunk.clone()
        } else {
            to_submit[i - 1].clone()
        };

        // Restacks rewrite history, so force-with-lease rather than plain push.
        git::run(&["push", "--force-with-lease", "origin", branch])?;

        let mut m = meta::read(branch)?.unwrap_or_default();
        let existing = m.pr_info.as_ref().and_then(|p| p.number);
        let prev_body = m.pr_info.as_ref().and_then(|p| p.body.clone());

        let pr = match existing {
            Some(num) => {
                gh::set_base(num, &base)?;
                gh::view(branch)?.ok_or_else(|| {
                    GtError::Gh(format!("PR #{num} for `{branch}` could not be read back"))
                })?
            }
            None => match gh::view(branch)? {
                // A PR already exists for this branch — adopt and re-target it.
                Some(v) => {
                    if v.base != base {
                        gh::set_base(v.number, &base)?;
                    }
                    gh::view(branch)?.unwrap_or(v)
                }
                None => {
                    let title = git::run(&["show", "-s", "--format=%s", branch])?;
                    let body = git::run(&["show", "-s", "--format=%b", branch])?;
                    gh::create(branch, &base, &title, &body)?;
                    gh::view(branch)?.ok_or_else(|| {
                        GtError::Gh(format!("PR for `{branch}` could not be read back"))
                    })?
                }
            },
        };

        m.pr_info = Some(PrInfo {
            number: Some(pr.number),
            base: Some(pr.base.clone()),
            url: Some(pr.url.clone()),
            title: Some(pr.title.clone()),
            body: prev_body,
            state: Some(pr.state.clone()),
        });
        meta::write(branch, &m)?;

        let verb = if existing.is_some() { "updated  " } else { "submitted" };
        println!("{verb}  {branch}  #{}  {}", pr.number, pr.url);
    }
    Ok(())
}
