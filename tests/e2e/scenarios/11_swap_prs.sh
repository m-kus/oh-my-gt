# Swap two PRs' places: submit a stack, swap the branches, then submit again.
# The re-submit must re-target each PR's base branch (gh pr edit --base).

commit "root commit" base.txt 0
push_trunk

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

# Initial: main<-alpha(PR#1)<-beta(PR#2).
step submit-initial submit

# Swap to main<-beta<-alpha.
co beta
step move-beta-onto-main move main

co alpha
step move-alpha-onto-beta move beta

# Re-submit: PR#2 base should become main, PR#1 base should become beta.
step submit-after-swap submit
