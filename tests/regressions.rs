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
