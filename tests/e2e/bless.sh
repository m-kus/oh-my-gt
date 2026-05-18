#!/usr/bin/env bash
# Regenerate golden snapshots from the current oh-my-gt build.
#
# Use this to accept current behavior as the regression baseline. To capture a
# correctness reference instead, use capture_charcoal.sh.
set -eu

E2E="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GT_BIN="${GT_BIN:-$E2E/../../target/debug/gt}"

rm -rf "$E2E/golden"
bash "$E2E/run.sh" "$GT_BIN" "$E2E/golden"

# .stdout files are tool-specific wording and are not part of the golden.
find "$E2E/golden" -name '*.stdout' -delete

echo
echo "golden snapshots written to tests/e2e/golden/"
