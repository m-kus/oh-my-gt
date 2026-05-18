# Rotate a three-branch stack: pull the bottom branch up to the top.
#   main<-alpha<-beta<-gamma  =>  main<-beta<-gamma<-alpha

commit "root commit" base.txt 0

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

stage gamma.txt c1
step create-gamma create "gamma feature" gamma

# Detach the beta<-gamma sub-stack onto main, leaving alpha alone on main.
co beta
step move-beta-onto-main move main

# Put alpha on top of gamma.
co alpha
step move-alpha-onto-gamma move gamma
