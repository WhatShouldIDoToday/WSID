#!/usr/bin/env bash
# Render the Homebrew formula and cask for a published release.
#
# Usage: render-homebrew.sh <version> <tag> <stable|beta> <output-dir>
# Writes <output-dir>/Formula/<formula>.rb and <output-dir>/Casks/<cask>.rb.
# Requires GH_TOKEN and GH_REPO. The release must already hold SHA256SUMS.
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 <version> <tag> <stable|beta> <output-dir>" >&2
  exit 2
fi

version="$1"
tag="$2"
channel="$3"
out="$4"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
templates="$here/../../packaging/homebrew"

case "$channel" in
  stable)
    formula_tmpl="wsid.rb.tmpl"
    formula_out="wsid.rb"
    cask_tmpl="wsid-gui.rb.tmpl"
    cask_out="wsid-gui.rb"
    ;;
  beta)
    formula_tmpl="wsid-beta.rb.tmpl"
    formula_out="wsid-beta.rb"
    cask_tmpl="wsid-gui-beta.rb.tmpl"
    cask_out="wsid-gui@beta.rb"
    ;;
  *)
    echo "unknown channel: $channel" >&2
    exit 2
    ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
gh release download "$tag" --pattern SHA256SUMS --dir "$work"
sums="$work/SHA256SUMS"

# sha_of <exact asset name> -> checksum
sha_of() {
  local sum
  sum="$(awk -v name="$1" '$2 == name { print $1 }' "$sums")"
  if [ -z "$sum" ]; then
    echo "no checksum for $1" >&2
    exit 1
  fi
  echo "$sum"
}

# Tauri names the macOS disk image after the product, and GitHub may replace spaces with dots.
dmg="$(awk '$2 ~ /_universal\.dmg$/ { print $2 }' "$sums")"
if [ "$(printf '%s\n' "$dmg" | grep -c .)" -ne 1 ]; then
  echo "expected exactly one universal .dmg in SHA256SUMS, found: ${dmg:-none}" >&2
  exit 1
fi

mkdir -p "$out/Formula" "$out/Casks"

render() {
  local tmpl="$1" dest="$2" target var
  cp "$templates/$tmpl" "$dest"
  sed -i.bak -e "s|@@VERSION@@|$version|g" -e "s|@@TAG@@|$tag|g" -e "s|@@DMG@@|$dmg|g" "$dest"
  rm -f "$dest.bak"
  for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
    if grep -q "@@SHA_$(echo "$target" | tr 'a-z-' 'A-Z_')@@" "$dest"; then
      var="$(sha_of "wsid-${target}-v${version}.tar.gz")"
      sed -i.bak "s|@@SHA_$(echo "$target" | tr 'a-z-' 'A-Z_')@@|$var|g" "$dest"
      rm -f "$dest.bak"
    fi
  done
  if grep -q '@@SHA_DMG@@' "$dest"; then
    sed -i.bak "s|@@SHA_DMG@@|$(sha_of "$dmg")|g" "$dest"
    rm -f "$dest.bak"
  fi
  if grep -q '@@' "$dest"; then
    echo "unreplaced placeholder in $dest:" >&2
    grep -n '@@' "$dest" >&2
    exit 1
  fi
}

render "$formula_tmpl" "$out/Formula/$formula_out"
render "$cask_tmpl" "$out/Casks/$cask_out"
echo "Rendered $channel $version into $out"
