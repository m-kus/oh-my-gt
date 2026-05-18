//! Per-branch stack metadata, stored Graphite-compatibly as a JSON blob at
//! `refs/branch-metadata/<branch>`.
//!
//! The schema and the camelCase field names match what the Graphite CLI and
//! charcoal write, so existing repos round-trip without a migration step.

use serde::{Deserialize, Serialize};

use crate::error::{GtError, Result};
use crate::git;

/// Ref namespace under which one metadata blob is kept per tracked branch.
pub const META_REF_PREFIX: &str = "refs/branch-metadata/";

/// Stack metadata for a single branch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchMetadata {
    /// Name of this branch's parent in the stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_branch_name: Option<String>,
    /// Commit the branch was forked from — the rebase base for restacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_branch_revision: Option<String>,
    /// Associated pull request, once submitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_info: Option<PrInfo>,
}

/// Pull-request details tracked alongside a branch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// `OPEN` | `MERGED` | `CLOSED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl BranchMetadata {
    pub fn new(parent: &str, parent_revision: &str) -> Self {
        BranchMetadata {
            parent_branch_name: Some(parent.to_string()),
            parent_branch_revision: Some(parent_revision.to_string()),
            pr_info: None,
        }
    }
}

/// The metadata ref name for a branch.
pub fn meta_ref(branch: &str) -> String {
    format!("{META_REF_PREFIX}{branch}")
}

/// Deserialize a metadata blob.
pub fn parse(blob: &str) -> Result<BranchMetadata> {
    Ok(serde_json::from_str(blob)?)
}

/// Serialize metadata to its on-disk JSON form.
pub fn serialize(meta: &BranchMetadata) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(meta)?)
}

/// Read a single branch's metadata, or `None` when the branch is untracked.
pub fn read(branch: &str) -> Result<Option<BranchMetadata>> {
    let out = git::run_allow_fail(&["cat-file", "-p", &meta_ref(branch)])?;
    if out.code != 0 {
        return Ok(None);
    }
    Ok(Some(parse(&out.stdout)?))
}

/// Write (creating or replacing) a branch's metadata blob.
pub fn write(branch: &str, meta: &BranchMetadata) -> Result<()> {
    let json = serialize(meta)?;
    let blob = git::run_with_stdin(&["hash-object", "-w", "--stdin"], &json)?;
    git::run(&["update-ref", &meta_ref(branch), &blob])?;
    Ok(())
}

/// The blob SHA backing a branch's metadata ref, for rollback snapshots.
pub fn blob_sha(branch: &str) -> Result<Option<String>> {
    git::rev_parse_blob(&meta_ref(branch))
}

/// Point a metadata ref directly at an existing blob SHA (used by `abort`).
pub fn restore_ref(branch: &str, blob: &str) -> Result<()> {
    git::run(&["update-ref", &meta_ref(branch), blob])?;
    Ok(())
}

/// Delete a branch's metadata ref. A missing ref is not an error.
pub fn delete(branch: &str) -> Result<()> {
    let r = meta_ref(branch);
    let out = git::run_allow_fail(&["update-ref", "-d", &r])?;
    if out.code != 0 && git::ref_exists(&r)? {
        return Err(GtError::Git(format!("failed to delete {r}: {}", out.stderr)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_full() {
        let m = BranchMetadata {
            parent_branch_name: Some("main".into()),
            parent_branch_revision: Some("a1b2c3d4".into()),
            pr_info: Some(PrInfo {
                number: Some(42),
                base: Some("main".into()),
                url: Some("https://github.com/x/y/pull/42".into()),
                title: Some("Add a thing".into()),
                body: Some("line one\nline two \"quoted\" — ünïcode".into()),
                state: Some("OPEN".into()),
            }),
        };
        let bytes = serialize(&m).unwrap();
        let back = parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn camelcase_keys_and_skipped_nones() {
        let m = BranchMetadata::new("trunk", "deadbeef");
        let s = String::from_utf8(serialize(&m).unwrap()).unwrap();
        assert!(s.contains("\"parentBranchName\":\"trunk\""));
        assert!(s.contains("\"parentBranchRevision\":\"deadbeef\""));
        // `prInfo` is None and must not be emitted.
        assert!(!s.contains("prInfo"));
    }

    #[test]
    fn parses_charcoal_blob_with_extra_fields() {
        // Charcoal writes extra prInfo fields we do not model; they must be ignored.
        let blob = r#"{
            "parentBranchName": "main",
            "parentBranchRevision": "abc123",
            "prInfo": {
                "number": 7,
                "state": "MERGED",
                "reviewDecision": "APPROVED",
                "isDraft": false
            }
        }"#;
        let m = parse(blob).unwrap();
        assert_eq!(m.parent_branch_name.as_deref(), Some("main"));
        let pr = m.pr_info.unwrap();
        assert_eq!(pr.number, Some(7));
        assert_eq!(pr.state.as_deref(), Some("MERGED"));
    }

    #[test]
    fn parses_blob_with_absent_pr_info() {
        let m = parse(r#"{"parentBranchName":"main","parentBranchRevision":"x"}"#).unwrap();
        assert!(m.pr_info.is_none());
    }
}
