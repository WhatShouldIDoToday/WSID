#!/usr/bin/env bash
# Keep the newest published beta releases, and delete drafts left behind by cancelled runs.
# Only tags starting with beta-v are ever touched.
#
# Usage: prune-betas.sh [--dry-run] <current-tag>
# Requires GH_TOKEN and GH_REPO.
set -euo pipefail

keep=10
dry_run=false
if [ "${1:-}" = "--dry-run" ]; then
  dry_run=true
  shift
fi
if [ "$#" -ne 1 ]; then
  echo "usage: $0 [--dry-run] <current-tag>" >&2
  exit 2
fi
current="$1"

delete_release() {
  local tag="$1"
  shift
  case "$tag" in
    beta-v*) ;;
    *)
      echo "refusing to delete non-beta tag: $tag" >&2
      exit 1
      ;;
  esac
  if [ "$tag" = "$current" ]; then
    return
  fi
  echo "delete $tag"
  if [ "$dry_run" = false ]; then
    gh release delete "$tag" --yes "$@"
  fi
}

releases="$(gh release list --limit 500 --json tagName,isDraft,publishedAt)"

old_published="$(jq -r --argjson keep "$keep" '
  [.[] | select((.tagName | startswith("beta-v")) and (.isDraft | not))]
  | sort_by(.publishedAt) | reverse | .[$keep:] | .[].tagName' <<< "$releases")"
drafts="$(jq -r '.[] | select((.tagName | startswith("beta-v")) and .isDraft) | .tagName' <<< "$releases")"

while IFS= read -r tag; do
  [ -z "$tag" ] || delete_release "$tag" --cleanup-tag
done <<< "$old_published"

# Draft releases have no git tag yet.
while IFS= read -r tag; do
  [ -z "$tag" ] || delete_release "$tag"
done <<< "$drafts"
