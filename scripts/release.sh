#!/usr/bin/env bash
# Cut a release: bump the version, verify the tree, commit and tag.
#
#   scripts/release.sh 0.2.0            # commit and tag, print the push command
#   scripts/release.sh 0.2.0 --push     # and push it
#   make release-tag VERSION=0.2.0      # the same thing through make
#
# Pushing the tag is what starts .github/workflows/release.yml, which builds
# the seven archives and creates the GitHub release. Nothing here talks to
# GitHub, so a mistake before the push costs a `git tag -d` and a `git reset`.

set -euo pipefail

CARGO="${CARGO:-cargo}"
MAKE="${MAKE:-make}"
BRANCH="${BRANCH:-main}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

usage() {
  cat <<USAGE
usage: scripts/release.sh X.Y.Z [--push]

  X.Y.Z    the new version, without a leading "v"
  --push   push the commit and the tag when everything passed

environment: CARGO, MAKE, BRANCH
USAGE
}

version=""
push=0

while [ $# -gt 0 ]; do
  case "$1" in
    --push)
      push=1
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*)
      echo "error: unknown flag: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [ -n "$version" ]; then
        echo "error: more than one version given" >&2
        exit 2
      fi
      version="$1"
      ;;
  esac
  shift
done

if [ -z "$version" ]; then
  echo "error: no version given" >&2
  usage >&2
  exit 2
fi

version="${version#v}"
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-.0-9A-Za-z]*)$'; then
  echo "error: '${version}' is not a version like 0.2.0" >&2
  exit 2
fi
tag="v${version}"

# --- the tree has to be in a releasable state -------------------------------

branch="$(git rev-parse --abbrev-ref HEAD)"
if [ "$branch" != "$BRANCH" ]; then
  echo "error: on branch '${branch}', releases are cut from '${BRANCH}'" >&2
  exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "error: the working tree has uncommitted changes" >&2
  git status --short >&2
  exit 1
fi

if [ -n "$(git ls-files --others --exclude-standard)" ]; then
  echo "error: the working tree has untracked files" >&2
  git ls-files --others --exclude-standard >&2
  exit 1
fi

if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
  echo "error: tag ${tag} already exists" >&2
  exit 1
fi

current="$(grep -m1 '^version = "' Cargo.toml | sed 's/^version = "\(.*\)"$/\1/')"
if [ "$current" = "$version" ]; then
  echo "error: Cargo.toml is already at ${version}" >&2
  exit 1
fi

echo "==> releasing ${current} -> ${version}"

# --- bump ------------------------------------------------------------------

# Only the first `version = "..."` is touched, which is the [package] one;
# dependency versions further down the file are left alone.
awk -v v="$version" '
  /^version = "/ && !done { sub(/"[^"]*"/, "\"" v "\""); done = 1 }
  { print }
' Cargo.toml > Cargo.toml.new
mv Cargo.toml.new Cargo.toml

echo "==> updating Cargo.lock"
if ! "$CARGO" update --package chaps-cli --offline; then
  # A lockfile that has never been built offline may need a real resolve.
  "$CARGO" build
fi

# --- verify ----------------------------------------------------------------

echo "==> make check test"
"$MAKE" check test

# --- commit and tag --------------------------------------------------------

git add Cargo.toml Cargo.lock
git commit -m "chore: release ${tag}"
git tag -a "$tag" -m "chaps ${tag}"

echo
echo "==> committed and tagged ${tag}"

if [ "$push" -eq 1 ]; then
  echo "==> git push origin ${BRANCH} ${tag}"
  git push origin "$BRANCH" "$tag"
  echo
  echo "the release workflow is building; follow it with:"
  echo "  gh run list --workflow release --limit 1"
else
  echo
  echo "nothing has been pushed. To publish the release:"
  echo
  echo "  git push origin ${BRANCH} ${tag}"
  echo
  echo "To undo instead:"
  echo
  echo "  git tag -d ${tag} && git reset --hard HEAD~1"
fi
