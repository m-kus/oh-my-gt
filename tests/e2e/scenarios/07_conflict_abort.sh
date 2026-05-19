# A restack that conflicts and is then undone with `gt abort`.

commit "root commit" shared.txt L1

write shared.txt fa
git add shared.txt
step create-alpha create "alpha feature" alpha

write shared.txt fa-fb
git add shared.txt
step create-beta create "beta feature" beta

co alpha
write shared.txt FA
git add shared.txt
step modify-alpha modify

# Abort: branches return to their pre-restack state.
step abort-restack abort
