#!/usr/bin/env bash
# Replay every scenario against the current build and diff against golden/.
#
# Exit status is non-zero if any normalized snapshot differs.
set -u

E2E="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GT_BIN="${GT_BIN:-$E2E/../../target/debug/gt}"
GOLDEN="$E2E/golden"

if [ ! -d "$GOLDEN" ] || [ -z "$(ls -A "$GOLDEN" 2>/dev/null)" ]; then
  echo "no golden snapshots — run tests/e2e/bless.sh or capture_charcoal.sh first" >&2
  exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

bash "$E2E/run.sh" "$GT_BIN" "$TMP" || true

fail=0
while IFS= read -r g; do
  rel=${g#"$GOLDEN"/}
  actual="$TMP/$rel"
  if [ ! -f "$actual" ]; then
    echo "MISSING: $rel"
    fail=1
  elif ! diff -u "$g" "$actual" > /dev/null 2>&1; then
    echo "MISMATCH: $rel"
    diff -u "$g" "$actual" | sed 's/^/    /'
    fail=1
  fi
done < <(find "$GOLDEN" -name '*.snap' | sort)

if [ "$fail" -eq 0 ]; then
  echo "e2e replay: all snapshots match golden"
else
  echo "e2e replay: FAILURES (see diffs above)"
fi
exit $fail
