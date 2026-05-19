# A restack that conflicts, is resolved, and is finished with `gt continue`.

commit "root commit" shared.txt L1

write shared.txt fa
git add shared.txt
step create-alpha create "alpha feature" alpha

write shared.txt fa-fb
git add shared.txt
step create-beta create "beta feature" beta

# Amending alpha's line conflicts with beta's edit of the same line.
co alpha
write shared.txt FA
git add shared.txt
step modify-alpha modify

# Resolve the conflict and finish the operation.
resolve shared.txt RESOLVED
step continue-restack continue
