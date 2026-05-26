//! `gt submit` — push the downstack and create or update its pull requests.

use crate::error::{GtError, Result};
use crate::graph::StackGraph;
use crate::meta::PrInfo;
use crate::{gh, git, meta};

pub fn run() -> Result<()> {
    let (graph, current) = StackGraph::load_current()?;
    if graph.is_trunk(&current) {
        return Err(GtError::Usage(
            "the trunk has no pull request to submit".into(),
        ));
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
        let refspec = git::head_refspec(branch);
        git::run(&["push", "--force-with-lease", "origin", &refspec])?;

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
                    // Fetch the full commit message and split it ourselves so a
                    // multi-line body (with blank lines, punctuation, etc.) is
                    // preserved verbatim in the PR description — GitHub only
                    // wants a single-line title, but the body has no such cap.
                    let branch_ref = git::head_ref(branch);
                    let full = git::run(&["show", "-s", "--format=%B", &branch_ref])?;
                    let (title, body) = split_subject_body(&full);
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

        let verb = if existing.is_some() {
            "updated  "
        } else {
            "submitted"
        };
        println!("{verb}  {branch}  #{}  {}", pr.number, pr.url);
    }
    Ok(())
}

/// Split a full commit message (`git show --format=%B`) into a PR title and
/// body. The title is the first line; the body is everything after the first
/// newline with surrounding whitespace stripped — internal blank lines and
/// punctuation are preserved exactly so a `Subject\n\nBody\nmore` message
/// round-trips into GitHub without losing structure.
fn split_subject_body(full: &str) -> (String, String) {
    match full.split_once('\n') {
        Some((subject, rest)) => (subject.trim().to_string(), rest.trim().to_string()),
        None => (full.trim().to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::split_subject_body;

    #[test]
    fn single_line_message_has_empty_body() {
        // A one-line commit must produce the same title and an empty body —
        // the historical behavior we must not regress.
        let (title, body) = split_subject_body("alpha feature");
        assert_eq!(title, "alpha feature");
        assert_eq!(body, "");

        // %B almost always has a trailing newline; treat it as one-line too.
        let (title, body) = split_subject_body("alpha feature\n");
        assert_eq!(title, "alpha feature");
        assert_eq!(body, "");
    }

    #[test]
    fn multi_line_message_preserves_body_structure() {
        // Subject + blank line + multi-paragraph body with punctuation: the
        // title is just the subject; the body keeps the internal blank line
        // and all trailing punctuation, with only surrounding whitespace
        // stripped.
        let raw = "Add login flow\n\
                   \n\
                   This wires the new sign-in screen end-to-end.\n\
                   \n\
                   - handles 2FA\n\
                   - resolves #42!\n";
        let (title, body) = split_subject_body(raw);
        assert_eq!(title, "Add login flow");
        assert_eq!(
            body,
            "This wires the new sign-in screen end-to-end.\n\
             \n\
             - handles 2FA\n\
             - resolves #42!"
        );
    }

    #[test]
    fn body_without_blank_separator_still_preserved() {
        // git is permissive — a message can have a body without the canonical
        // blank line after the subject. We still treat the first line as the
        // title and keep the rest as body.
        let (title, body) = split_subject_body("Subject line\nBody on the next line.\n");
        assert_eq!(title, "Subject line");
        assert_eq!(body, "Body on the next line.");
    }
}
