# Shared helpers for oh-my-gt e2e scenarios.
#
# A scenario is a plain bash script (sourced by run_scenario.sh) that builds a
# repo with these helpers and runs `gt` commands through `step`. After every
# step a normalized snapshot of repository state is written to $OUT.
#
# Snapshots are SHA-normalized: commit hashes are replaced with @<message-slug>
# labels, so they compare equal across tools and across runs even though
# absolute commit SHAs depend on commit dates. Tree SHAs are kept verbatim —
# they are content-only and therefore deterministic.

# ── deterministic environment ───────────────────────────────────────────────

setup_env() {
  export TZ=UTC LC_ALL=C
  export GIT_AUTHOR_NAME="E2E Bot"   GIT_AUTHOR_EMAIL="e2e@example.test"
  export GIT_COMMITTER_NAME="E2E Bot" GIT_COMMITTER_EMAIL="e2e@example.test"
  export GIT_EDITOR=true             # never block on an editor
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
  DATE_N=0
  bump
}

# Advance the deterministic commit clock. `gt` and `git` both read these.
bump() {
  DATE_N=$((DATE_N + 1))
  local d
  d=$(printf '2026-01-01T00:%02d:00' "$DATE_N")
  export GIT_AUTHOR_DATE="$d" GIT_COMMITTER_DATE="$d"
}

# ── repository setup ────────────────────────────────────────────────────────

# repo_init: create a bare origin + working repo, with the mock gh on PATH.
repo_init() {
  RUN=$(mktemp -d)
  export FAKE_GH_STATE="$RUN/ghstate"
  mkdir -p "$RUN/bin"
  cp "$HARNESS/fake_gh" "$RUN/bin/gh"
  chmod +x "$RUN/bin/gh"
  export PATH="$RUN/bin:$PATH"

  git init -q --bare "$RUN/origin.git"
  mkdir -p "$RUN/repo"
  cd "$RUN/repo" || exit 1
  git init -q -b main .
  git remote add origin "$RUN/origin.git"
  STEP=0
}

cleanup() {
  cd / || true
  [ -n "${RUN:-}" ] && rm -rf "$RUN"
}

# ── scenario building blocks ────────────────────────────────────────────────

# commit <message> [file] [content] — make a deterministic commit.
commit() {
  local msg=$1 file=${2:-file_$DATE_N.txt} content=${3:-content $DATE_N}
  printf '%s\n' "$content" > "$file"
  git add -A
  bump
  git commit -q -m "$msg"
}

# stage <file> <content> — write and `git add` a file (for `gt create`).
stage() {
  printf '%s\n' "$2" > "$1"
  git add "$1"
}

# write <file> <content> — write a file without staging (for conflicts).
write() {
  printf '%s\n' "$2" > "$1"
}

# append <file> <line> — append a line without staging.
append() {
  printf '%s\n' "$2" >> "$1"
}

# resolve <file> <content> — write and stage a conflict resolution.
resolve() {
  printf '%s\n' "$2" > "$1"
  git add "$1"
}

# co <branch> — switch branches.
co() {
  git switch -q "$1"
}

# push_trunk — publish main to the origin remote.
push_trunk() {
  git push -q origin main
}

# gh_merge <branch> — mark the mock PR for <branch> as MERGED.
gh_merge() {
  local f="$FAKE_GH_STATE/pr_$1"
  [ -f "$f" ] || return 0
  grep -v '^STATE=' "$f" > "$f.tmp"
  echo "STATE=MERGED" >> "$f.tmp"
  mv "$f.tmp" "$f"
}

# step <label> <gt-subcommand> [stdin-answers...] — run a gt command and snapshot.
step() {
  local label=$1 cmd=$2
  shift 2
  bump
  local input="" a
  for a in "$@"; do
    input+="$a"$'\n'
  done
  printf '%s' "$input" | "$GT_BIN" "$cmd" > "$RUN/last.stdout" 2>&1
  local code=$?
  snapshot "$label" "$code"
}

# ── snapshots ───────────────────────────────────────────────────────────────

# slugify <text> — a filesystem/label-safe slug of a commit message.
slugify() {
  local s
  s=$(printf '%s' "$1" | tr '[:upper:] ' '[:lower:]-' | tr -cd 'a-z0-9-')
  printf '%s' "${s:-commit}"
}

# snapshot <label> <exit-code> — write a normalized state file to $OUT.
snapshot() {
  local label=$1 code=$2
  STEP=$((STEP + 1))
  local out
  out=$(printf '%s/%02d-%s.snap' "$OUT" "$STEP" "$label")

  # Build a sed program mapping every commit SHA -> @<label>. Labels come from
  # the commit message; when two commits share a message (e.g. a commit and its
  # amended self) the tree prefix disambiguates them — still fully deterministic.
  # (Kept free of bash-4 associative arrays so it runs on macOS's bash 3.2.)
  local sedf="$RUN/labels.sed"
  local commits="$RUN/commits.tsv"
  : > "$sedf"
  : > "$commits"
  local h t s slug cnt
  while IFS=$'\t' read -r h t s; do
    slug=$(slugify "$s")
    printf '%s\t%s\t%s\n' "$h" "$t" "$slug" >> "$commits"
  done < <(git log --all --pretty='%H%x09%T%x09%s' 2>/dev/null)
  while IFS=$'\t' read -r h t slug; do
    cnt=$(cut -f3 "$commits" | grep -cxF "$slug")
    if [ "$cnt" -gt 1 ]; then
      printf 's/%s/@%s-%s/g\n' "$h" "$slug" "${t:0:6}" >> "$sedf"
    else
      printf 's/%s/@%s/g\n' "$h" "$slug" >> "$sedf"
    fi
  done < "$commits"

  {
    echo "# step: $label (exit=$code)"

    local head
    head=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || echo '(detached)')
    echo "HEAD: $head"

    if [ -d .git/rebase-merge ] || [ -d .git/rebase-apply ]; then
      echo "rebase: IN-PROGRESS"
      echo "conflicts:"
      git diff --name-only --diff-filter=U 2>/dev/null | sort | sed 's/^/  /'
    else
      echo "rebase: none"
    fi

    echo "status:"
    git status --porcelain 2>/dev/null | sort | sed 's/^/  /'

    echo "branches:"
    git for-each-ref --format='%(refname:short) %(objectname)' refs/heads/ \
      | sort | while read -r b hh; do
        echo "  $b -> $hh tree=$(git rev-parse "$hh^{tree}")"
      done

    echo "commits:"
    git log --all --pretty='%H tree=%T parents=[%P] %s' 2>/dev/null \
      | sort -t']' -k2

    echo "metadata:"
    git for-each-ref --format='%(refname)' refs/branch-metadata/ \
      | sort | while read -r r; do
        echo "  ${r#refs/branch-metadata/}: $(git cat-file -p "$r" 2>/dev/null)"
      done
  } | sed -f "$sedf" > "$out"

  # Keep raw stdout alongside for debugging; it is not diffed across tools.
  cp "$RUN/last.stdout" "${out%.snap}.stdout" 2>/dev/null || true
}
