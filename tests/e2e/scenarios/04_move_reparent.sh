# Three branches off main; re-parent one onto another with `gt move`.

commit "root commit" base.txt 0

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

co main
stage beta.txt b1
step create-beta create "beta feature" beta

co main
stage gamma.txt c1
step create-gamma create "gamma feature" gamma

# Candidates are sorted (alpha, beta, main); pick beta as the new parent.
co gamma
step move-gamma move beta
