#!/usr/bin/env bash
# Run a single scenario against a `gt` binary, writing snapshots to an out dir.
#
#   run_scenario.sh <scenario.sh> <gt-binary> <out-dir>
set -u

HARNESS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$HARNESS/lib.sh"

SCENARIO="$1"
GT_BIN="$2"
OUT="$3"

mkdir -p "$OUT"
setup_env
repo_init

# The scenario script runs here, in $RUN/repo, using the helpers above.
# shellcheck disable=SC1090
source "$SCENARIO"

cleanup
