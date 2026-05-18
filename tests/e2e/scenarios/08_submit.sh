# Submit a two-branch stack, then submit again to confirm idempotence.

commit "root commit" base.txt 0
push_trunk

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

step submit-first submit
step submit-again submit
