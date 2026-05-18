# Submit a stack, merge the bottom PR on the remote, then `gt sync`:
# the merged branch is deleted, its child reparented onto trunk and restacked.

commit "root commit" base.txt 0
push_trunk

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

step submit submit

# Merge alpha into trunk on the remote and mark its PR merged.
co main
git merge -q --no-ff alpha -m "merge alpha"
push_trunk
co beta
gh_merge alpha

step sync-merged sync y
