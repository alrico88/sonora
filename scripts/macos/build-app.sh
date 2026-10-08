#!/usr/bin/env bash
# Assembles Sonora.app from a release build, so a macOS machine without Nix can produce a
# double-clickable app with its icon. It needs cmake and Xcode's Metal toolchain; inside
# `nix develop` both are already on PATH.
#
#   scripts/macos/build-app.sh [--no-build] [--open] [--version X.Y.Z] [--out DIR]
#
# The bundle replaces /Applications/Sonora.app, where Launchpad and Spotlight find it, and is
# signed ad-hoc so macOS treats it as a normal app. It is not notarised, so Finder may ask the
# first time: right-click the app and Open.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

build=1
launch=0
version=""
out="/Applications/Sonora.app"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --no-build) build=0 ;;
    --open) launch=1 ;;
    --version) version="${2:?--version needs a value}"; shift ;;
    --out) out="${2:?--out needs a value}"; shift ;;
    -h | --help)
      sed -n '2,11p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      exit 2
      ;;
  esac
  shift
done

if [ -z "$version" ]; then
  version="$(awk -F '"' '/^\[workspace.package\]/ { pkg = 1; next } pkg && /^version/ { print $2; exit }' Cargo.toml)"
fi
[ -n "$version" ] || {
  echo "cannot read the workspace version from Cargo.toml" >&2
  exit 1
}

if [ "$build" = 1 ]; then
  cargo build --locked --release --package sonora
fi

binary="target/release/sonora"
[ -x "$binary" ] || {
  echo "missing $binary; run without --no-build" >&2
  exit 1
}

rm -rf "$out"
mkdir -p "$out/Contents/MacOS" "$out/Contents/Resources"
cp "$binary" "$out/Contents/MacOS/sonora"
cp assets/macos/sonora.icns "$out/Contents/Resources/sonora.icns"
sed "s/@VERSION@/$version/g" assets/macos/Info.plist > "$out/Contents/Info.plist"

# Ad-hoc signing is enough for a locally built app; without it some APIs (keychain,
# notifications) treat the bundle as unsigned.
codesign --force --deep --sign - "$out" >/dev/null 2>&1 ||
  echo "warning: codesign failed; the app still runs from Finder" >&2

echo "built $out ($version)"
if [ "$launch" = 1 ]; then
  open "$out"
fi
