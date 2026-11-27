#!/bin/sh
# Sets Mothr's version everywhere it lives: the Cargo workspace (every crate and the desktop
# app inherit it), its entries in Cargo.lock, and through Cargo.toml the Android app's
# versionName (apps/mobile/composeApp/build.gradle.kts reads it from there).
#
#   scripts/set-version.sh 1.2.0          the prepare-release workflow, committed on release/1.2.0
#   scripts/set-version.sh 1.2.0-dev.3    the build jobs, for a nightly build (not committed)
#
# Plain POSIX sh and awk: the build jobs run it on Linux, macOS (BSD tools) and Windows (Git Bash).
set -eu

version="${1:?usage: scripts/set-version.sh <version>}"
root="$(cd "$(dirname "$0")/.." && pwd)"

# Rewrites file $1 through the awk program $2.
rewrite() {
  awk -v v="$version" "$2" "$1" > "$1.tmp"
  mv "$1.tmp" "$1"
}

# The first top-level `version = ` in Cargo.toml is [workspace.package]'s.
rewrite "$root/Cargo.toml" '
  !done && /^version = "/ { $0 = "version = \"" v "\""; done = 1 }
  { print }
'

# Workspace members are the lock's `mothr` / `mothr-*` packages.
rewrite "$root/Cargo.lock" '
  /^\[\[package\]\]/ { ours = 0 }
  /^name = "mothr(-[a-z0-9-]+)?"$/ { ours = 1 }
  ours && /^version = / { $0 = "version = \"" v "\""; ours = 0 }
  { print }
'

grep -q "^version = \"$version\"" "$root/Cargo.toml"
