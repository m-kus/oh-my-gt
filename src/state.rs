//! The conflict-resume state file at `.git/oh-my-gt/state.json`.
//!
//! It is written before any branch ref is mutated and removed on clean
//! completion. While present, ordinary commands refuse to run; only
//! `gt continue` and `gt abort` are allowed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::trunk;

/// Pre-operation state of one branch, used to roll back on `abort`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSnapshot {
    pub branch: String,
    pub tip: String,
    /// The metadata blob SHA, or `None` if the branch had no metadata.
    #[serde(default)]
    pub metadata_blob: Option<String>,
}

/// One linear segment of the stack, rebased by a single `git rebase` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chain {
    /// Branches in the chain, parent-most first; the last is the chain tip.
    pub branches: Vec<String>,
    /// The branch the chain head is parented onto.
    pub parent: String,
    /// The recorded fork point of the chain head — the rebase `--onto` upstream.
    pub old_base: String,
    #[serde(default)]
    pub done: bool,
}

impl Chain {
    /// The branch at the tip of the chain — what `git rebase` operates on.
    pub fn tip(&self) -> &str {
        self.branches.last().expect("a chain is never empty")
    }
}

/// The whole in-progress operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpState {
    pub version: u32,
    pub operation: String,
    pub trunk: String,
    /// Branch to return to once the operation completes.
    #[serde(default)]
    pub return_branch: Option<String>,
    pub snapshot: Vec<BranchSnapshot>,
    pub chains: Vec<Chain>,
    /// Index of the chain currently being processed.
    pub current_chain: usize,
}

fn path() -> Result<PathBuf> {
    Ok(trunk::state_dir()?.join("state.json"))
}

/// Whether an operation is currently in progress.
pub fn exists() -> bool {
    path().map(|p| p.exists()).unwrap_or(false)
}

/// Load the in-progress operation, if any.
pub fn load() -> Result<Option<OpState>> {
    let p = path()?;
    match std::fs::read(&p) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(_) => Ok(None),
    }
}

/// Persist the in-progress operation.
pub fn save(st: &OpState) -> Result<()> {
    let p = path()?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(st)?)?;
    Ok(())
}

/// Remove the state file (operation complete or aborted).
pub fn clear() -> Result<()> {
    let p = path()?;
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}
