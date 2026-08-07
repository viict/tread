# Cutting a release

The whole process is a signed tag; `release.yml` does the rest. What follows is
mostly the order that avoids re-doing it.

## 1. The version lives in three places

```sh
$EDITOR Cargo.toml            # version = "X.Y.Z"
cargo build                   # updates Cargo.lock's own entry — commit it
$EDITOR README.md             # the `cargo tree` sample: `tread vX.Y.Z`
grep -rn 'tread v0\.' README.md   # nothing may still name the old version
```

`tread --version` prints what `Cargo.toml` says. A tag that disagrees with it
ships a binary that lies about itself, and nothing catches that automatically —
the README sample in particular is a copy no build touches, so it is the one
that gets forgotten. `v0.2.0` mentions in `CLAUDE.md` and `docs/windows.md` are
different: they record *when* something was verified by hand, and must not be
bumped just because a new version shipped.

## 2. Land it on master and wait for green

The bump goes through a pull request like anything else — the branch is named
`release/vX.Y.Z` by convention:

```sh
git commit -am "release: bump to X.Y.Z"
git push -u origin release/vX.Y.Z
gh pr create --base master --title "release: bump to X.Y.Z"
gh pr checks --watch            # ci.yml: six native targets, must be success
gh pr merge --merge
gh run list --branch master --limit 1   # ci.yml again, on the merge commit
```

The run on the PR proves the branch; the run on master proves the commit you
are about to tag, which is a different commit once it is merged. Wait for that
second one.

**Do not tag before CI is green on the commit you intend to tag.** Before
`ci.yml` existed, the Windows suite first ran at publish time — a test that
passed on Linux and failed on Windows reached a tag, and the tag had to be
deleted and re-pushed. That is the failure this step exists to prevent.

## 3. Tag, signed, with a message

```sh
git checkout master && git pull   # tag the merge commit, not the branch head
git tag -m "tread vX.Y.Z" vX.Y.Z
git push origin vX.Y.Z
```

`tag.gpgsign` is on, so a tag is an annotated object and **`-m` is mandatory** —
a bare `git tag vX.Y.Z` fails with `fatal: no tag message?`. Verify before
pushing if you like: `git tag -v vX.Y.Z` should report a good signature.

## 4. Watch it publish

```sh
gh run list --limit 1
gh release view vX.Y.Z --json assets --jq '.assets[].name'
```

Six archives plus `SHA256SUMS`. The workflow builds and tests each target on its
own native runner, so a green run means every artifact was tested on the
architecture it ships for.

## If the release run fails

Land the fix on master the same way — a branch and a PR — then move the tag:

```sh
git tag -d vX.Y.Z && git push origin :refs/tags/vX.Y.Z
git tag -m "tread vX.Y.Z" vX.Y.Z && git push origin vX.Y.Z
```

The publish step replaces assets rather than colliding with existing ones, so a
re-tag is safe and needs no cleanup on the release itself. Moving a tag is not
the rewriting of published history that `CLAUDE.md` forbids: the commits on
master are untouched, and the tag is only a label being corrected. Once a
release is announced, though, the label has been read by other people — from
that point a mistake gets a new version, not a moved tag.

## Afterwards

- `install.sh` and `install.ps1` resolve the newest release, so nothing needs
  updating for the new version to be installable.
- If the release is the first to exercise something on real hardware — a new
  platform, a new installer path — update `docs/windows.md`'s verified /
  not-verified lists. That document is only worth having while it is honest.
