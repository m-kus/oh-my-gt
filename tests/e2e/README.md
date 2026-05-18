# oh-my-gt end-to-end tests

Reproducible, offline e2e scenarios. Each scenario builds a throwaway git repo
with a fixed environment, drives `gt` through a sequence of commands, and
snapshots repository state after every step.

## Layout

```
scenarios/      one bash script per scenario (the reproducible generators)
harness/
  lib.sh           helpers: deterministic env, repo setup, step, snapshot
  fake_gh          mock `gh` CLI — no network, state under $FAKE_GH_STATE
  run_scenario.sh  run one scenario against one gt binary
golden/         committed expected snapshots (regenerated, never hand-edited)
run.sh          run scenarios against a chosen gt binary
bless.sh        regenerate golden/ from the current oh-my-gt build
replay.sh       replay scenarios and diff against golden/  (the actual test)
capture_charcoal.sh   regenerate golden/ from charcoal (correctness baseline)
```

## How snapshots stay reproducible

* Fixed `GIT_AUTHOR_*`/`GIT_COMMITTER_*` identities and per-commit fixed dates,
  `TZ=UTC`, `LC_ALL=C` — so commits are deterministic.
* Snapshots are **SHA-normalized**: every commit hash is rewritten to
  `@<message-slug>`. Commit *tree* hashes are kept verbatim — they depend only
  on file content, so a correctly rebased branch has the same tree regardless
  of commit dates or which tool produced it.
* A snapshot therefore compares: commit topology, per-branch tree hashes,
  branch parent metadata, HEAD, working-tree status, rebase-in-progress state,
  and the step's exit code — none of which depend on absolute SHAs.
* `gh` is mocked, so `submit`/`sync` scenarios need no network or auth.

## Running

```sh
cargo build
tests/e2e/bless.sh      # write golden/ from the current build (baseline)
tests/e2e/replay.sh     # replay and diff against golden/  (fails on drift)
```

`cargo test` runs `replay.sh` automatically via `tests/e2e.rs`.

## Golden as a correctness reference (charcoal)

`bless.sh` freezes *current* behavior — it catches regressions. To anchor
golden to a known-correct implementation, capture it from charcoal instead:

```sh
git clone https://github.com/danerwilliams/charcoal /tmp/charcoal
cd /tmp/charcoal && npm install && npm run build
export CHARCOAL_GT=/tmp/charcoal/path/to/gt
tests/e2e/capture_charcoal.sh
```

Charcoal's CLI differs (flags, different prompts); snapshots compare repo
*state*, not wording, so a thin shim translating the bare verbs is usually
enough. Review the diff before committing a charcoal-sourced golden.

## Adding a scenario

Drop a `scenarios/NN_name.sh` script. It runs inside a fresh repo and may use
the helpers from `lib.sh` (`commit`, `stage`, `co`, `step`, `resolve`,
`push_trunk`, `gh_merge`, ...). `step <label> <gt-cmd> [stdin answers...]`
runs a command and records a snapshot. Then re-run `bless.sh`.
