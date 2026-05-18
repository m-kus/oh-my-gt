# Swap two adjacent branches in a stack: main<-alpha<-beta  =>  main<-beta<-alpha.
# Done with two `gt move`s; each rebases cleanly (disjoint files).

commit "root commit" base.txt 0

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

# Move the top branch down to the trunk, then the old bottom branch on top of it.
co beta
step move-beta-onto-main move main

co alpha
step move-alpha-onto-beta move beta
