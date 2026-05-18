//! Trunk-branch detection and the per-repo config file under `.git/oh-my-gt/`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{GtError, Result};
use crate::git;

/// Persisted per-repository settings.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk: Option<String>,
    /// Whether the one-time legacy-metadata import has run.
    #[serde(default)]
    pub migrated: bool,
}

/// Directory holding oh-my-gt's repo-local state (inside `.git`).
pub fn state_dir() -> Result<PathBuf> {
    Ok(git::git_dir()?.join("oh-my-gt"))
}

fn config_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("config.json"))
}

pub fn load_config() -> Result<Config> {
    match std::fs::read(config_path()?) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        Err(_) => Ok(Config::default()),
    }
}

pub fn save_config(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(cfg)?)?;
    Ok(())
}

/// Resolve the trunk branch, caching the answer in the repo config.
pub fn resolve(heads: &HashMap<String, String>) -> Result<String> {
    let mut cfg = load_config()?;
    if let Some(t) = &cfg.trunk {
        if heads.contains_key(t) {
            return Ok(t.clone());
        }
    }
    let detected = detect(heads)?;
    cfg.trunk = Some(detected.clone());
    save_config(&cfg)?;
    Ok(detected)
}

fn detect(heads: &HashMap<String, String>) -> Result<String> {
    // 1. The remote's default branch, if a remote is configured.
    if let Ok(out) = git::run(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        if let Some(name) = out.strip_prefix("origin/") {
            if heads.contains_key(name) {
                return Ok(name.to_string());
            }
        }
    }
    // 2. Conventional names.
    for cand in ["main", "master"] {
        if heads.contains_key(cand) {
            return Ok(cand.to_string());
        }
    }
    // 3. A single-branch repository.
    if heads.len() == 1 {
        return Ok(heads.keys().next().unwrap().clone());
    }
    Err(GtError::State(
        "cannot determine the trunk branch; create `main`/`master` or set origin/HEAD".into(),
    ))
}
