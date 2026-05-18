# Branches that carry more than one commit each. Restack (via modify) and sync
# must replay every commit on a branch, not merely its tip.

commit "root commit" base.txt 0
push_trunk

# alpha: two commits.
stage alpha1.txt a1
step create-alpha create "alpha first" alpha
commit "alpha second" alpha2.txt a2

# beta stacked on alpha: three commits.
stage beta1.txt b1
step create-beta create "beta first" beta
commit "beta second" beta2.txt b2
commit "beta third" beta3.txt b3

step submit submit

# Amend alpha's tip commit; beta's three commits must all restack.
co alpha
append alpha2.txt a2-extra
step modify-alpha modify y n

# Merge alpha (two commits) on the remote, then sync.
co main
git merge -q --no-ff alpha -m "merge alpha"
push_trunk
co beta
gh_merge alpha

step sync-merged sync y
