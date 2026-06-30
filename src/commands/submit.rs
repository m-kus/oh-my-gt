//! `gt submit` — push the downstack and create or update its pull requests.

use std::io::Write;
use std::time::Instant;

use crate::error::{GtError, Result};
use crate::gh::PrView;
use crate::graph::StackGraph;
use crate::meta::PrInfo;
use crate::style::OutputStyle;
use crate::{gh, git, meta};

/// Stable HTML-comment markers delimiting the auto-generated stack overview
/// inside a PR body. Anything between (and including) these is owned by `gt`
/// and rewritten on every submit; anything outside is left alone.
const STACK_START: &str = "<!-- gt-stack-start -->";
const STACK_END: &str = "<!-- gt-stack-end -->";

/// Run one network step, printing a live `  <label>… ` line to stderr and
/// appending its elapsed time once it returns (or `failed` if it errors).
///
/// Submitting a stack is a sequence of slow network round-trips — a push and
/// one or more `gh` API calls per branch — with nothing to show in between, so
/// `gt submit` otherwise looks hung. Progress goes to stderr so stdout stays the
/// clean, pipeable list of submitted PRs; timing makes it obvious which round
/// trip is the slow one.
fn step<T>(style: &OutputStyle, label: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    eprint!("  {label}… ");
    let _ = std::io::stderr().flush();
    let start = Instant::now();
    let result = f();
    let secs = start.elapsed().as_secs_f64();
    match &result {
        Ok(_) => eprintln!("{}", style.status(format!("{secs:.1}s"))),
        Err(_) => eprintln!("{}", style.error("failed")),
    }
    result
}

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

    let style = OutputStyle::stderr();
    let total = to_submit.len();
    eprintln!(
        "submitting {total} branch{} to origin",
        if total == 1 { "" } else { "es" }
    );

    // First pass: push each branch and ensure a PR exists with the correct
    // base. We collect the resulting PrView (which carries the live PR body)
    // for use in the second pass, where we render and apply the stack overview.
    let mut prs: Vec<(String, PrView, bool)> = Vec::with_capacity(to_submit.len());
    for (i, branch) in to_submit.iter().enumerate() {
        let base = if i == 0 {
            graph.trunk.clone()
        } else {
            to_submit[i - 1].clone()
        };

        eprintln!(
            "{} {}",
            style.status(format!("[{}/{total}]", i + 1)),
            style.branch(branch)
        );

        // Skip the push when the branch already matches its remote-tracking
        // ref: a force-push would be a no-op, and that ref is what
        // `--force-with-lease` checks anyway, so the comparison is free and
        // consistent. Restacks rewrite history, so when a push is needed it is
        // force-with-lease rather than a plain push.
        let local_tip = git::branch_tip(branch)?;
        if git::remote_tracking_tip("origin", branch)?.as_deref() == Some(local_tip.as_str()) {
            eprintln!("  {}", style.status("already on origin, push skipped"));
        } else {
            let refspec = git::head_refspec(branch);
            step(&style, "pushing to origin", || {
                git::run(&["push", "--force-with-lease", "origin", &refspec]).map(|_| ())
            })?;
        }

        let m = meta::read(branch)?.unwrap_or_default();
        let existing = m.pr_info.as_ref().and_then(|p| p.number);

        let (pr, was_existing) = match existing {
            Some(num) => {
                step(
                    &style,
                    &format!("retargeting PR #{num} base → {base}"),
                    || gh::set_base(num, &base),
                )?;
                let v = step(&style, &format!("reading back PR #{num}"), || {
                    gh::view(branch)
                })?
                .ok_or_else(|| {
                    GtError::Gh(format!("PR #{num} for `{branch}` could not be read back"))
                })?;
                (v, true)
            }
            None => match step(&style, "looking up existing PR", || gh::view(branch))? {
                // A PR already exists for this branch — adopt and re-target it.
                Some(v) => {
                    let need_refresh = v.base != base;
                    if need_refresh {
                        let num = v.number;
                        step(
                            &style,
                            &format!("retargeting PR #{num} base → {base}"),
                            || gh::set_base(num, &base),
                        )?;
                    }
                    let view = if need_refresh {
                        step(&style, &format!("reading back PR #{}", v.number), || {
                            gh::view(branch)
                        })?
                        .unwrap_or(v)
                    } else {
                        v
                    };
                    (view, true)
                }
                None => {
                    // Fetch the full commit message and split it ourselves so a
                    // multi-line body (with blank lines, punctuation, etc.) is
                    // preserved verbatim in the PR description — GitHub only
                    // wants a single-line title, but the body has no such cap.
                    let branch_ref = git::head_ref(branch);
                    let full = git::run(&["show", "-s", "--format=%B", &branch_ref])?;
                    let (title, body) = split_subject_body(&full);
                    step(&style, "creating draft PR", || {
                        gh::create(branch, &base, &title, &body)
                    })?;
                    let v = step(&style, "reading back new PR", || gh::view(branch))?.ok_or_else(
                        || GtError::Gh(format!("PR for `{branch}` could not be read back")),
                    )?;
                    (v, false)
                }
            },
        };

        prs.push((branch.clone(), pr, was_existing));
    }

    // Second pass: rewrite each PR body to include a fresh stack overview that
    // links every PR in the chain. For single-branch stacks we still strip any
    // stale section from a previous submit but never add a new one.
    let section = if prs.len() > 1 {
        let entries: Vec<(&str, &str)> = prs
            .iter()
            .map(|(b, v, _)| (b.as_str(), v.url.as_str()))
            .collect();
        Some(render_stack_overview(&entries, &graph.trunk))
    } else {
        None
    };

    if prs.len() > 1 {
        eprintln!("updating stack overview in each PR description");
    }

    for (branch, pr, was_existing) in &prs {
        let new_body = compose_body(&pr.body, section.as_deref(), &pr.url);
        if new_body != pr.body {
            step(
                &style,
                &format!("updating description of PR #{}", pr.number),
                || gh::set_body(pr.number, &new_body),
            )?;
        }

        let mut m = meta::read(branch)?.unwrap_or_default();
        m.pr_info = Some(PrInfo {
            number: Some(pr.number),
            base: Some(pr.base.clone()),
            url: Some(pr.url.clone()),
            title: Some(pr.title.clone()),
            // The body is computed fresh on every submit from the live PR and
            // the current graph, so there is no need to snapshot it here.
            body: None,
            state: Some(pr.state.clone()),
        });
        meta::write(branch, &m)?;

        let verb = if *was_existing {
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

/// Render the marker-delimited stack overview block. `entries` lists every PR
/// in the stack from bottom (trunk's first child) to top. Each row is a bare
/// PR URL on its own line so GitHub auto-links it with title and merge-state
/// badge — the branch name in `entries` is unused but kept for call-site
/// readability. `trunk` is the base branch the stack is rooted on; it appears
/// as the bottom row in backticks since it has no PR.
fn render_stack_overview(entries: &[(&str, &str)], trunk: &str) -> String {
    // Top of the stack appears first so the chain reads from tip downwards —
    // matching how reviewers walk a stack on GitHub.
    let mut s = String::new();
    s.push_str(STACK_START);
    s.push('\n');
    s.push_str("**Stack:**\n");
    s.push('\n');
    for (_branch, url) in entries.iter().rev() {
        s.push_str("- ");
        s.push_str(url);
        s.push('\n');
    }
    s.push_str("- `");
    s.push_str(trunk);
    s.push_str("`\n");
    s.push_str(STACK_END);
    s
}

/// Highlight the current PR's line inside a rendered overview section. The
/// marker sits on the right and points back at the URL so the chain of bare
/// URLs stays left-aligned and easy to scan. Matches by URL so the rendered
/// output can stay link-text-free.
fn mark_current(section: &str, current_url: &str) -> String {
    let needle = format!("- {current_url}\n");
    let replacement = format!("- {current_url} 👈\n");
    section.replacen(&needle, &replacement, 1)
}

/// Build the final PR body for the branch whose PR is at `current_url`: strip
/// any prior marker block, then append the freshly rendered section with this
/// branch's row marked. When `section` is `None` the body is returned with any
/// prior block stripped but no new block added — used for single-branch stacks
/// where the overview is suppressed.
fn compose_body(existing: &str, section: Option<&str>, current_url: &str) -> String {
    let stripped = strip_stack_section(existing);
    match section {
        None => stripped,
        Some(s) => {
            let marked = mark_current(s, current_url);
            append_stack_section(&stripped, &marked)
        }
    }
}

/// Remove an existing `<!-- gt-stack-start -->`…`<!-- gt-stack-end -->` block
/// from `body`, along with the single blank-line separator that precedes it.
/// If the markers are absent the body is returned unchanged.
fn strip_stack_section(body: &str) -> String {
    let Some(start) = body.find(STACK_START) else {
        return body.to_string();
    };
    let Some(end) = body[start..].find(STACK_END) else {
        // Half-open block — leave the body alone rather than risk destroying
        // user content between mismatched markers.
        return body.to_string();
    };
    let end_byte = start + end + STACK_END.len();

    // Eat the blank line we previously inserted between the user's body and
    // the section, plus any single trailing newline immediately after the
    // closing marker, so repeated re-renders do not accumulate whitespace.
    let mut prefix_end = start;
    while prefix_end > 0 && body.as_bytes()[prefix_end - 1] == b'\n' {
        prefix_end -= 1;
    }
    let mut suffix_start = end_byte;
    if body.as_bytes().get(suffix_start).copied() == Some(b'\n') {
        suffix_start += 1;
    }

    let mut out = String::with_capacity(body.len());
    out.push_str(&body[..prefix_end]);
    if !body[suffix_start..].is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&body[suffix_start..]);
    }
    out
}

/// Append `section` to `body` with a blank-line separator. If `body` is empty
/// the section stands alone.
fn append_stack_section(body: &str, section: &str) -> String {
    let trimmed = body.trim_end_matches('\n');
    if trimmed.is_empty() {
        return section.to_string();
    }
    let mut out = String::with_capacity(trimmed.len() + section.len() + 2);
    out.push_str(trimmed);
    out.push_str("\n\n");
    out.push_str(section);
    out
}

#[cfg(test)]
mod tests {
    use super::{
        append_stack_section, compose_body, mark_current, render_stack_overview,
        split_subject_body, strip_stack_section, STACK_END, STACK_START,
    };

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

    #[test]
    fn render_stack_overview_orders_top_first() {
        // The stack should read tip-first so reviewers see where to start.
        // Rows are bare PR URLs so GitHub auto-renders title + status badges.
        // Trunk is the last row, backticked, since it has no PR of its own.
        let entries = vec![
            ("alpha", "https://example.test/pr/1"),
            ("beta", "https://example.test/pr/2"),
            ("gamma", "https://example.test/pr/3"),
        ];
        let section = render_stack_overview(&entries, "main");
        assert!(section.starts_with(STACK_START));
        assert!(section.ends_with(STACK_END));
        // No markdown link syntax — bare URLs only.
        assert!(!section.contains("]("));
        let g = section.find("/pr/3").unwrap();
        let b = section.find("/pr/2").unwrap();
        let a = section.find("/pr/1").unwrap();
        let trunk = section.find("`main`").unwrap();
        assert!(
            g < b && b < a && a < trunk,
            "expected gamma, beta, alpha, then trunk"
        );
    }

    #[test]
    fn mark_current_highlights_only_one_entry() {
        // Only the current PR's row should gain the arrow.
        let entries = vec![
            ("alpha", "https://example.test/pr/1"),
            ("beta", "https://example.test/pr/2"),
        ];
        let section = render_stack_overview(&entries, "main");
        let marked = mark_current(&section, "https://example.test/pr/2");
        assert!(marked.contains("- https://example.test/pr/2 👈"));
        assert!(marked.contains("- https://example.test/pr/1\n"));
        // The non-current row must stay free of any marker.
        assert!(!marked.contains("- https://example.test/pr/1 👈"));
        // Trunk row stays plain — there is no PR to mark current on.
        assert!(marked.contains("- `main`"));
        // Re-marking is a no-op once the marker is in place.
        assert_eq!(mark_current(&marked, "https://example.test/pr/2"), marked);
    }

    #[test]
    fn strip_then_append_round_trip_preserves_user_edits() {
        // User content outside the markers must survive a re-render.
        let original = "User wrote this above.\n\nAnd this paragraph too.";
        let section_a = render_stack_overview(
            &[
                ("alpha", "https://example.test/pr/1"),
                ("beta", "https://example.test/pr/2"),
            ],
            "main",
        );
        let with_a = append_stack_section(original, &section_a);
        assert!(with_a.starts_with("User wrote this above."));
        assert!(with_a.contains(STACK_START));

        let stripped = strip_stack_section(&with_a);
        assert_eq!(stripped, original);

        let section_b = render_stack_overview(
            &[
                ("alpha", "https://example.test/pr/1"),
                ("beta", "https://example.test/pr/2"),
                ("gamma", "https://example.test/pr/3"),
            ],
            "main",
        );
        let with_b = append_stack_section(&stripped, &section_b);
        assert!(with_b.contains("/pr/3"));
        assert!(with_b.starts_with("User wrote this above."));
    }

    #[test]
    fn compose_body_replaces_existing_section_idempotently() {
        // A first render then a second render with a different stack should
        // produce the same output as rendering once with the new stack.
        let body0 = "Initial commit body.";
        let v1 = vec![
            ("alpha", "https://example.test/pr/1"),
            ("beta", "https://example.test/pr/2"),
        ];
        let s1 = render_stack_overview(&v1, "main");
        let body1 = compose_body(body0, Some(&s1), "https://example.test/pr/2");
        let body1_again = compose_body(&body1, Some(&s1), "https://example.test/pr/2");
        assert_eq!(body1, body1_again, "rendering twice must be a no-op");

        let v2 = vec![
            ("alpha", "https://example.test/pr/1"),
            ("beta", "https://example.test/pr/2"),
            ("gamma", "https://example.test/pr/3"),
        ];
        let s2 = render_stack_overview(&v2, "main");
        let body2 = compose_body(&body1, Some(&s2), "https://example.test/pr/2");
        let expected = compose_body(body0, Some(&s2), "https://example.test/pr/2");
        assert_eq!(body2, expected);
    }

    #[test]
    fn compose_body_with_none_section_strips_existing_block() {
        // When the stack shrinks to one branch we drop the section entirely.
        let v = vec![
            ("alpha", "https://example.test/pr/1"),
            ("beta", "https://example.test/pr/2"),
        ];
        let s = render_stack_overview(&v, "main");
        let with_section = compose_body("Body.", Some(&s), "https://example.test/pr/1");
        assert!(with_section.contains(STACK_START));
        let stripped = compose_body(&with_section, None, "https://example.test/pr/1");
        assert_eq!(stripped, "Body.");
    }

    #[test]
    fn strip_leaves_body_unchanged_when_no_markers() {
        let body = "Just a body without any section.";
        assert_eq!(strip_stack_section(body), body);
    }

    #[test]
    fn strip_leaves_body_unchanged_when_only_start_marker_present() {
        // Half-open block — refuse to delete user content.
        let body = "Body\n\n<!-- gt-stack-start -->\n- something\n";
        assert_eq!(strip_stack_section(body), body);
    }
}
