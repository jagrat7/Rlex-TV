#!/usr/bin/env bash
# Prints the version a release should get.
#
#   scripts/release-version.sh <current> <patch|minor|major> [exact]
#
# With `exact`, checks it and prints it. Otherwise bumps `current` the way most
# version tools do: bumping a pre-release finishes it rather than skipping past
# it, so `patch` on 0.3.0-beta.1 gives 0.3.0, not 0.3.1.
set -euo pipefail

semver='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'

current=${1:?usage: release-version.sh <current> <patch|minor|major> [exact]}
bump=${2:?usage: release-version.sh <current> <patch|minor|major> [exact]}
exact=${3:-}

if [[ -n $exact ]]; then
  exact=${exact#v}
  if [[ ! $exact =~ $semver ]]; then
    echo "\"$exact\" isn't a version like 1.2.3 or 1.2.3-beta.1" >&2
    exit 1
  fi
  # Semver forbids leading zeros in number-only pre-release parts (beta.01).
  if [[ $exact =~ ^[0-9]+\.[0-9]+\.[0-9]+-(.*\.)?0[0-9]+(\.|$) ]]; then
    echo "\"$exact\" has a number with a leading zero" >&2
    exit 1
  fi
  echo "$exact"
  exit 0
fi

current=${current%%+*}
if [[ ! $current =~ $semver ]]; then
  echo "Cargo.toml's version \"$current\" isn't a version like 1.2.3" >&2
  exit 1
fi
major=${BASH_REMATCH[1]} minor=${BASH_REMATCH[2]} patch=${BASH_REMATCH[3]}
pre=${BASH_REMATCH[4]}

case $bump in
  patch)
    [[ -n $pre ]] || patch=$((patch + 1))
    ;;
  minor)
    if [[ -z $pre || $patch != 0 ]]; then minor=$((minor + 1)); fi
    patch=0
    ;;
  major)
    if [[ -z $pre || $minor != 0 || $patch != 0 ]]; then major=$((major + 1)); fi
    minor=0 patch=0
    ;;
  *)
    echo "bump must be patch, minor or major, not \"$bump\"" >&2
    exit 1
    ;;
esac

echo "$major.$minor.$patch"
