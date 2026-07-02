//! Embed the commit `gt` was built from into the binary so `gt --version` can
//! report it (e.g. `gt 0.1.0 (358ead9 2026-07-01)`). Compare the short SHA to
//! `git rev-parse --short HEAD` to tell at a glance whether an installed `gt` is
//! the latest. Falls back to a bare version when built outside a git checkout.

use std::path::Path;
use std::process::Command;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());

    // Rebuild when HEAD moves (a new commit/checkout) or the index changes (so a
    // dirty tree is reflected), keeping `--version` in step with the source.
    let git = Path::new(&dir).join(".git");
    println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
    println!("cargo:rerun-if-changed={}", git.join("index").display());
    if let Ok(head) = std::fs::read_to_string(git.join("HEAD")) {
        if let Some(rf) = head.strip_prefix("ref: ").map(str::trim) {
            let refpath = git.join(rf);
            if refpath.exists() {
                println!("cargo:rerun-if-changed={}", refpath.display());
            }
        }
    }

    println!("cargo:rustc-env=GT_GIT_INFO={}", git_info(&dir));
}

/// `<short-sha>[-dirty] <commit-date>`, or empty when git is unavailable.
fn git_info(dir: &str) -> String {
    let (Some(sha), Some(date)) = (
        run(dir, &["rev-parse", "--short", "HEAD"]),
        run(dir, &["log", "-1", "--date=short", "--format=%cd"]),
    ) else {
        return String::new();
    };
    let dirty = match run(dir, &["status", "--porcelain"]) {
        Some(s) if !s.is_empty() => "-dirty",
        _ => "",
    };
    format!("{sha}{dirty} {date}")
}

fn run(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}
