#!/usr/bin/env bash
# Run e2e scenarios against a `gt` binary.
#
#   run.sh <gt-binary> <out-dir> [scenario.sh ...]
#
# With no scenarios listed, every file in scenarios/ is run.
set -u

E2E="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

GT_BIN="${1:?usage: run.sh <gt-binary> <out-dir> [scenario...]}"
OUT_ROOT="${2:?usage: run.sh <gt-binary> <out-dir> [scenario...]}"
shift 2

# Absolutize the binary path (scenarios chdir into temp repos).
case "$GT_BIN" in
  /*) ;;
  *)  GT_BIN="$(pwd)/$GT_BIN" ;;
esac
[ -x "$GT_BIN" ] || { echo "gt binary not executable: $GT_BIN" >&2; exit 1; }

scenarios=("$@")
if [ ${#scenarios[@]} -eq 0 ]; then
  scenarios=("$E2E"/scenarios/*.sh)
fi

mkdir -p "$OUT_ROOT"
rc=0
for scn in "${scenarios[@]}"; do
  name=$(basename "$scn" .sh)
  out="$OUT_ROOT/$name"
  rm -rf "$out"
  mkdir -p "$out"
  echo ">> $name"
  if ! bash "$E2E/harness/run_scenario.sh" "$scn" "$GT_BIN" "$out"; then
    echo "   scenario errored: $name" >&2
    rc=1
  fi
done
exit $rc
