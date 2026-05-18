# oh-my-gt

- small fast Rust tool for stacked PRs. binary called `gt`.
- subset of [Graphite CLI](https://graphite.com/docs/graphite-cli). few deps.
- stack = chain of branches, each on top of last. ship big change as many small PRs.
- `gt` remembers each branch parent. keeps stack rebased.

## why

- Graphite sometimes make you run `git rebase --onto <commit>` by hand. bad.
- `gt` never make you do that. whole stack rebased in one `git rebase --update-refs`.
- re-parent branch = automatic.
- metadata stored same place as Graphite/[charcoal](https://github.com/danerwilliams/charcoal). old repo just work.
- deps = only `serde`. talk to git by running `git`. talk to GitHub by running `gh`.

## need

- git >= 2.38. (need `git rebase --update-refs`.)
- `gh` CLI, logged in. only for `gt submit` and `gt sync`. rest work offline.

## install

```sh
cargo install --path .          # makes binary `gt`
```

## quick start

```sh
git switch main
git add .
gt create                       # asks: commit message, branch name
git add .
gt create                       # stack another on top
gt log                          # see stack
gt submit                       # push stack, open one PR per branch
```

- change low branch, stack keeps up:

```sh
git switch first-branch
git add .
gt modify                        # amend + restack everything above
gt submit                        # update PRs
```

## commands

- no flags. ever. command ask question if it need answer.
- command work on current branch unless said else.

| command       | what it does |
|---------------|--------------|
| `gt track`    | start tracking the current branch (pick its parent) |
| `gt untrack`  | stop tracking the current branch (children are reparented) |
| `gt create`   | create a new branch stacked on the current one from staged changes |
| `gt modify`   | amend the current branch and restack everything above it |
| `gt move`     | re-parent the current branch onto another and restack descendants |
| `gt restack`  | rebase the stack so every branch sits on its parent's tip |
| `gt submit`   | push the stack and create/update its pull requests |
| `gt sync`     | fetch, fast-forward trunk, drop merged/closed branches, restack survivors |
| `gt continue` | resume after resolving rebase conflicts |
| `gt abort`    | undo an in-progress operation, restoring the previous state |
| `gt log`      | show the current stack |

### `gt track`
- start tracking current branch.
- asks: pick parent from ancestor branches. (only one option = no ask.)
- records fork point = merge-base.
- run again on tracked branch = fix or change parent.

### `gt untrack`
- stop tracking current branch.
- branch NOT deleted. only `gt` metadata gone.
- has tracked children? asks first. children move to this branch parent.

### `gt create`
- new branch on top of current. commits staged changes onto it.
- asks: commit message, branch name (default = slug of message).
- nothing staged = error. `git add` first.

### `gt modify`
- amend current branch commit. then restack every branch above.
- asks: stage all changes? edit message?
- conflict = pause. see [conflicts](#conflicts).

### `gt move`
- re-parent current branch onto other branch. restack it + descendants.
- asks: pick new parent. own descendants hidden = no cycle possible.
- replaces every hand `git rebase --onto`.

### `gt restack`
- rebase whole stack so each branch sits on parent tip.
- nothing drifted = no-op.
- use after pull or any outside rebase.

### `gt submit`
- push downstack (trunk..current). make or update one PR per branch.
- push = `--force-with-lease` (restack rewrites history).
- no PR yet = create one, title from commit.
- PR exists = update it, fix base branch to match stack order. (swap branches just work.)
- idempotent. run many times. safe.
- needs remote + `gh`. refuses if stack needs restack.

### `gt sync`
- `git fetch`. fast-forward trunk.
- find branches with PR merged or closed (or commits already on trunk).
- ask per branch: delete?
- before delete: save tip to `refs/oh-my-gt/deleted/<branch>`, print restore command. delete always reversible.
- children of deleted branch move to nearest survivor.
- restack what remains.

### `gt continue` / `gt abort`
- resume or undo a paused operation. see below.

### `gt log`
- print stack as tree. mark current branch. mark branch needing restack.
- also list untracked + broken branches.

### `gt help` / `gt version`
- show usage / version.

## conflicts

- restack hits conflict (from `modify`, `move`, `restack`, `sync`) = `gt` stops. tells which files.

```sh
git add <files>                  # after fixing them
gt continue                      # finish rest of operation
# or:
gt abort                         # throw it away, restore every branch
```

- operation paused = other commands refuse until `gt continue` or `gt abort`.

## how it works

- **metadata.** each tracked branch = small JSON blob at `refs/branch-metadata/<branch>`. holds parent branch, fork commit, PR info. same format as Graphite/charcoal.
- **rebasing.** restack = set of linear chains. each chain = one `git rebase --update-refs --onto <new-base> <old-base> <tip>`. moves all branch refs in one pass. branch-with-many-children = many chains, parents first.
- **pause + recover.** before touching any branch, write operation + snapshot of every branch tip/metadata to `.git/oh-my-gt/state.json`. `gt continue` resumes mid-plan. `gt abort` rolls everything back from snapshot.
- **safety.** never deletes working-tree files. never force-discards uncommitted work. only branch delete = `gt sync`, asks first + backup ref. only runs `git` and `gh`. only network = your remote + your GitHub.
- **migrate from Graphite.** old Graphite/charcoal repo = nothing to do, metadata read directly. Graphite new SQLite metadata = can't import, `gt` tells you to re-run `gt track`.

## development

```sh
cargo build
cargo test                       # unit tests + end-to-end replay suite
```

- e2e tests in `tests/e2e/`. reproducible scripts drive `gt` through throwaway repos. diff SHA-normalized snapshots vs committed golden. see `tests/e2e/README.md`.
