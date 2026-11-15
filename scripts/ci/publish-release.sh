#!/usr/bin/env bash
# Add SHA256SUMS to a draft release, then publish it.
#
# Usage: publish-release.sh <tag> <latest|prerelease>
# Requires GH_TOKEN and GH_REPO.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <tag> <latest|prerelease>" >&2
  exit 2
fi

tag="$1"
case "$2" in
  latest) publish_args=(--draft=false --latest) ;;
  prerelease) publish_args=(--draft=false --prerelease --latest=false) ;;
  *)
    echo "unknown channel: $2" >&2
    exit 2
    ;;
esac

dist="$(mktemp -d)"
trap 'rm -rf "$dist"' EXIT

gh release download "$tag" --dir "$dist"
rm -f "$dist/SHA256SUMS"
if [ -z "$(ls -A "$dist")" ]; then
  echo "No assets found on release $tag." >&2
  exit 1
fi

(cd "$dist" && sha256sum -- * > SHA256SUMS)
cat "$dist/SHA256SUMS"
gh release upload "$tag" "$dist/SHA256SUMS" --clobber
gh release edit "$tag" "${publish_args[@]}"
