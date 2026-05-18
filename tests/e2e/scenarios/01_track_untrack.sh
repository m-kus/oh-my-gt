# Track existing git branches into a stack, then untrack the middle branch
# and confirm its child is reparented.

commit "root commit" base.txt 0

git switch -q -c alpha
commit "alpha work" alpha.txt a1

git switch -q -c beta
commit "beta work" beta.txt b1

co alpha
step track-alpha track main

co beta
step track-beta track alpha

co alpha
step untrack-alpha untrack y
