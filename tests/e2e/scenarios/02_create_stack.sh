# Build a three-branch linear stack with `gt create`.

commit "root commit" base.txt 0

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

stage gamma.txt c1
step create-gamma create "gamma feature" gamma
