# AGENTS.md

Guidance for agents working on **oh-my-gt** — the `gt` binary, a small subset of
the Graphite CLI for stacked PRs, written in Rust.

## Golden rule — never destroy data

The single most important constraint. Under **no circumstances** may a change let
an operation cause:

- unrecoverable loss or corruption of local files (the working tree),
- unrecoverable loss of a branch — local or remote,
- corruption of remote branches.

Concretely:

- **Branch deletion** only after an explicit interactive confirmation **and**
  after saving the tip to a backup ref under `refs/oh-my-gt/deleted/`. The only
  place this happens is `gt sync`.
- **Remote pushes** use `--force-with-lease` only — never bare `--force`.
- **Before any rebase or ref rewrite**, write the operation plus a snapshot of
  every affected branch tip and metadata to `.git/oh-my-gt/state.json`, so
  `gt abort` can fully roll back.
- **Never discard uncommitted work**: use a plain `git switch`, never `-f`; never
  `reset --hard` or `git clean`. Mutating commands require a clean working tree
  up front (`git::ensure_clean`).
- `git update-ref -d` is allowed only on `refs/branch-metadata/*` (regenerable
  bookkeeping), never on `refs/heads/*`.
- If you are unsure an operation is reversible, do not do it.

## Verification — run before every commit

- `cargo fmt` — format the code (CI checks with `cargo fmt --check`).
- `cargo clippy --all-targets` — must be completely warning-free.
- `cargo test` — unit tests **and** the e2e replay suite; all must pass.
- If you changed behavior on purpose, re-generate the e2e golden with
  `tests/e2e/bless.sh` and review the diff carefully before committing it.

## Development approach

- **Minimal dependencies.** Only `serde`/`serde_json`. Adding a crate needs a
  strong, stated reason. No `libgit2` — git is driven by shelling out.
- **Only useful functionality.** Build what is actually used; no dead options,
  no speculative features. Commands are bare verbs with interactive prompts —
  no flags.
- **Simple over clever.** Small functions, readable control flow, match the
  style of surrounding code. Comments explain *why*, not *what*.
- **Single chokepoints.** All `git` access goes through `src/git.rs`; all `gh`
  access through `src/gh.rs`. Never spawn a process anywhere else. Arguments are
  always passed as `argv` entries — never built into a shell string.

## Repo map

```
src/
  main.rs      entry point; module list; maps errors to exit codes
  cli.rs       argv dispatch and usage text
  error.rs     GtError
  git.rs       the ONLY place that runs `git`
  gh.rs        the ONLY place that runs `gh`
  graph.rs     StackGraph: the branch DAG + validation, built from git refs
  meta.rs      per-branch metadata blobs at refs/branch-metadata/<branch>
  rebase.rs    restack engine: plan chains, drive them, continue, abort
  state.rs     .git/oh-my-gt/state.json — conflict-resume + rollback snapshot
  trunk.rs     trunk detection + .git/oh-my-gt/config.json
  prompt.rs    interactive stdin prompts
  migrate.rs   first-run detection of pre-existing Graphite metadata
  commands/    one module per subcommand (track, create, modify, ...)
tests/
  e2e.rs       runs the e2e replay suite as a cargo test
  e2e/         scenario scripts, harness, golden snapshots (see its README)
```

## Conventions

- New subcommand: add `src/commands/<name>.rs`, register it in
  `commands/mod.rs` and the `cli.rs` dispatch match.
- Branch metadata is the Graphite-compatible `refs/branch-metadata/` format —
  do not change the schema.
- Load repository state once per command via `StackGraph::load*`; reuse it
  instead of re-spawning git.
