#!/bin/sh
# Sets Mothr's version everywhere it lives: the Cargo workspace (every crate and the desktop
# app inherit it), its entries in Cargo.lock, and through Cargo.toml the Android app's
# versionName (apps/mobile/composeApp/build.gradle.kts reads it from there).
#
# semantic-release runs this before it commits and tags a release:
#   scripts/set-version.sh 1.2.0-dev.3
set -eu

version="${1:?usage: scripts/set-version.sh <version>}"
root="$(cd "$(dirname "$0")/.." && pwd)"

# The first top-level `version = ` in Cargo.toml is [workspace.package]'s.
sed -i "0,/^version = \".*\"/s//version = \"$version\"/" "$root/Cargo.toml"

# Workspace members are the lock's `mothr` / `mothr-*` packages.
awk -v v="$version" '
  /^\[\[package\]\]/ { ours = 0 }
  /^name = "mothr(-[a-z0-9-]+)?"$/ { ours = 1 }
  ours && /^version = / { $0 = "version = \"" v "\""; ours = 0 }
  { print }
' "$root/Cargo.lock" > "$root/Cargo.lock.tmp"
mv "$root/Cargo.lock.tmp" "$root/Cargo.lock"

grep -q "^version = \"$version\"" "$root/Cargo.toml"
