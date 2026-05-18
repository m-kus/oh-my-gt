#!/usr/bin/env bash
# Capture golden snapshots from charcoal (the reference `gt` implementation).
#
# This produces a *correctness* baseline: charcoal defines the expected
# resulting repo state, and `replay.sh` then checks oh-my-gt against it.
#
# Requirements:
#   - node + npm, to install charcoal
#   - a charcoal `gt` binary; set CHARCOAL_GT to its path
#
# Install charcoal (one-time):
#   git clone https://github.com/danerwilliams/charcoal /tmp/charcoal
#   cd /tmp/charcoal && npm install && npm run build
#   export CHARCOAL_GT=/tmp/charcoal/apps/cli/dist/src/index.js   # or the bin
#
# NOTE: charcoal's CLI surface differs from oh-my-gt's (it takes flags and has
# different prompts). Scenarios here drive oh-my-gt's bare-verb interface. If
# charcoal needs adapting, edit scenarios or wrap CHARCOAL_GT in a shim that
# translates the bare verbs. Snapshots are SHA-normalized and compare repo
# *state*, so wording differences do not matter — only the resulting tree
# topology, branch parents and metadata.
set -eu

E2E="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GT_BIN="${CHARCOAL_GT:?set CHARCOAL_GT to a charcoal gt binary (see comments)}"

rm -rf "$E2E/golden"
bash "$E2E/run.sh" "$GT_BIN" "$E2E/golden"
find "$E2E/golden" -name '*.stdout' -delete

echo
echo "charcoal golden snapshots written to tests/e2e/golden/"
echo "review the diff against the previous baseline before committing."
