//! End-to-end suite: replays the scenarios in `tests/e2e/` against the freshly
//! built `gt` binary and diffs each normalized snapshot against `golden/`.
//!
//! The scenario generators and golden data live under `tests/e2e/`; see
//! `tests/e2e/README.md`.

use std::process::Command;

#[test]
fn e2e_replay_matches_golden() {
    let replay = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/e2e/replay.sh");
    let status = Command::new("bash")
        .arg(replay)
        .env("GT_BIN", env!("CARGO_BIN_EXE_gt"))
        .status()
        .expect("failed to run tests/e2e/replay.sh");
    assert!(status.success(), "e2e snapshots diverged from golden/");
}
