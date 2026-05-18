# A submitted stack: the bottom PR is merged on the remote and main also gains
# an unrelated commit. `gt sync` must fast-forward main, drop the merged
# branch, and restack the survivors onto the advanced main.

commit "root commit" base.txt 0
push_trunk
root=$(git rev-parse HEAD)

stage alpha.txt a1
step create-alpha create "alpha feature" alpha

stage beta.txt b1
step create-beta create "beta feature" beta

stage gamma.txt c1
step create-gamma create "gamma feature" gamma

step submit submit

# On the remote: merge alpha's PR and land an unrelated commit on main.
co main
git merge -q --no-ff alpha -m "merge alpha"
commit "unrelated hotfix" hotfix.txt h1
push_trunk
# Rewind local main so sync genuinely fast-forwards it from the remote.
git reset -q --hard "$root"
co gamma
gh_merge alpha

step sync-merged sync y
