#!/usr/bin/env bash
# Package the built CLI into dist/wsid-<target>-v<version>.<tar.gz|zip>.
# Usage: package-cli.sh <target> <version>
# Prints the archive path on the last line of stdout.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <target> <version>" >&2
  exit 2
fi

target="$1"
version="$2"

case "$target" in
  *-windows-*)
    exe="wsid.exe"
    ext="zip"
    ;;
  *)
    exe="wsid"
    ext="tar.gz"
    ;;
esac

binary="target/${target}/release/${exe}"
if [ ! -f "$binary" ]; then
  echo "missing binary: $binary" >&2
  exit 1
fi

archive="dist/wsid-${target}-v${version}.${ext}"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

cp "$binary" "$stage/$exe"
cp LICENSE README.md "$stage/"
mkdir -p dist

if [ "$ext" = "zip" ]; then
  (cd "$stage" && 7z a -tzip -bso0 "$OLDPWD/$archive" "$exe" LICENSE README.md)
else
  tar -C "$stage" -czf "$archive" "$exe" LICENSE README.md
fi

echo "$archive"
