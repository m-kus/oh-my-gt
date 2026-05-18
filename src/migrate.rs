//! First-run detection of pre-existing Graphite metadata.
//!
//! oh-my-gt stores metadata in the same `refs/branch-metadata/` blobs the
//! Graphite CLI and charcoal use, so a repo tracked by those tools works with
//! no import step. The newer SQLite-backed Graphite metadata cannot be read;
//! this module just surfaces a one-time notice.

use std::path::Path;

use crate::error::Result;
use crate::{git, meta, trunk};

/// Runs once per repository (guarded by the `migrated` config flag).
pub fn check() -> Result<()> {
    let mut cfg = trunk::load_config()?;
    if cfg.migrated {
        return Ok(());
    }
    cfg.migrated = true;
    trunk::save_config(&cfg)?;

    let legacy = git::run(&["for-each-ref", "--format=%(refname)", meta::META_REF_PREFIX])?;
    let count = legacy.lines().filter(|l| !l.is_empty()).count();
    if count > 0 {
        println!(
            "oh-my-gt: found {count} branch(es) with existing Graphite metadata — using them as-is"
        );
    } else if let Some(file) = sqlite_metadata(&git::git_dir()?) {
        println!(
            "oh-my-gt: detected Graphite's newer metadata ({file}), which cannot be imported;"
        );
        println!("          run `gt track` on each stack to start tracking with oh-my-gt");
    }
    Ok(())
}

/// Name of a modern-Graphite metadata file in `.git`, if present.
fn sqlite_metadata(git_dir: &Path) -> Option<String> {
    for name in [
        "graphite.db",
        ".graphite_repo_config",
        ".graphite_cache_persist",
    ] {
        if git_dir.join(name).exists() {
            return Some(name.to_string());
        }
    }
    None
}
