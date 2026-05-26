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
        let mut cmd = Command::new(program);
        cmd.current_dir(&self.repo);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd
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
