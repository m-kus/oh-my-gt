# `gt sync` must not leave the repo in a rebase-in-progress state when a
# branch cannot be restacked cleanly. Branches that conflict are reported as
# outdated; everything else restacks successfully.
#
# Setup: main -> a -> b. We amend `a` directly with `git commit --amend` so
# that gt is not aware (b's recorded fork point now points at the orphaned
# pre-amend tip). main is advanced on the remote with an independent commit.
# When sync replays the chain it should restack `a` onto the new main cleanly
# but fail on `b` (b's commit overlaps with a's amended content) — leaving `b`
# at its old tip.

commit "root commit" base.txt 0
push_trunk
root=$(git rev-parse HEAD)

# a touches shared.txt — sets up the file we will later cause to conflict.
stage shared.txt a1
step create-a create "a feature" a

# b extends shared.txt; the same line a touched is now also touched by b.
write shared.txt "a1
b1"
git add shared.txt
step create-b create "b feature" b

# Amend `a` directly (no gt modify), so b's fork-point metadata is stale and
# rebasing b onto the new a body will conflict on shared.txt.
co a
write shared.txt A_AMENDED
git add shared.txt
bump
git commit --amend --no-edit -q

# Advance main on the remote with a change that does NOT touch shared.txt,
# then rewind local main so sync genuinely fast-forwards it.
co main
commit "remote update" unrelated.txt u1
push_trunk
git reset -q --hard "$root"

co b
step sync-partial sync
