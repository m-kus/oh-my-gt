# Move a middle branch — carrying its descendant — onto the trunk, splitting
# one stack into two:  main<-alpha<-beta<-gamma  =>  main<-alpha , main<-beta<-gamma.

commit "root commit" base.txt 0

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

stage gamma.txt c1
step create-gamma create "gamma feature" gamma

# Moving beta also brings gamma along.
co beta
step move-beta-onto-main move main
