#!/usr/bin/env bash
# Install Mothr on Linux (x86_64) or macOS (Apple silicon or Intel, macOS 11 or later).
#
#   curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash
#
# Installs for the current user (no sudo). Run it again to update or repair. Mothr then keeps
# itself up to date (Settings → Updates).
#   Linux: the app in ~/.local/lib/mothr, the `mothr` command in ~/.local/bin and a menu entry.
#   macOS: Mothr.app in ~/Applications (Launchpad, Spotlight) and the `mothr` command in
#          ~/.local/bin.
#
# Options (pass after `bash -s --` when piping, e.g. `| bash -s -- --version 0.2.0`):
#   --version <x.y.z>   install this release instead of the newest
#   --dir <path>        where the app goes (default ~/.local/lib/mothr on Linux, ~/Applications
#                       on macOS, or $MOTHR_INSTALL_DIR)
#   --bin-dir <path>    where the `mothr` command goes (default ~/.local/bin, or $MOTHR_BIN_DIR)
#   --no-menu-entry     don't add Mothr to the application menu (Linux)
#   --uninstall         remove Mothr (your chats and settings stay)
#   -h, --help          show this help
#
# Everything is wrapped in main(), so a download cut short runs nothing. It sticks to what
# macOS's bash 3.2 and BSD tools understand. Brace a variable that touches non-ASCII text
# ("${version}…"): in the C locale bash 3.2 reads the UTF-8 bytes as part of the name.

set -euo pipefail

main() {
    local releases="${MOTHR_RELEASES_URL:-https://github.com/evolvedmesh/mothr/releases}"
    local os
    os="$(uname -s)"
    local root bin_dir="${MOTHR_BIN_DIR:-$HOME/.local/bin}"
    case "$os" in
        Linux) root="${MOTHR_INSTALL_DIR:-$HOME/.local/lib/mothr}" ;;
        Darwin) root="${MOTHR_INSTALL_DIR:-$HOME/Applications}" ;;
        *) die "this installer is for Linux and macOS. On Windows, use the setup from $releases/latest." ;;
    esac
    local apps_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    local version="" menu=1 uninstall=0

    while [ $# -gt 0 ]; do
        case "$1" in
            --version) version="${2:?--version needs a value}"; version="${version#v}"; shift 2 ;;
            --dir) root="${2:?--dir needs a value}"; shift 2 ;;
            --bin-dir) bin_dir="${2:?--bin-dir needs a value}"; shift 2 ;;
            --no-menu-entry) menu=0; shift ;;
            --uninstall) uninstall=1; shift ;;
            -h|--help) usage; return 0 ;;
            *) die "unknown option: $1 (see --help)" ;;
        esac
    done

    if [ "$uninstall" = 1 ]; then
        if [ "$os" = Darwin ]; then remove_macos "$root" "$bin_dir"; else remove_linux "$root" "$bin_dir" "$apps_dir"; fi
        return 0
    fi

    # Set by the functions below. They run in this shell, not in $(…): bash drops `set -e` there.
    PLATFORM="" PREVIOUS=""
    detect_platform "$os"
    local platform="$PLATFORM"
    for tool in curl tar mktemp; do
        command -v "$tool" >/dev/null || die "needs '$tool'; install it and run this again."
    done
    command -v sha256sum >/dev/null || command -v shasum >/dev/null || die "needs 'sha256sum' or 'shasum'; install one and run this again."

    if [ -z "$version" ]; then
        say "Finding the newest release…"
        version="$(latest_version "$releases")" || die "couldn't find a Mothr release at $releases"
    fi
    local name="mothr_${version}_${platform}"
    local tmp
    tmp="$(mktemp -d)"
    # Expanded now: main's locals are gone when the trap runs.
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp'" EXIT

    say "Downloading Mothr ${version}…"
    fetch "$releases/download/v$version/$name.tar.gz" "$tmp/$name.tar.gz" \
        || die "couldn't download Mothr $version for $(platform_label "$platform"). Is it a released version? See $releases"
    fetch "$releases/download/v$version/SHA256SUMS" "$tmp/SHA256SUMS" \
        || die "couldn't download the checksums for $version"
    verify "$tmp" "$name.tar.gz" \
        || die "the download is damaged (checksum mismatch). Nothing was installed; try again."

    if [ "$os" = Darwin ]; then
        install_macos "$tmp" "$name" "$root" "$bin_dir"
    else
        install_linux "$tmp" "$name" "$version" "$root" "$bin_dir" "$apps_dir" "$menu"
    fi
    local previous="$PREVIOUS"

    say ""
    if [ -n "$previous" ] && [ "$previous" != "$version" ]; then
        say "Updated Mothr $previous → $version."
    else
        say "Installed Mothr $version."
    fi
    # A path to show, not to expand.
    # shellcheck disable=SC2088
    local launcher="your app menu" profile="~/.profile"
    if [ "$os" = Darwin ]; then
        launcher="Launchpad or Spotlight"
        # shellcheck disable=SC2088
        profile="~/.zprofile"
    fi
    case ":$PATH:" in
        *":$bin_dir:"*) say "Start it from $launcher, or run: mothr" ;;
        *)
            say "Start it from $launcher, or run: $bin_dir/mothr"
            say "To run plain \`mothr\`, add $bin_dir to your PATH (e.g. in $profile):"
            say "  export PATH=\"$bin_dir:\$PATH\""
            ;;
    esac
    say "Mothr updates itself from now on; see Settings → Updates."
}

usage() {
    cat <<'USAGE'
Install Mothr on Linux (x86_64) or macOS (Apple silicon or Intel), for the current user.

  curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash -s -- [options]

Options:
  --version <x.y.z>   install this release instead of the newest
  --dir <path>        where the app goes (default ~/.local/lib/mothr on Linux, ~/Applications
                      on macOS, or $MOTHR_INSTALL_DIR)
  --bin-dir <path>    where the `mothr` command goes (default ~/.local/bin, or $MOTHR_BIN_DIR)
  --no-menu-entry     don't add Mothr to the application menu (Linux)
  --uninstall         remove Mothr (your chats and settings stay)
  -h, --help          show this help
USAGE
}

say() { printf '%s\n' "$*"; }
die() { printf 'mothr install: %s\n' "$*" >&2; exit 1; }

# Sets PLATFORM to the release build for this machine, as asset names spell it: linux_x64,
# macos_arm64 or macos_x64.
detect_platform() {
    local arch
    arch="$(uname -m)"
    if [ "$1" = Darwin ]; then
        local major
        major="$(sw_vers -productVersion 2>/dev/null | cut -d. -f1)"
        if [ -n "$major" ] && [ "$major" -lt 11 ] 2>/dev/null; then
            die "Mothr needs macOS 11 or later (this Mac has $(sw_vers -productVersion))."
        fi
        # A shell under Rosetta reports x86_64 on Apple silicon; install the native build.
        if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = 1 ]; then
            arch=arm64
        fi
        case "$arch" in
            arm64|aarch64) PLATFORM=macos_arm64 ;;
            x86_64) PLATFORM=macos_x64 ;;
            *) die "Mothr isn't released for this Mac ($arch)." ;;
        esac
    else
        case "$arch" in
            x86_64|amd64) PLATFORM=linux_x64 ;;
            *) die "Mothr is released for x86_64 Linux only so far (this machine is $arch)." ;;
        esac
    fi
}

platform_label() {
    case "$1" in
        linux_x64) echo "Linux x86_64" ;;
        macos_arm64) echo "macOS on Apple silicon" ;;
        macos_x64) echo "macOS on Intel" ;;
        *) echo "$1" ;;
    esac
}

fetch() {
    curl -fL --proto '=https,http' --retry 3 --connect-timeout 15 -sS -o "$2" "$1"
}

# Whether <dir>/<file> matches its line in <dir>/SHA256SUMS. macOS has shasum, not sha256sum.
verify() {
    local want got
    want="$(awk -v n="$2" '{ f = $2; sub(/^\*/, "", f); if (f == n) { print $1; exit } }' "$1/SHA256SUMS")"
    [ -n "$want" ] || return 1
    if command -v sha256sum >/dev/null; then
        got="$(sha256sum "$1/$2" | awk '{print $1}')"
    else
        got="$(shasum -a 256 "$1/$2" | awk '{print $1}')"
    fi
    [ "$got" = "$(printf '%s' "$want" | tr 'A-F' 'a-f')" ]
}

# The newest release, from where GitHub's /releases/latest redirects (no API, no JSON).
latest_version() {
    local url
    url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "$1/latest")" || return 1
    case "$url" in
        */tag/v*) printf '%s\n' "${url##*/tag/v}" ;;
        *) return 1 ;;
    esac
}

# Linux: versions/<version>/ next to each other and a `current` symlink (mothr-update's
# Layout::Versions). Sets PREVIOUS to the version it replaced.
install_linux() {
    local tmp="$1" name="$2" version="$3" root="$4" bin_dir="$5" apps_dir="$6" menu="$7"
    say "Installing to ${root}…"
    mkdir -p "$root/versions" "$bin_dir"
    local staging="$root/versions/.staging-$version"
    rm -rf "$staging"
    mkdir -p "$staging"
    tar -xzf "$tmp/$name.tar.gz" -C "$staging"
    [ -x "$staging/$name/mothr" ] || die "the release archive doesn't contain the mothr binary"
    local previous=""
    if [ -L "$root/current" ]; then previous="$(basename "$(readlink "$root/current")")"; fi
    rm -rf "$root/versions/$version"
    mv "$staging/$name" "$root/versions/$version"
    rm -rf "$staging"
    # Marks a copy that updates itself (Mothr checks for this file).
    printf '{"installer":"install.sh","version":"%s"}\n' "$version" > "$root/install.json"
    # Switch in one rename, so a running Mothr keeps working and the next start is the new one.
    ln -sfn "versions/$version" "$root/.current.new"
    mv -Tf "$root/.current.new" "$root/current"
    # Keep the new version and the one it replaces; drop older ones.
    local d
    for d in "$root"/versions/*/; do
        d="$(basename "$d")"
        [ "$d" = "$version" ] || [ "$d" = "$previous" ] || rm -rf "${root:?}/versions/$d"
    done

    ln -sfn "$root/current/mothr" "$bin_dir/mothr"

    if [ "$menu" = 1 ]; then
        mkdir -p "$apps_dir"
        sed "s|^Exec=.*|Exec=$root/current/mothr|" "$root/current/dev.mothr.Mothr.desktop" > "$apps_dir/dev.mothr.Mothr.desktop"
        command -v update-desktop-database >/dev/null && update-desktop-database -q "$apps_dir" 2>/dev/null || true
    fi

    check_libraries "$root/current/mothr"
    PREVIOUS="$previous"
}

# macOS: <root>/Mothr.app, replaced as a whole (mothr-update's Layout::Bundle: any Mothr.app the
# user can write updates itself, so there's no marker file). Sets PREVIOUS to the version it
# replaced.
install_macos() {
    local tmp="$1" name="$2" root="$3" bin_dir="$4"
    local dest="$root/Mothr.app"
    say "Installing to ${dest}…"
    mkdir -p "$tmp/unpacked"
    tar -xzf "$tmp/$name.tar.gz" -C "$tmp/unpacked"
    local new="$tmp/unpacked/$name/Mothr.app"
    [ -x "$new/Contents/MacOS/mothr" ] || die "the release archive doesn't contain Mothr.app"

    mkdir -p "$root" "$bin_dir"
    local previous=""
    if [ -e "$dest" ]; then
        is_mothr_app "$dest" || die "$dest exists and isn't Mothr; move it away and run this again."
        previous="$(plist_version "$dest/Contents/Info.plist")"
    fi
    # Next to the destination first (the download may be on another volume), then swap by
    # renaming. A running Mothr keeps running from the bundle that moved away.
    local work="$root/.mothr-update/install-$$"
    rm -rf "$work"
    mkdir -p "$work"
    mv "$new" "$work/Mothr.app"
    if [ -e "$dest" ]; then mv "$dest" "$work/replaced"; fi
    mv "$work/Mothr.app" "$dest"
    rm -rf "$work"
    rmdir "$root/.mothr-update" 2>/dev/null || true

    # curl doesn't quarantine what it downloads, but a copy that came through a browser may be.
    command -v xattr >/dev/null && xattr -dr com.apple.quarantine "$dest" 2>/dev/null || true
    local lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
    [ -x "$lsregister" ] && "$lsregister" -f "$dest" 2>/dev/null || true

    ln -sfn "$dest/Contents/MacOS/mothr" "$bin_dir/mothr"

    if [ "$dest" != /Applications/Mothr.app ] && [ -d /Applications/Mothr.app ] && is_mothr_app /Applications/Mothr.app; then
        say "There's another Mothr in /Applications. Delete it so Spotlight and Launchpad open this one."
    fi
    PREVIOUS="$previous"
}

is_mothr_app() {
    grep -q '<string>dev.mothr.Mothr</string>' "$1/Contents/Info.plist" 2>/dev/null
}

# CFBundleShortVersionString from an XML Info.plist.
plist_version() {
    awk '/<key>CFBundleShortVersionString<\/key>/ { getline; gsub(/.*<string>|<\/string>.*/, ""); print; exit }' "$1" 2>/dev/null || true
}

# Mothr draws with Vulkan and talks to Wayland or X11; say what to install if a library is missing.
check_libraries() {
    command -v ldd >/dev/null || return 0
    local missing
    missing="$(ldd "$1" 2>/dev/null | awk '/not found/ {print $1}' | tr '\n' ' ')" || true
    if [ -n "$missing" ]; then
        say ""
        say "Mothr needs libraries this system doesn't have: $missing"
        hint_packages
    fi
    if ! { ldconfig -p 2>/dev/null | grep 'libvulkan\.so\.1' >/dev/null; } && ! ls /usr/lib*/libvulkan.so.1 /usr/lib/*/libvulkan.so.1 >/dev/null 2>&1; then
        say ""
        say "Mothr draws with Vulkan, but no Vulkan loader (libvulkan.so.1) was found."
        hint_packages
    fi
}

hint_packages() {
    if command -v apt-get >/dev/null; then
        say "  sudo apt install libvulkan1 mesa-vulkan-drivers libxkbcommon-x11-0 libwayland-client0 libfontconfig1"
    elif command -v dnf >/dev/null; then
        say "  sudo dnf install vulkan-loader mesa-vulkan-drivers libxkbcommon-x11 libwayland-client fontconfig"
    elif command -v pacman >/dev/null; then
        say "  sudo pacman -S vulkan-icd-loader libxkbcommon-x11 wayland fontconfig  (plus vulkan-radeon / vulkan-intel / nvidia-utils)"
    else
        say "  Install your distribution's Vulkan loader and driver, xkbcommon-x11, wayland-client and fontconfig."
    fi
}

remove_linux() {
    local root="$1" bin_dir="$2" apps_dir="$3"
    if [ -L "$bin_dir/mothr" ] && [ "$(readlink "$bin_dir/mothr")" = "$root/current/mothr" ]; then
        rm -f "$bin_dir/mothr"
    fi
    rm -f "$apps_dir/dev.mothr.Mothr.desktop"
    if [ -f "$root/install.json" ]; then
        rm -rf "${root:?}"
    elif [ -e "$root" ]; then
        die "$root doesn't look like a Mothr install (no install.json); not removing it."
    fi
    say "Removed Mothr."
    say "Your chats and settings are still in ${XDG_CONFIG_HOME:-$HOME/.config}/mothr and ${XDG_DATA_HOME:-$HOME/.local/share}/mothr; delete those folders to remove them too."
}

remove_macos() {
    local root="$1" bin_dir="$2"
    local app="$root/Mothr.app"
    if [ -L "$bin_dir/mothr" ] && [ "$(readlink "$bin_dir/mothr")" = "$app/Contents/MacOS/mothr" ]; then
        rm -f "$bin_dir/mothr"
    fi
    if [ -e "$app" ]; then
        is_mothr_app "$app" || die "$app isn't Mothr; not removing it."
        rm -rf "${app:?}"
    fi
    rm -rf "${root:?}/.mothr-update"
    say "Removed Mothr."
    say "Your chats and settings are still in ~/Library/Application Support/mothr; delete that folder to remove them too."
}

main "$@"
