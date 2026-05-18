# A branch with two children (a DAG); amending it restacks both chains.

commit "root commit" base.txt 0

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

# Second child of alpha.
co alpha
stage gamma.txt c1
step create-gamma create "gamma feature" gamma

co alpha
append alpha.txt a-extra
step modify-alpha modify y n
