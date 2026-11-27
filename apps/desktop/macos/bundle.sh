#!/usr/bin/env bash
# Package a macOS build of Mothr for a release:
#
#   apps/desktop/macos/bundle.sh <version> <arch> <mothr binary> [out dir]
#
# <arch> is arm64 (Apple silicon) or x64 (Intel). Writes, in <out dir> (default .):
#   mothr_<version>_macos_<arch>.tar.gz   one folder mothr_<version>_macos_<arch>/ with Mothr.app;
#                                         install.sh and the in-app updater install it
#   mothr_<version>_macos_<arch>.dmg      Mothr.app and an Applications link, to drag across
# The tarball's name and shape are what installed copies update from: keep them.
#
# The app is signed ad hoc (Apple silicon only runs signed code), not with a Developer ID, so a
# disk image downloaded in a browser needs a right-click → Open (or Privacy & Security → Open
# Anyway) the first time. install.sh downloads with curl, which doesn't mark the app as
# downloaded, so it opens directly. Without codesign/hdiutil (on Linux) only the tarball is made,
# unsigned.

set -euo pipefail

[ $# -ge 3 ] || { echo "usage: $0 <version> <arch> <mothr binary> [out dir]" >&2; exit 2; }
version="$1" arch="$2" binary="$3" out="${4:-.}"
case "$arch" in arm64|x64) ;; *) echo "arch must be arm64 or x64, not $arch" >&2; exit 2 ;; esac
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
name="mothr_${version}_macos_${arch}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
app="$work/$name/Mothr.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/licenses"
install -m 755 "$binary" "$app/Contents/MacOS/mothr"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
install -m 644 "$here/Mothr.icns" "$app/Contents/Resources/Mothr.icns"
install -m 644 "$repo/LICENSE" "$app/Contents/Resources/LICENSE"
install -m 644 "$repo/apps/desktop/assets/fonts/OFL.txt" "$app/Contents/Resources/licenses/OFL-Geist.txt"
install -m 644 "$repo/apps/desktop/assets/logos/NOTICE.md" "$app/Contents/Resources/licenses/NOTICE-logos.md"

if command -v plutil >/dev/null; then
    plutil -lint "$app/Contents/Info.plist"
fi
if command -v codesign >/dev/null; then
    codesign --force --sign - --timestamp=none "$app"
    codesign --verify --strict --verbose=2 "$app"
fi

# No extended attributes or AppleDouble (._*) files in the tarball (macOS's bsdtar adds them).
# (No arrays: macOS's bash 3.2 can't expand an empty one under set -u.)
no_mac_metadata=""
if tar --version 2>/dev/null | grep -q bsdtar; then no_mac_metadata="--no-mac-metadata"; fi
# shellcheck disable=SC2086
COPYFILE_DISABLE=1 tar $no_mac_metadata -czf "$out/$name.tar.gz" -C "$work" "$name"
echo "wrote $out/$name.tar.gz"

if command -v hdiutil >/dev/null; then
    dmg="$work/dmg"
    mkdir -p "$dmg"
    cp -R "$app" "$dmg/Mothr.app"
    ln -s /Applications "$dmg/Applications"
    rm -f "$out/$name.dmg"
    hdiutil create -volname Mothr -srcfolder "$dmg" -fs HFS+ -format UDZO -ov "$out/$name.dmg" >/dev/null
    echo "wrote $out/$name.dmg"
fi
