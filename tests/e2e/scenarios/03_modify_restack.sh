# Amend the bottom branch of a linear stack; the whole stack must restack.

commit "root commit" base.txt 0

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

stage gamma.txt c1
step create-gamma create "gamma feature" gamma

co alpha
append alpha.txt a-extra
step modify-alpha modify y n
