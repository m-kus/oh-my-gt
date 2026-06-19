use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestRepo {
    root: PathBuf,
    repo: PathBuf,
    env: HashMap<String, OsString>,
}

impl TestRepo {
    fn new(name: &str) -> Self {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("oh-my-gt-{name}-{}-{id}", std::process::id()));
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();

        let mut t = TestRepo {
            root,
            repo,
            env: HashMap::new(),
        };
        t.setup_env();
        t.git(&["init", "-q", "-b", "main", "."]);
        t.git(&["config", "user.email", "e2e@example.test"]);
        t.git(&["config", "user.name", "E2E Bot"]);
        t
    }

    fn setup_env(&mut self) {
        self.env.insert("TZ".into(), "UTC".into());
        self.env.insert("LC_ALL".into(), "C".into());
        self.env.insert("GIT_AUTHOR_NAME".into(), "E2E Bot".into());
        self.env
            .insert("GIT_AUTHOR_EMAIL".into(), "e2e@example.test".into());
        self.env
            .insert("GIT_COMMITTER_NAME".into(), "E2E Bot".into());
        self.env
            .insert("GIT_COMMITTER_EMAIL".into(), "e2e@example.test".into());
        self.env.insert("GIT_EDITOR".into(), "true".into());
        self.env
            .insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
        self.env
            .insert("GIT_CONFIG_SYSTEM".into(), "/dev/null".into());

        let bin = self.root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let fake_gh = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/harness/fake_gh");
        fs::copy(fake_gh, bin.join("gh")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let path = format!("{}:{}", bin.display(), old_path.to_string_lossy());
        self.env.insert("PATH".into(), path.into());
        self.env.insert(
            "FAKE_GH_STATE".into(),
            self.root.join("ghstate").into_os_string(),
        );
    }

    fn cmd(&self, program: &str) -> Command {
        self.cmd_in(program, &self.repo)
    }

    fn cmd_in(&self, program: &str, cwd: &Path) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(cwd);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd
    }

    fn git_in(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = self.cmd_in("git", cwd);
        cmd.args(args);
        let out = cmd.output().unwrap();
        assert_success(&out, &format!("git {} (in {:?})", args.join(" "), cwd));
        String::from_utf8_lossy(&out.stdout)
            .trim_end_matches('\n')
            .to_string()
    }

    fn gt_in(&self, cwd: &Path, subcommand: &str, input: &str) -> Output {
        let mut cmd = self.cmd_in(gt_bin().to_str().unwrap(), cwd);
        cmd.arg(subcommand)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        let mut cmd = self.cmd("git");
        cmd.args(args);
        let out = cmd.output().unwrap();
        assert_success(&out, &format!("git {}", args.join(" ")));
        String::from_utf8_lossy(&out.stdout)
            .trim_end_matches('\n')
            .to_string()
    }

    fn git_allow_fail(&self, args: &[&str]) -> Output {
        let mut cmd = self.cmd("git");
        cmd.args(args);
        cmd.output().unwrap()
    }

    fn gt(&self, subcommand: &str, input: &str) -> Output {
        let mut cmd = self.cmd(gt_bin().to_str().unwrap());
        cmd.arg(subcommand)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn write(&self, file: &str, content: &str) {
        fs::write(self.repo.join(file), content).unwrap();
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    fn add_origin(&self) {
        self.git(&[
            "init",
            "-q",
            "--bare",
            self.root.join("origin.git").to_str().unwrap(),
        ]);
        self.git(&[
            "remote",
            "add",
            "origin",
            self.root.join("origin.git").to_str().unwrap(),
        ]);
    }

    fn create_with_gt(&self, message: &str, branch: &str, file: &str, content: &str) {
        self.write(file, content);
        self.git(&["add", file]);
        let out = self.gt("create", &format!("{message}\n{branch}\n"));
        assert_success(&out, "gt create");
    }

    fn write_metadata(&self, branch: &str, parent: &str, parent_revision: &str) {
        let json = format!(
            r#"{{"parentBranchName":"{parent}","parentBranchRevision":"{parent_revision}"}}"#
        );
        let mut hash = self.cmd("git");
        hash.args(["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = hash.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(json.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_success(&out, "git hash-object");
        let blob = String::from_utf8_lossy(&out.stdout).trim().to_string();
        self.git(&[
            "update-ref",
            &format!("refs/branch-metadata/{branch}"),
            &blob,
        ]);
    }

    fn set_pr_number(&self, branch: &str, number: u64) {
        // Splice a synthetic `prInfo` into the existing metadata blob so the
        // branch looks "submitted" without going through `gt submit` (which
        // would need a working remote + fake gh state).
        let raw = self.git(&["cat-file", "-p", &format!("refs/branch-metadata/{branch}")]);
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let obj = value.as_object_mut().unwrap();
        obj.insert(
            "prInfo".into(),
            serde_json::json!({
                "number": number,
                "base": "main",
                "url": format!("https://example.test/pr/{number}"),
                "state": "OPEN"
            }),
        );
        let new_json = serde_json::to_string(&value).unwrap();
        let mut hash = self.cmd("git");
        hash.args(["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = hash.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(new_json.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_success(&out, "git hash-object");
        let blob = String::from_utf8_lossy(&out.stdout).trim().to_string();
        self.git(&[
            "update-ref",
            &format!("refs/branch-metadata/{branch}"),
            &blob,
        ]);
    }

    fn metadata_parent(&self, branch: &str) -> String {
        let json = self.git(&["cat-file", "-p", &format!("refs/branch-metadata/{branch}")]);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["parentBranchName"].as_str().unwrap().to_string()
    }

    fn local_branch_exists(&self, branch: &str) -> bool {
        self.git_allow_fail(&[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .status
        .success()
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"])
    }

    fn current_branch(&self) -> String {
        self.git(&["symbolic-ref", "--short", "HEAD"])
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn gt_bin() -> PathBuf {
    option_env!("CARGO_BIN_EXE_gt")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/gt"))
}

fn assert_success(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn submit_branch_named_like_git_option_does_not_mirror_other_refs() {
    let repo = TestRepo::new("submit-option-branch");
    repo.add_origin();

    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    let root = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["push", "-q", "origin", "main"]);

    repo.git(&["update-ref", "refs/heads/--mirror", "HEAD"]);
    repo.git(&["switch", "--", "--mirror"]);
    repo.write("option.txt", "option\n");
    repo.commit("option branch");
    repo.write_metadata("--mirror", "main", &root);

    repo.git(&["switch", "-q", "main"]);
    repo.git(&["switch", "-q", "-c", "secret"]);
    repo.write("secret.txt", "must stay local\n");
    repo.commit("secret branch");
    repo.git(&["switch", "--", "--mirror"]);

    let out = repo.gt("submit", "");
    let leaked = repo.git(&["ls-remote", "--heads", "origin", "secret"]);
    assert!(
        leaked.is_empty(),
        "submitting --mirror must not push unrelated branch `secret`; got {leaked:?}"
    );
    assert_success(&out, "gt submit");
}

#[test]
fn abort_after_conflicted_move_restores_old_parent_metadata() {
    let repo = TestRepo::new("move-abort-metadata");
    repo.write("shared.txt", "L1\n");
    repo.commit("root commit");

    repo.create_with_gt("alpha feature", "alpha", "shared.txt", "alpha\n");
    repo.create_with_gt("beta feature", "beta", "shared.txt", "alpha beta\n");

    let move_out = repo.gt("move", "main\n");
    assert!(
        !move_out.status.success(),
        "move should pause on a conflict\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&move_out.stdout),
        String::from_utf8_lossy(&move_out.stderr)
    );

    let abort_out = repo.gt("abort", "");
    assert_success(&abort_out, "gt abort");
    assert_eq!(repo.metadata_parent("beta"), "alpha");
}

#[test]
fn sync_deletes_branch_whose_commits_are_already_on_trunk_without_pr_metadata() {
    let repo = TestRepo::new("sync-empty-cherry");
    repo.add_origin();
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    repo.git(&["switch", "-q", "main"]);
    repo.git(&["merge", "-q", "--no-ff", "alpha", "-m", "merge alpha"]);
    repo.git(&["push", "-q", "origin", "main"]);
    repo.git(&["switch", "-q", "alpha"]);

    let out = repo.gt("sync", "y\n");
    assert_success(&out, "gt sync");
    assert!(
        !repo.local_branch_exists("alpha"),
        "sync should delete a tracked branch that has no commits left outside trunk"
    );
}

#[test]
fn modify_amends_staged_changes_without_editing_message() {
    let repo = TestRepo::new("modify-staged-only");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");

    repo.write("alpha.txt", "alpha\namended\n");
    repo.git(&["add", "alpha.txt"]);
    let out = repo.gt("modify", "");

    assert_success(&out, "gt modify");
    assert_eq!(repo.git(&["log", "-1", "--pretty=%s"]), "alpha feature");
    assert_eq!(repo.git(&["show", "HEAD:alpha.txt"]), "alpha\namended");
    assert!(repo.git(&["status", "--porcelain"]).is_empty());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("stage all changes before amending?"));
    assert!(!stdout.contains("edit the commit message?"));
}

#[test]
fn modify_warns_and_leaves_unstaged_changes_out_of_amend() {
    let repo = TestRepo::new("modify-staged-plus-unstaged");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");

    repo.write("alpha.txt", "alpha\nstaged\n");
    repo.git(&["add", "alpha.txt"]);
    repo.write("alpha.txt", "alpha\nstaged\nunstaged\n");
    repo.write("scratch.txt", "scratch\n");
    let out = repo.gt("modify", "");

    assert_success(&out, "gt modify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("warning: unstaged changes will not be included in the amend"));
    assert_eq!(repo.git(&["log", "-1", "--pretty=%s"]), "alpha feature");
    assert_eq!(repo.git(&["show", "HEAD:alpha.txt"]), "alpha\nstaged");
    let status = repo.git(&["status", "--porcelain"]);
    assert!(
        status.lines().any(|line| line == " M alpha.txt"),
        "expected unstaged edit to remain, got status:\n{status}"
    );
    assert!(
        status.lines().any(|line| line == "?? scratch.txt"),
        "expected untracked file to remain, got status:\n{status}"
    );
}

#[test]
fn move_prompt_shows_stack_tree_with_only_valid_parents_selectable() {
    // Build a multi-branch stack:  main <- alpha <- (beta, gamma) , and
    // create a child of beta so the picker has a non-selectable descendant
    // to render for shape.
    //
    //   main
    //     alpha
    //       beta          (current)
    //         delta       (descendant — must appear but not be pickable)
    //       gamma
    let repo = TestRepo::new("move-tree-picker");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");

    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    repo.create_with_gt("beta feature", "beta", "beta.txt", "beta\n");
    repo.create_with_gt("delta feature", "delta", "delta.txt", "delta\n");
    repo.git(&["switch", "-q", "alpha"]);
    repo.create_with_gt("gamma feature", "gamma", "gamma.txt", "gamma\n");
    repo.git(&["switch", "-q", "beta"]);

    // Type the new parent by name so we don't depend on numeric ordering.
    // Moving beta from alpha onto main exercises the picker *and* the
    // rebase so we know the picker rewrite did not regress move's behavior.
    let out = repo.gt("move", "main\n");
    assert_success(&out, "gt move");
    let stdout = String::from_utf8_lossy(&out.stdout);

    // The prompt header is followed by the rendered tree. Every tracked
    // branch must appear (so the user sees the whole stack), but only the
    // valid parent candidates (main, alpha, gamma) should be numbered.
    assert!(
        stdout.contains("move `beta` onto:"),
        "missing prompt header in:\n{stdout}"
    );
    for branch in ["main", "alpha", "beta", "delta", "gamma"] {
        assert!(
            stdout.contains(branch),
            "expected `{branch}` to appear in tree:\n{stdout}"
        );
    }

    // Capture the numbered lines: format is `  > N) ...` or `    N) ...`.
    let numbered: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            let s = l.trim_start();
            s.starts_with("> ") || s.starts_with(|c: char| c.is_ascii_digit())
        })
        .filter(|l| l.contains(") "))
        .collect();
    let selectable: Vec<&str> = numbered
        .iter()
        .filter_map(|l| {
            ["main", "alpha", "beta", "delta", "gamma"]
                .iter()
                .find(|b| l.contains(*b))
                .copied()
        })
        .collect();
    assert_eq!(
        selectable,
        vec!["main", "alpha", "gamma"],
        "only valid parents should be numbered choices; got picker lines:\n{}",
        numbered.join("\n")
    );

    // Trunk is the default when valid; the `>` marker sits on its line.
    let default_line = stdout
        .lines()
        .find(|l| l.contains("> ") && l.contains("main"))
        .unwrap_or_else(|| panic!("trunk should be the default choice; got:\n{stdout}"));
    assert!(default_line.contains("1)"));

    // And the rebase actually re-parented beta onto main (sanity check —
    // the picker change must not affect the move logic).
    assert_eq!(repo.metadata_parent("beta"), "main");
    assert_eq!(repo.metadata_parent("delta"), "beta");
}

#[test]
fn modify_errors_without_staged_changes_before_mutating() {
    let repo = TestRepo::new("modify-nothing-staged");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    let before = repo.head();

    repo.write("alpha.txt", "alpha\nunstaged\n");
    let out = repo.gt("modify", "");

    assert!(
        !out.status.success(),
        "gt modify should fail with nothing staged"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no staged changes; stage something with `git add` first"));
    assert_eq!(repo.head(), before);
    assert_eq!(repo.git(&["show", "HEAD:alpha.txt"]), "alpha");
}

#[test]
fn down_switches_from_feature_to_parent_including_trunk() {
    // main <- alpha <- beta. `gt down` from beta lands on alpha, and a
    // second `gt down` lands on trunk.
    let repo = TestRepo::new("down-walks-to-trunk");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    repo.create_with_gt("beta feature", "beta", "beta.txt", "beta\n");
    assert_eq!(repo.current_branch(), "beta");

    let out = repo.gt("down", "");
    assert_success(&out, "gt down (beta -> alpha)");
    assert_eq!(repo.current_branch(), "alpha");

    let out = repo.gt("down", "");
    assert_success(&out, "gt down (alpha -> main)");
    assert_eq!(repo.current_branch(), "main");

    // Trunk has no parent; the next `gt down` must refuse cleanly.
    let out = repo.gt("down", "");
    assert!(
        !out.status.success(),
        "gt down on trunk should fail with a precondition error"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("trunk"),
        "expected trunk precondition error; got stderr:\n{stderr}"
    );
}

#[test]
fn up_with_single_child_switches_silently() {
    // main <- alpha <- beta. From alpha, `gt up` has exactly one valid
    // child (beta) and must switch without prompting.
    let repo = TestRepo::new("up-single-child");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    repo.create_with_gt("beta feature", "beta", "beta.txt", "beta\n");
    repo.git(&["switch", "-q", "alpha"]);

    let out = repo.gt("up", "");
    assert_success(&out, "gt up (alpha -> beta)");
    assert_eq!(repo.current_branch(), "beta");
    // A single-child `up` should not print a numbered chooser.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("choose ["),
        "single-child `gt up` should not prompt; got stdout:\n{stdout}"
    );
}

#[test]
fn up_from_leaf_branch_errors_with_no_children() {
    // main <- alpha. `gt up` from leaf alpha must fail cleanly without
    // switching anywhere.
    let repo = TestRepo::new("up-leaf-precondition");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    assert_eq!(repo.current_branch(), "alpha");

    let out = repo.gt("up", "");
    assert!(!out.status.success(), "gt up on a leaf branch must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no valid child branch"),
        "expected no-child precondition error; got stderr:\n{stderr}"
    );
    // Still on alpha; the failed command must not move HEAD.
    assert_eq!(repo.current_branch(), "alpha");
}

#[test]
fn create_preserves_branch_name_and_rejects_whitespace_inputs() {
    // Issue #12: branch-name input must be preserved verbatim, never silently
    // trimmed. Surrounding whitespace is an explicit, recoverable error
    // (because git itself would reject it), and the default still flows
    // through when the user just presses Enter.
    let repo = TestRepo::new("branch-name-no-trim");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");

    // 1) Typed input with surrounding whitespace must NOT be silently
    //    stripped — it must produce a clear error and leave nothing behind.
    repo.write("alpha.txt", "alpha\n");
    repo.git(&["add", "alpha.txt"]);
    let out = repo.gt("create", "alpha feature\n  bad-name  \n");
    assert!(
        !out.status.success(),
        "create should reject whitespace-padded branch names; got stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("invalid character"),
        "expected explicit rejection of whitespace; stderr:\n{stderr}"
    );
    assert!(
        !repo.local_branch_exists("bad-name"),
        "rejected name must not have been silently trimmed into a real branch"
    );

    // 2) An exact, valid name is preserved character-for-character.
    let out = repo.gt("create", "alpha feature\nalpha\n");
    assert_success(&out, "gt create with explicit name");
    assert_eq!(repo.current_branch(), "alpha");

    // 3) Pressing Enter on the branch-name prompt falls back to the slugged
    //    default (the existing behavior we must not break).
    repo.git(&["switch", "-q", "main"]);
    repo.write("beta.txt", "beta\n");
    repo.git(&["add", "beta.txt"]);
    let out = repo.gt("create", "beta feature\n\n");
    assert_success(&out, "gt create with default branch name");
    assert_eq!(repo.current_branch(), "beta-feature");
}

#[test]
fn up_with_multiple_children_prompts_with_deterministic_order() {
    // main <- alpha <- (beta, gamma). From alpha, `gt up` should show a
    // numbered prompt listing both children in sorted order, and pick the
    // one named by the user's input.
    let repo = TestRepo::new("up-branch-point");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    repo.create_with_gt("beta feature", "beta", "beta.txt", "beta\n");
    repo.git(&["switch", "-q", "alpha"]);
    repo.create_with_gt("gamma feature", "gamma", "gamma.txt", "gamma\n");
    repo.git(&["switch", "-q", "alpha"]);

    // Type the branch name to avoid coupling the test to numeric order.
    let out = repo.gt("up", "gamma\n");
    assert_success(&out, "gt up (alpha -> gamma)");
    assert_eq!(repo.current_branch(), "gamma");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("child branch for `alpha`:"),
        "expected prompt header; got stdout:\n{stdout}"
    );
    // Children render in sorted order, so beta is listed before gamma.
    let beta_pos = stdout.find("beta").expect("beta in prompt");
    let gamma_pos = stdout.find("gamma").expect("gamma in prompt");
    assert!(
        beta_pos < gamma_pos,
        "children should be listed in sorted order; got stdout:\n{stdout}"
    );
}

#[test]
fn modify_prints_submit_hint_for_submitted_descendants() {
    // main <- a <- b, with b marked as submitted (PR #42). Amending a moves
    // its tip, so the restack rewrites b — and b's remote PR branch is now
    // behind. The modify command must say so without changing anything else.
    let repo = TestRepo::new("modify-hint-submitted-descendant");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("a feature", "a", "a.txt", "a\n");
    repo.create_with_gt("b feature", "b", "b.txt", "b\n");
    repo.set_pr_number("b", 42);

    // Amend `a` with a fresh staged change.
    repo.git(&["switch", "-q", "a"]);
    repo.write("a.txt", "a\namended\n");
    repo.git(&["add", "a.txt"]);
    let out = repo.gt("modify", "");

    assert_success(&out, "gt modify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let hint_line = stdout
        .lines()
        .find(|l| l.contains("hint: re-submit affected PRs:"))
        .unwrap_or_else(|| {
            panic!("expected re-submit hint after restacking submitted descendant; got stdout:\n{stdout}")
        });
    assert!(
        hint_line.contains('b'),
        "expected branch `b` to be named in the hint line; got: {hint_line}"
    );
    // Sanity: the metadata PR number must not have been touched.
    let json = repo.git(&["cat-file", "-p", "refs/branch-metadata/b"]);
    assert!(
        json.contains("\"number\":42"),
        "modify must not rewrite pr_info; metadata is:\n{json}"
    );
}

#[test]
fn submit_preserves_full_multi_line_commit_message_in_new_pr() {
    // A branch whose tip commit has a subject + blank line + multi-paragraph
    // body must produce a PR with the subject as title and the *entire* body
    // (blank lines, punctuation, every line) preserved verbatim. Regression
    // for issue #9, where only the first line was being kept.
    let repo = TestRepo::new("submit-multiline-body");
    repo.add_origin();

    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    let root = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["push", "-q", "origin", "main"]);

    // Build a branch with a deliberately multi-line commit message — git
    // accepts repeated `-m` flags as paragraphs separated by a blank line,
    // so this mirrors the real-world editor flow without needing a TTY.
    repo.git(&["switch", "-q", "-c", "alpha"]);
    repo.write("alpha.txt", "alpha\n");
    repo.git(&["add", "alpha.txt"]);
    repo.git(&[
        "commit",
        "-q",
        "-m",
        "Add login flow",
        "-m",
        "This wires the new sign-in screen end-to-end.\n\n- handles 2FA\n- resolves #42!",
    ]);
    repo.write_metadata("alpha", "main", &root);

    let out = repo.gt("submit", "");
    assert_success(&out, "gt submit");

    // The fake `gh` records the body bytes alongside the rest of the PR
    // state, so we can assert on what `gh pr create --body` actually got.
    let body_path = repo.root.join("ghstate/body_alpha");
    let body = fs::read_to_string(&body_path)
        .unwrap_or_else(|e| panic!("missing fake-gh body file {body_path:?}: {e}"));
    assert_eq!(
        body, "This wires the new sign-in screen end-to-end.\n\n- handles 2FA\n- resolves #42!",
        "submit must forward the full commit body (blank lines + punctuation) to gh pr create",
    );

    // And the title is just the subject — GitHub only accepts a single line.
    let pr_path = repo.root.join("ghstate/pr_alpha");
    let pr_state = fs::read_to_string(&pr_path).unwrap();
    assert!(
        pr_state.contains("TITLE=\"Add login flow\""),
        "submit must use the first line as the PR title; got pr state:\n{pr_state}"
    );
}

#[test]
fn submit_sends_empty_body_for_single_line_commit_message() {
    // A one-line commit must continue to produce an empty PR body — the
    // historical contract this issue must not regress.
    let repo = TestRepo::new("submit-single-line-body");
    repo.add_origin();

    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");

    let out = repo.gt("submit", "");
    assert_success(&out, "gt submit");

    let body_path = repo.root.join("ghstate/body_alpha");
    let body = fs::read_to_string(&body_path)
        .unwrap_or_else(|e| panic!("missing fake-gh body file {body_path:?}: {e}"));
    assert_eq!(body, "", "single-line commit must produce an empty PR body");
}

#[test]
fn submit_appends_stack_overview_with_current_pr_marker() {
    // A multi-branch stack must produce one PR body per branch that contains
    // the marker block, every branch link in the chain, and an arrow next to
    // the current branch's line. A re-submit after adding another branch
    // must update every body to include the new entry, and user content
    // written outside the markers must survive the rewrite.
    let repo = TestRepo::new("submit-stack-overview");
    repo.add_origin();

    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    repo.create_with_gt("beta feature", "beta", "beta.txt", "beta\n");

    let out = repo.gt("submit", "");
    assert_success(&out, "gt submit (initial)");

    let body_alpha = fs::read_to_string(repo.root.join("ghstate/body_alpha")).unwrap();
    let body_beta = fs::read_to_string(repo.root.join("ghstate/body_beta")).unwrap();
    for (branch, body) in [("alpha", &body_alpha), ("beta", &body_beta)] {
        assert!(
            body.contains("<!-- gt-stack-start -->") && body.contains("<!-- gt-stack-end -->"),
            "body for `{branch}` missing markers:\n{body}"
        );
        // Each row is a bare PR URL so GitHub renders title + status badges.
        // The current-PR marker is the 👈 emoji appended after the URL.
        assert!(
            body.contains("- https://example.test/pr/1"),
            "body for `{branch}` missing alpha PR URL:\n{body}"
        );
        assert!(
            body.contains("- https://example.test/pr/2"),
            "body for `{branch}` missing beta PR URL:\n{body}"
        );
        assert!(
            !body.contains("](https://example.test/pr/"),
            "body for `{branch}` must use bare URLs, not markdown links:\n{body}"
        );
        // Trunk is shown as the bottom row, in backticks, since it has no PR.
        assert!(
            body.contains("- `main`"),
            "body for `{branch}` missing trunk row:\n{body}"
        );
    }
    assert!(
        body_alpha.contains("- https://example.test/pr/1 👈"),
        "alpha body must mark its own PR URL as current:\n{body_alpha}"
    );
    assert!(
        body_beta.contains("- https://example.test/pr/2 👈"),
        "beta body must mark its own PR URL as current:\n{body_beta}"
    );

    // Simulate the user editing the PR body on GitHub outside the markers,
    // then add a third branch and re-submit. The user prose must survive and
    // the section must update to include the new branch.
    let preserved = "User wrote this prose above the section.\n\nAnd kept this paragraph too.";
    let edited = format!("{preserved}\n\n{}", body_beta.trim());
    fs::write(repo.root.join("ghstate/body_beta"), &edited).unwrap();

    repo.create_with_gt("gamma feature", "gamma", "gamma.txt", "gamma\n");
    let out = repo.gt("submit", "");
    assert_success(&out, "gt submit (after gamma)");

    let body_beta_after = fs::read_to_string(repo.root.join("ghstate/body_beta")).unwrap();
    assert!(
        body_beta_after.starts_with("User wrote this prose above the section."),
        "user prose outside the markers must survive re-submit; got:\n{body_beta_after}"
    );
    assert!(
        body_beta_after.contains("And kept this paragraph too."),
        "user prose must not be truncated; got:\n{body_beta_after}"
    );
    assert!(
        body_beta_after.contains("- https://example.test/pr/3"),
        "stack section must list the newly-added branch; got:\n{body_beta_after}"
    );
    assert!(
        body_beta_after.contains("- https://example.test/pr/2 👈"),
        "current marker must remain on `beta`'s PR URL line; got:\n{body_beta_after}"
    );
}

/// Push a new commit onto `origin/main` without touching the local `main`
/// branch in any existing worktree. We use a throwaway helper worktree on a
/// scratch branch so the local trunk ref stays at its original tip and no
/// existing worktree is dirtied.
fn advance_origin_main(repo: &TestRepo, message: &str, file: &str, content: &str) {
    let upstream_dir = repo.root.join("upstream_helper");
    repo.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "upstream-helper",
        upstream_dir.to_str().unwrap(),
        "main",
    ]);
    fs::write(upstream_dir.join(file), content).unwrap();
    repo.git_in(&upstream_dir, &["add", file]);
    repo.git_in(&upstream_dir, &["commit", "-q", "-m", message]);
    repo.git_in(&upstream_dir, &["push", "-q", "origin", "HEAD:main"]);
    repo.git(&["worktree", "remove", "-f", upstream_dir.to_str().unwrap()]);
    repo.git(&["branch", "-D", "-q", "upstream-helper"]);
}

#[test]
fn sync_fast_forwards_trunk_when_checked_out_in_other_worktree() {
    // Set up: a primary worktree on `main`, a secondary feature worktree on
    // `alpha`. Advance `origin/main` by one commit, then run `gt sync` from
    // the feature worktree. The fast-forward must reach into the primary
    // worktree without dirtying it, and the feature branch must restack on
    // top of the new trunk tip.
    let repo = TestRepo::new("sync-trunk-other-worktree");
    repo.add_origin();
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    let original_main = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["push", "-q", "origin", "main"]);

    // Build a feature branch via gt, then move it into its own worktree.
    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    let feature_tip_before = repo.git(&["rev-parse", "HEAD"]);
    let feature_dir = repo.root.join("alpha_wt");
    // The feature branch is checked out in the primary worktree right now;
    // move it out by switching primary back to main first.
    repo.git(&["switch", "-q", "main"]);
    repo.git(&[
        "worktree",
        "add",
        "-q",
        feature_dir.to_str().unwrap(),
        "alpha",
    ]);

    // Advance origin/main without touching local main.
    advance_origin_main(&repo, "upstream commit", "upstream.txt", "upstream\n");

    // Sanity: local main is still at the original tip in both worktrees.
    assert_eq!(repo.git(&["rev-parse", "main"]), original_main);

    // Run gt sync from the feature worktree.
    let out = repo.gt_in(&feature_dir, "sync", "");
    assert_success(&out, "gt sync from feature worktree");

    // Trunk advanced (and both worktrees see the same ref).
    let new_main = repo.git(&["rev-parse", "main"]);
    assert_ne!(
        new_main,
        original_main,
        "trunk should have fast-forwarded; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        repo.git_in(&feature_dir, &["rev-parse", "main"]),
        new_main,
        "the secondary worktree must observe the new trunk tip"
    );
    // The primary worktree (which had main checked out) must not be dirty.
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "primary worktree must stay clean after a remote trunk fast-forward"
    );
    assert!(
        repo.git_in(&feature_dir, &["status", "--porcelain"])
            .is_empty(),
        "feature worktree must stay clean after sync"
    );

    // The feature branch was restacked onto the new trunk: its commits now
    // sit on top of the upstream commit, so it has a new tip.
    let feature_tip_after = repo.git(&["rev-parse", "alpha"]);
    assert_ne!(
        feature_tip_after, feature_tip_before,
        "alpha should have been restacked onto the advanced trunk"
    );
    // And the new trunk tip is an ancestor of alpha.
    let merge_base = repo.git(&["merge-base", "alpha", "main"]);
    assert_eq!(
        merge_base, new_main,
        "alpha must have been rebased on top of the new main"
    );
}

#[test]
fn sync_refuses_when_other_trunk_worktree_is_dirty() {
    // Same setup as the happy path, but dirty the primary worktree (which
    // owns `main`) before running sync. The command must refuse with a
    // precondition error, and trunk must NOT advance — neither in the local
    // ref nor in the worktrees.
    let repo = TestRepo::new("sync-trunk-other-worktree-dirty");
    repo.add_origin();
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    let original_main = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["push", "-q", "origin", "main"]);

    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    let feature_dir = repo.root.join("alpha_wt");
    repo.git(&["switch", "-q", "main"]);
    repo.git(&[
        "worktree",
        "add",
        "-q",
        feature_dir.to_str().unwrap(),
        "alpha",
    ]);

    advance_origin_main(&repo, "upstream commit", "upstream.txt", "upstream\n");

    // Dirty the primary worktree (it has main checked out).
    repo.write("dirty.txt", "uncommitted\n");

    let out = repo.gt_in(&feature_dir, "sync", "");
    assert!(
        !out.status.success(),
        "gt sync must fail when the trunk worktree is dirty; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("uncommitted changes") && stderr.contains(repo.repo.to_str().unwrap()),
        "expected a precondition error naming the dirty worktree; stderr:\n{stderr}"
    );

    // Trunk did NOT advance.
    assert_eq!(
        repo.git(&["rev-parse", "main"]),
        original_main,
        "trunk must not move when the owning worktree is dirty"
    );
    // And the uncommitted file is still where we left it.
    assert_eq!(
        fs::read_to_string(repo.repo.join("dirty.txt")).unwrap(),
        "uncommitted\n",
        "the dirty worktree must remain exactly as the user left it"
    );
}

#[test]
fn sync_restacks_with_untracked_files_present() {
    // Untracked files (e.g. .env, editor scratch dirs) never block a rebase —
    // git refuses to overwrite an untracked file rather than discarding it —
    // so sync must proceed and leave them exactly where they are.
    let repo = TestRepo::new("sync-untracked-files");
    repo.add_origin();
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    advance_origin_main(&repo, "upstream commit", "upstream.txt", "upstream\n");

    repo.write("scratch.env", "SECRET=1\n");

    let out = repo.gt("sync", "");
    assert_success(&out, "gt sync");

    // The feature branch was restacked onto the new trunk tip.
    assert_eq!(
        repo.git(&["merge-base", "alpha", "main"]),
        repo.git(&["rev-parse", "main"]),
        "alpha must be restacked onto the advanced trunk"
    );
    // And the untracked file survived untouched.
    assert_eq!(
        fs::read_to_string(repo.repo.join("scratch.env")).unwrap(),
        "SECRET=1\n",
        "untracked file must survive sync untouched"
    );
}

#[test]
fn sync_still_refuses_with_modified_tracked_files() {
    // Unlike untracked files, edits to tracked files could be clobbered by a
    // rebase, so the clean-tree precondition must still hold for them.
    let repo = TestRepo::new("sync-modified-tracked");
    repo.add_origin();
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    repo.create_with_gt("alpha feature", "alpha", "alpha.txt", "alpha\n");
    advance_origin_main(&repo, "upstream commit", "upstream.txt", "upstream\n");
    let alpha_tip_before = repo.git(&["rev-parse", "alpha"]);

    repo.write("alpha.txt", "alpha\nedited\n");

    let out = repo.gt("sync", "");
    assert!(
        !out.status.success(),
        "gt sync must fail with modified tracked files; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("uncommitted changes"),
        "expected the clean-tree precondition error; stderr:\n{stderr}"
    );
    assert_eq!(
        repo.git(&["rev-parse", "alpha"]),
        alpha_tip_before,
        "alpha must not move when the working tree has tracked edits"
    );
    assert_eq!(
        fs::read_to_string(repo.repo.join("alpha.txt")).unwrap(),
        "alpha\nedited\n",
        "the user's edit must remain exactly as they left it"
    );
}

#[test]
fn modify_does_not_print_hint_without_submitted_descendants() {
    // main <- a <- b with NO PR info anywhere. The modify hint is purely a
    // re-submit nudge, so it must stay silent here.
    let repo = TestRepo::new("modify-hint-no-pr");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.create_with_gt("a feature", "a", "a.txt", "a\n");
    repo.create_with_gt("b feature", "b", "b.txt", "b\n");

    repo.git(&["switch", "-q", "a"]);
    repo.write("a.txt", "a\namended\n");
    repo.git(&["add", "a.txt"]);
    let out = repo.gt("modify", "");

    assert_success(&out, "gt modify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("re-submit"),
        "no descendants have PRs, so the hint must stay silent; got stdout:\n{stdout}"
    );
}

#[test]
fn sync_warns_when_current_branch_is_untracked() {
    // The user is on a plain git branch that gt never tracked. Sync must say
    // explicitly that this branch will not be restacked (and how to fix it),
    // while still restacking the tracked stack.
    let repo = TestRepo::new("sync-untracked-current");
    repo.add_origin();
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    repo.create_with_gt("a feature", "a", "a.txt", "a\n");

    // An untracked branch made with plain git, no gt metadata.
    repo.git(&["switch", "-q", "main"]);
    repo.git(&["switch", "-q", "-c", "loose"]);
    repo.write("loose.txt", "loose\n");
    repo.commit("loose work");
    let loose_tip_before = repo.git(&["rev-parse", "loose"]);

    advance_origin_main(&repo, "upstream commit", "upstream.txt", "upstream\n");

    let out = repo.gt("sync", "");
    assert_success(&out, "gt sync");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("loose is not tracked by gt") && stdout.contains("gt track"),
        "sync must warn that the current branch is untracked; got stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("restacked a"),
        "sync should still restack the tracked stack; got stdout:\n{stdout}"
    );

    // The untracked branch is untouched and still checked out.
    assert_eq!(
        repo.git(&["rev-parse", "loose"]),
        loose_tip_before,
        "an untracked branch must never be rewritten by sync"
    );
    assert_eq!(
        repo.git(&["symbolic-ref", "--short", "HEAD"]),
        "loose",
        "sync must return to the original branch"
    );
}

#[test]
fn restack_errors_on_untracked_current_branch() {
    // `gt restack` on an untracked branch used to claim "the stack is
    // already up to date" — misleading, since gt was not managing the branch
    // at all. It must error with a pointer to `gt track` instead.
    let repo = TestRepo::new("restack-untracked-current");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");

    repo.git(&["switch", "-q", "-c", "loose"]);
    repo.write("loose.txt", "loose\n");
    repo.commit("loose work");

    let out = repo.gt("restack", "");
    assert!(
        !out.status.success(),
        "gt restack must fail on an untracked branch; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not tracked") && stderr.contains("gt track"),
        "the error must point at `gt track`; got stderr:\n{stderr}"
    );
}

#[test]
fn track_offers_trunk_even_when_branch_is_behind_and_skips_untracked_decoys() {
    // The user's branch forked from an older main (trunk advanced past the
    // fork point, so main's tip is NOT an ancestor of the branch tip), and a
    // stale untracked branch points into the branch's own history. `gt track`
    // must offer main — the only usable parent — and not the untracked decoy,
    // which would produce an invalid stack.
    let repo = TestRepo::new("track-behind-main");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");

    repo.git(&["switch", "-q", "-c", "loose"]);
    repo.write("loose.txt", "loose\n");
    repo.commit("loose work");
    repo.write("loose.txt", "loose\nmore\n");
    repo.commit("more loose work");

    // A stale untracked branch inside loose's history (like a worktree base).
    repo.git(&["branch", "decoy", "loose~1"]);

    // Advance main so loose is behind it.
    repo.git(&["switch", "-q", "main"]);
    repo.write("upstream.txt", "upstream\n");
    repo.commit("upstream commit");
    repo.git(&["switch", "-q", "loose"]);

    // main is the only candidate, so it is chosen without prompting.
    let out = repo.gt("track", "");
    assert_success(&out, "gt track");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tracked") && stdout.contains("main"),
        "track should parent the branch onto main; got stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("decoy"),
        "untracked branches must not be offered as parents; got stdout:\n{stdout}"
    );

    // Metadata records main as parent with the true fork point.
    let meta = repo.git(&["cat-file", "-p", "refs/branch-metadata/loose"]);
    assert!(
        meta.contains(r#""parentBranchName":"main""#),
        "expected main as recorded parent; got metadata:\n{meta}"
    );
}

#[test]
fn sync_pauses_when_current_branch_conflicts_then_continue_finishes() {
    // main <- a <- b, with the user ON `b`. We amend `a` directly (bypassing
    // `gt modify`, which would restack b eagerly), so rebasing b's commit
    // onto the amended a conflicts. Then we advance `origin/main` so sync has
    // work to do. Because `b` is the current branch, sync must NOT abort the
    // conflicted rebase: it pauses so the user can resolve manually, and
    // `gt continue` finishes the restack.
    let repo = TestRepo::new("sync-pause-current");
    repo.add_origin();
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    // a touches shared.txt. b extends the same file — the line a touches.
    repo.create_with_gt("a feature", "a", "shared.txt", "a1\n");
    repo.create_with_gt("b feature", "b", "shared.txt", "a1\nb1\n");

    // Amend `a` directly so b's commit (which rewrites shared.txt to
    // "a1\nb1") will conflict against an a-tip that says "A_AMENDED".
    repo.git(&["switch", "-q", "a"]);
    repo.write("shared.txt", "A_AMENDED\n");
    repo.git(&["add", "shared.txt"]);
    repo.git(&["commit", "--amend", "--no-edit", "-q"]);

    advance_origin_main(&repo, "upstream commit", "upstream.txt", "upstream\n");

    repo.git(&["switch", "-q", "b"]);
    let out = repo.gt("sync", "");

    // Sync exits non-zero (paused), with `a` already restacked and clear
    // instructions for `b`.
    assert!(
        !out.status.success(),
        "gt sync must exit non-zero when paused on a conflict; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restacked a"),
        "sync should report `a` as restacked; got stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("`b` cannot be restacked automatically")
            && stdout.contains("gt continue")
            && stdout.contains("gt abort"),
        "sync should pause with resolution instructions for `b`; got stdout:\n{stdout}"
    );

    // The rebase is left in progress, with the resume state file present.
    let git_dir = PathBuf::from(repo.git(&["rev-parse", "--absolute-git-dir"]));
    assert!(
        git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists(),
        "a conflict on the current branch must stay in rebase mode"
    );
    assert!(
        git_dir.join("oh-my-gt").join("state.json").exists(),
        "the resume state file must be kept while paused"
    );

    // Resolve the conflict and continue.
    repo.write("shared.txt", "A_AMENDED\nb1\n");
    repo.git(&["add", "shared.txt"]);
    let out = repo.gt("continue", "");
    assert_success(&out, "gt continue");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restacked b"),
        "continue should report `b` as restacked; got stdout:\n{stdout}"
    );

    // The whole stack now sits on the new trunk: main <- a <- b.
    let new_main = repo.git(&["rev-parse", "main"]);
    assert_eq!(
        repo.git(&["rev-parse", "a^"]),
        new_main,
        "a should now be parented on the advanced main"
    );
    assert_eq!(
        repo.git(&["rev-parse", "b^"]),
        repo.git(&["rev-parse", "a"]),
        "b should now be parented on the restacked a"
    );
    assert_eq!(
        repo.git(&["show", "b:shared.txt"]),
        "A_AMENDED\nb1",
        "b must carry the manually resolved content"
    );

    // Fully cleaned up: no rebase in progress, no state file, back on `b`.
    assert!(
        !git_dir.join("rebase-merge").exists() && !git_dir.join("rebase-apply").exists(),
        "continue must finish the rebase"
    );
    assert!(
        !git_dir.join("oh-my-gt").join("state.json").exists(),
        "continue must clear the state file on completion"
    );
    assert_eq!(
        repo.git(&["symbolic-ref", "--short", "HEAD"]),
        "b",
        "continue must return to the original branch"
    );
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "the working tree must be clean after continue"
    );
}

#[test]
fn sync_reports_off_path_conflicts_and_completes() {
    // Tree:
    //   main
    //     b   (off-path sibling whose commit conflicts with the new trunk)
    //     z   (current; restacks cleanly)
    //
    // `b`'s recorded fork point is still an ancestor of the new main, so it
    // is attempted — but its commit conflicts with the upstream change. Since
    // `b` is NOT the checked-out branch, sync must report it (with a hint to
    // check it out and restack) and finish cleanly without pausing.
    let repo = TestRepo::new("sync-offpath-report");
    repo.add_origin();
    repo.write("shared.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    repo.create_with_gt("b feature", "b", "shared.txt", "base\nb1\n");
    let b_tip_before = repo.git(&["rev-parse", "b"]);
    repo.git(&["switch", "-q", "main"]);
    repo.create_with_gt("z feature", "z", "z.txt", "z\n");

    // Upstream rewrites shared.txt, so replaying b's commit conflicts.
    advance_origin_main(&repo, "upstream commit", "shared.txt", "UPSTREAM\n");

    repo.git(&["switch", "-q", "z"]);
    let out = repo.gt("sync", "");

    assert_success(&out, "gt sync");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restacked z"),
        "sync should report `z` as restacked; got stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("b cannot be restacked automatically")
            && stdout.contains("check it out and run `gt restack`"),
        "sync should tell the user how to resolve `b` manually; got stdout:\n{stdout}"
    );

    // No rebase-in-progress directory remains, and no dangling state file.
    let git_dir = PathBuf::from(repo.git(&["rev-parse", "--absolute-git-dir"]));
    assert!(
        !git_dir.join("rebase-merge").exists() && !git_dir.join("rebase-apply").exists(),
        "an off-path conflict must not leave the repo in a rebase-in-progress state"
    );
    assert!(
        !git_dir.join("oh-my-gt").join("state.json").exists(),
        "sync must clear its state file even when some branches conflicted"
    );

    // `z` was rebased onto the new trunk tip; `b` was left untouched.
    let new_main = repo.git(&["rev-parse", "main"]);
    assert_eq!(
        repo.git(&["rev-parse", "z^"]),
        new_main,
        "z should now be parented on the advanced main"
    );
    assert_eq!(
        repo.git(&["rev-parse", "b"]),
        b_tip_before,
        "b must be left untouched when its restack cannot be applied cleanly"
    );

    // Working tree is clean; status is empty.
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "sync must leave a clean working tree even when some branches conflicted"
    );
}

#[test]
fn sync_defers_current_branch_conflict_until_clean_branches_finish() {
    // Tree:
    //   main
    //     a   (current; its commit conflicts with the new trunk)
    //     z   (off-path sibling; restacks cleanly)
    //
    // `a` sorts before `z`, so it is attempted first and conflicts. Sync must
    // defer it, finish the cleanly-rebaseable `z`, and only then pause on
    // `a`'s conflict for manual resolution.
    let repo = TestRepo::new("sync-defer-current");
    repo.add_origin();
    repo.write("shared.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    repo.create_with_gt("a feature", "a", "shared.txt", "base\na1\n");
    repo.git(&["switch", "-q", "main"]);
    repo.create_with_gt("z feature", "z", "z.txt", "z\n");

    // Upstream rewrites shared.txt, so replaying a's commit conflicts.
    advance_origin_main(&repo, "upstream commit", "shared.txt", "UPSTREAM\n");

    repo.git(&["switch", "-q", "a"]);
    let out = repo.gt("sync", "");

    assert!(
        !out.status.success(),
        "gt sync must exit non-zero when paused on a conflict; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restacked z"),
        "sync should restack `z` before pausing on `a`; got stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("`a` cannot be restacked automatically") && stdout.contains("gt continue"),
        "sync should pause with resolution instructions for `a`; got stdout:\n{stdout}"
    );

    // `z` already sits on the new trunk while `a` is paused mid-conflict.
    let new_main = repo.git(&["rev-parse", "main"]);
    assert_eq!(
        repo.git(&["rev-parse", "z^"]),
        new_main,
        "z must already be restacked when the pause happens"
    );

    // Resolve and continue.
    repo.write("shared.txt", "UPSTREAM\na1\n");
    repo.git(&["add", "shared.txt"]);
    let out = repo.gt("continue", "");
    assert_success(&out, "gt continue");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restacked a"),
        "continue should report `a` as restacked; got stdout:\n{stdout}"
    );

    assert_eq!(
        repo.git(&["rev-parse", "a^"]),
        new_main,
        "a should be parented on the advanced main after continue"
    );
    assert_eq!(
        repo.git(&["symbolic-ref", "--short", "HEAD"]),
        "a",
        "continue must return to the original branch"
    );
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "the working tree must be clean after continue"
    );
}

#[test]
fn restack_skips_stale_branches_off_the_current_path() {
    // Stack: main -> a -> b -> c, with `b`'s recorded parent_branch_revision
    // rewritten so that the recorded fork point is no longer in `a`'s
    // history (simulating an out-of-band rebase under `b`). The user is on
    // `a`, and `gt restack` runs after main has advanced locally.
    //
    // Expected behavior:
    //   * `a` restacks cleanly onto the new main.
    //   * `b` is reported as skipped (stale) up-front and its tip is
    //     untouched.
    //   * `c` is not pulled into the rebase — its parent (b) did not move,
    //     so there is nothing to do for it; its tip is untouched too.
    //   * The repository ends in a clean state with no rebase-in-progress.
    let repo = TestRepo::new("restack-skip-stale");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");

    repo.create_with_gt("a feature", "a", "a.txt", "a\n");
    repo.create_with_gt("b feature", "b", "b.txt", "b\n");
    repo.create_with_gt("c feature", "c", "c.txt", "c\n");
    let b_tip_before = repo.git(&["rev-parse", "b"]);
    let c_tip_before = repo.git(&["rev-parse", "c"]);

    // Rewrite b's recorded fork point to a SHA that is NOT in a's history:
    // the root commit's tree under a fresh commit. That commit is detached
    // from any branch and is guaranteed unreachable from a. We construct it
    // by committing on a throwaway branch, copying its SHA, and discarding
    // the branch.
    repo.git(&["switch", "-q", "-c", "scratch", "main"]);
    repo.write("scratch.txt", "scratch\n");
    repo.git(&["add", "scratch.txt"]);
    repo.git(&["commit", "-q", "-m", "scratch"]);
    let orphan = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["switch", "-q", "main"]);
    repo.git(&["branch", "-D", "-q", "scratch"]);
    repo.write_metadata("b", "a", &orphan);

    // Advance main locally so `a` genuinely needs a restack.
    repo.git(&["switch", "-q", "main"]);
    repo.write("upstream.txt", "upstream\n");
    repo.commit("upstream commit");
    let new_main = repo.git(&["rev-parse", "main"]);

    repo.git(&["switch", "-q", "a"]);
    let out = repo.gt("restack", "");

    assert_success(&out, "gt restack");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restacked a"),
        "gt restack should report `a` as restacked; got stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("skipped (stale):") && stdout.contains(" b"),
        "gt restack should report `b` as skipped (stale); got stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("outdated:"),
        "gt restack should not run `b` and report it outdated; got stdout:\n{stdout}"
    );

    // `a` was rebased onto the new trunk tip.
    let a_parent = repo.git(&["rev-parse", "a^"]);
    assert_eq!(
        a_parent, new_main,
        "a should now be parented on the advanced main"
    );

    // `b` and `c` were NOT touched.
    assert_eq!(
        repo.git(&["rev-parse", "b"]),
        b_tip_before,
        "b must be left at its pre-restack tip when its fork point is stale"
    );
    assert_eq!(
        repo.git(&["rev-parse", "c"]),
        c_tip_before,
        "c must be left untouched when its stale ancestor was skipped"
    );

    // No rebase-in-progress; conflict-resume state is cleared.
    let git_dir = PathBuf::from(repo.git(&["rev-parse", "--absolute-git-dir"]));
    assert!(
        !git_dir.join("rebase-merge").exists() && !git_dir.join("rebase-apply").exists(),
        "restack must not leave the repo in a rebase-in-progress state"
    );
    assert!(
        !git_dir.join("oh-my-gt").join("state.json").exists(),
        "restack must clear its state file even when some branches are skipped"
    );

    // Working tree is clean.
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "restack must leave a clean working tree"
    );
}

#[test]
fn sync_skips_stale_upstream_branches_not_on_current_path() {
    // Tree:
    //   main
    //     a   (current path leaf; the user is on `a`)
    //     b   (off-path sibling whose recorded fork point is stale)
    //
    // Both `a` and `b` are direct children of main; `b` has had its recorded
    // parent_branch_revision rewritten to a SHA outside main's history. The
    // user is on `a`; origin/main advances. `gt sync` must:
    //   * restack `a` cleanly onto the new main.
    //   * NOT attempt `b` — it is off the current path and stale; report it
    //     as skipped (stale).
    //   * leave `b`'s tip untouched.
    let repo = TestRepo::new("sync-skip-stale-off-path");
    repo.add_origin();
    repo.write("base.txt", "base\n");
    repo.commit("root commit");
    repo.git(&["push", "-q", "origin", "main"]);

    // Create `a` off main (clean fork-point), then `b` as a sibling off main.
    repo.create_with_gt("a feature", "a", "a.txt", "a\n");
    repo.git(&["switch", "-q", "main"]);
    repo.create_with_gt("b feature", "b", "b.txt", "b\n");
    let b_tip_before = repo.git(&["rev-parse", "b"]);

    // Cook an orphan commit, then rewrite b's parent_branch_revision to
    // point at it. The orphan is unreachable from main, so b looks stale.
    repo.git(&["switch", "-q", "-c", "scratch", "main"]);
    repo.write("scratch.txt", "scratch\n");
    repo.git(&["add", "scratch.txt"]);
    repo.git(&["commit", "-q", "-m", "scratch"]);
    let orphan = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["switch", "-q", "main"]);
    repo.git(&["branch", "-D", "-q", "scratch"]);
    repo.write_metadata("b", "main", &orphan);

    // Advance origin/main with an unrelated commit.
    advance_origin_main(&repo, "upstream commit", "upstream.txt", "upstream\n");

    repo.git(&["switch", "-q", "a"]);
    let out = repo.gt("sync", "");

    assert_success(&out, "gt sync");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restacked a"),
        "sync should report `a` as restacked; got stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("skipped (stale):") && stdout.contains(" b"),
        "sync should report `b` as skipped (stale); got stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("outdated:"),
        "sync must not attempt `b` and report it outdated; got stdout:\n{stdout}"
    );

    // `b`'s tip is exactly what it was before sync.
    assert_eq!(
        repo.git(&["rev-parse", "b"]),
        b_tip_before,
        "off-path stale branches must not be touched by sync"
    );

    // `a` was rebased onto the new trunk.
    let new_main = repo.git(&["rev-parse", "main"]);
    assert_eq!(
        repo.git(&["rev-parse", "a^"]),
        new_main,
        "a should now be parented on the advanced main"
    );

    // No rebase-in-progress; state file cleared; working tree clean.
    let git_dir = PathBuf::from(repo.git(&["rev-parse", "--absolute-git-dir"]));
    assert!(
        !git_dir.join("rebase-merge").exists() && !git_dir.join("rebase-apply").exists(),
        "sync must not leave the repo in a rebase-in-progress state"
    );
    assert!(
        !git_dir.join("oh-my-gt").join("state.json").exists(),
        "sync must clear its state file even when some branches are skipped"
    );
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "sync must leave a clean working tree"
    );
}

#[test]
fn restack_reconciles_when_user_ends_rebase_with_native_git() {
    // Regression for the desync where gt's state.json outlives the git rebase
    // it represents. gt pauses a restack by leaving a live, detached-HEAD
    // `git rebase` — and git's own status output tells the user to drive it
    // with `git rebase --continue` / `--abort`. If the user follows git's
    // advice instead of gt's, git's rebase state clears while gt's state file
    // lingers: `git status` then reports no rebase even though gt still holds
    // the operation. gt must detect this and reconcile rather than wedge.
    //
    // main <- a <- b, user ON `b`. Amending `a` directly makes replaying b's
    // commit conflict, so `gt restack` pauses on `b`.
    let repo = TestRepo::new("restack-native-rebase-desync");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");

    repo.create_with_gt("a feature", "a", "shared.txt", "a1\n");
    repo.create_with_gt("b feature", "b", "shared.txt", "a1\nb1\n");

    repo.git(&["switch", "-q", "a"]);
    repo.write("shared.txt", "A_AMENDED\n");
    repo.git(&["add", "shared.txt"]);
    repo.git(&["commit", "--amend", "--no-edit", "-q"]);

    repo.git(&["switch", "-q", "b"]);
    let out = repo.gt("restack", "");
    assert!(!out.status.success(), "restack must pause on the conflict");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The pause now steers the user toward gt, away from native git rebase.
    assert!(
        stdout.contains("not `git rebase --continue`"),
        "the conflict message must warn against native git rebase; got:\n{stdout}"
    );

    let git_dir = PathBuf::from(repo.git(&["rev-parse", "--absolute-git-dir"]));
    assert!(
        git_dir.join("rebase-merge").exists(),
        "the pause must leave a live git rebase"
    );

    // The user follows git's advice and ends the rebase out of band. Now git
    // is no longer mid-rebase, but gt's state file remains: the reported
    // desync.
    repo.git(&["rebase", "--abort"]);
    assert!(
        !git_dir.join("rebase-merge").exists(),
        "native abort must clear git's rebase state"
    );
    assert!(
        git_dir.join("oh-my-gt").join("state.json").exists(),
        "gt's state file outlives the rebase — the desync under test"
    );

    // A mutating command is still blocked, but the message must now explain the
    // desync (clean `git status` vs gt's recorded op) rather than read as a bug.
    let out = repo.gt("restack", "");
    assert!(
        !out.status.success(),
        "a recorded op must still block restack"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no longer in progress") && stderr.contains("git status"),
        "the guard must explain the out-of-band end; got stderr:\n{stderr}"
    );

    // `gt continue` reconciles: it notes the out-of-band end and re-drives the
    // plan. Because the user aborted, the rebase runs again and re-pauses on
    // the same conflict — now back under gt's control.
    let out = repo.gt("continue", "");
    assert!(
        !out.status.success(),
        "continue must re-drive and re-pause on the still-unresolved conflict"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ended outside gt"),
        "continue must report that it is reconciling; got:\n{stdout}"
    );
    assert!(
        git_dir.join("rebase-merge").exists(),
        "reconciling must restore a live git rebase to resolve in"
    );

    // Resolve and finish through gt, as intended.
    repo.write("shared.txt", "A_AMENDED\nb1\n");
    repo.git(&["add", "shared.txt"]);
    let out = repo.gt("continue", "");
    assert_success(&out, "gt continue after resolving");

    assert_eq!(
        repo.git(&["rev-parse", "b^"]),
        repo.git(&["rev-parse", "a"]),
        "b must end up parented on the amended a"
    );
    assert_eq!(
        repo.git(&["show", "b:shared.txt"]),
        "A_AMENDED\nb1",
        "b must carry the manually resolved content"
    );
    assert!(
        !git_dir.join("rebase-merge").exists()
            && !git_dir.join("oh-my-gt").join("state.json").exists(),
        "a clean finish must clear both git's rebase and gt's state file"
    );
    assert_eq!(
        repo.git(&["symbolic-ref", "--short", "HEAD"]),
        "b",
        "continue must return to the original branch"
    );
}

#[test]
fn abort_warns_when_rebase_was_ended_out_of_band() {
    // Companion to the reconcile test: if the user finishes the rebase by hand
    // and then runs `gt abort`, abort still restores the pre-operation state
    // (its contract), but must say so rather than silently rewinding work the
    // user just resolved.
    let repo = TestRepo::new("abort-native-rebase-desync");
    repo.write("base.txt", "base\n");
    repo.commit("root commit");

    repo.create_with_gt("a feature", "a", "shared.txt", "a1\n");
    repo.create_with_gt("b feature", "b", "shared.txt", "a1\nb1\n");
    let b_before = repo.git(&["rev-parse", "b"]);

    repo.git(&["switch", "-q", "a"]);
    repo.write("shared.txt", "A_AMENDED\n");
    repo.git(&["add", "shared.txt"]);
    repo.git(&["commit", "--amend", "--no-edit", "-q"]);

    repo.git(&["switch", "-q", "b"]);
    let out = repo.gt("restack", "");
    assert!(!out.status.success(), "restack must pause on the conflict");

    // Resolve and finish the rebase with native git, leaving gt's state behind.
    repo.write("shared.txt", "A_AMENDED\nb1\n");
    repo.git(&["add", "shared.txt"]);
    let cont = repo.git_allow_fail(&["rebase", "--continue"]);
    assert!(
        cont.status.success(),
        "native rebase --continue should finish"
    );

    let git_dir = PathBuf::from(repo.git(&["rev-parse", "--absolute-git-dir"]));
    assert!(
        !git_dir.join("rebase-merge").exists(),
        "native continue must clear git's rebase state"
    );

    let out = repo.gt("abort", "");
    assert_success(&out, "gt abort");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ended outside gt"),
        "abort must announce the out-of-band end; got:\n{stdout}"
    );

    // Abort still honors its contract: `b` is rewound to its pre-operation tip,
    // and the operation state is cleared.
    assert_eq!(
        repo.git(&["rev-parse", "b"]),
        b_before,
        "abort must restore b to its pre-operation tip"
    );
    assert!(
        !git_dir.join("oh-my-gt").join("state.json").exists(),
        "abort must clear the state file"
    );
}
