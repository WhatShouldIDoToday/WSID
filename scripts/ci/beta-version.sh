#!/usr/bin/env bash
# Compute the beta version from the workspace version and GITHUB_RUN_NUMBER: 0.2.0 becomes
# 0.2.1-beta.<run>. Writes `version` and `tag` to GITHUB_OUTPUT when it is set.
#
# Usage: beta-version.sh [--print]
# Without --print, the version is also written to the manifests in the working tree. Beta versions
# are never committed.
set -euo pipefail

if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ "$1" != "--print" ]; }; then
  echo "usage: $0 [--print]" >&2
  exit 2
fi

run="${GITHUB_RUN_NUMBER:?GITHUB_RUN_NUMBER must be set}"
current="$(perl -ne 'if (/^\[workspace\.package\]/) { $in = 1 } elsif ($in && /^version = "([^"]+)"/) { print $1; exit }' Cargo.toml)"

if ! [[ "$current" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "unexpected workspace version: '$current'" >&2
  exit 1
fi

export NEW_VERSION="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.$((BASH_REMATCH[3] + 1))-beta.${run}"
tag="beta-v${NEW_VERSION}"

if [ "${1:-}" != "--print" ]; then
  perl -pi -e 'if (/^\[workspace\.package\]/) { $in = 1 } if ($in && !$done && s/^version = "[^"]*"/version = "$ENV{NEW_VERSION}"/) { $done = 1 }' Cargo.toml
  perl -pi -e 's/^  "version": "[^"]*"/  "version": "$ENV{NEW_VERSION}"/' \
    crates/wsid-gui/tauri.conf.json crates/wsid-gui/ui/package.json
  cargo update --workspace
fi

echo "version=$NEW_VERSION"
echo "tag=$tag"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "version=$NEW_VERSION"
    echo "tag=$tag"
  } >> "$GITHUB_OUTPUT"
fi
