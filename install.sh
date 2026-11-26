#!/usr/bin/env bash
# Install Mothr on Linux (x86_64).
#
#   curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash
#
# Installs for the current user (no sudo): the app in ~/.local/lib/mothr, the `mothr` command
# in ~/.local/bin and a menu entry. Run it again to update or repair. Mothr then keeps itself
# up to date (Settings → Updates).
#
# Options (pass after `bash -s --` when piping, e.g. `| bash -s -- --version 0.2.0`):
#   --version <x.y.z>   install this release instead of the newest
#   --dir <path>        where the app goes          (default ~/.local/lib/mothr, or $MOTHR_INSTALL_DIR)
#   --bin-dir <path>    where the `mothr` command goes (default ~/.local/bin, or $MOTHR_BIN_DIR)
#   --no-menu-entry     don't add Mothr to the application menu
#   --uninstall         remove Mothr (your chats and settings stay)
#   -h, --help          show this help
#
# Everything is wrapped in main(), so a download cut short runs nothing.

set -euo pipefail

main() {
    local releases="${MOTHR_RELEASES_URL:-https://github.com/evolvedmesh/mothr/releases}"
    local root="${MOTHR_INSTALL_DIR:-$HOME/.local/lib/mothr}"
    local bin_dir="${MOTHR_BIN_DIR:-$HOME/.local/bin}"
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
        remove "$root" "$bin_dir" "$apps_dir"
        return 0
    fi

    [ "$(uname -s)" = Linux ] || die "this installer is for Linux. Other platforms aren't released yet."
    case "$(uname -m)" in
        x86_64|amd64) ;;
        *) die "Mothr is released for x86_64 only so far (this machine is $(uname -m))." ;;
    esac
    for tool in curl tar sha256sum mktemp; do
        command -v "$tool" >/dev/null || die "needs '$tool'; install it and run this again."
    done

    if [ -z "$version" ]; then
        say "Finding the newest release…"
        version="$(latest_version "$releases")" || die "couldn't find a Mothr release at $releases"
    fi
    local name="mothr_${version}_linux_x64"
    local tmp
    tmp="$(mktemp -d)"
    # Expanded now: main's locals are gone when the trap runs.
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp'" EXIT

    say "Downloading Mothr $version…"
    fetch "$releases/download/v$version/$name.tar.gz" "$tmp/$name.tar.gz" \
        || die "couldn't download Mothr $version. Is it a released version? See $releases"
    fetch "$releases/download/v$version/SHA256SUMS" "$tmp/SHA256SUMS" \
        || die "couldn't download the checksums for $version"
    (cd "$tmp" && grep " \*\?$name.tar.gz\$" SHA256SUMS | sha256sum -c --status) \
        || die "the download is damaged (checksum mismatch). Nothing was installed; try again."

    say "Installing to $root…"
    mkdir -p "$root/versions" "$bin_dir"
    local staging="$root/versions/.staging-$version"
    rm -rf "$staging"
    mkdir -p "$staging"
    tar -xzf "$tmp/$name.tar.gz" -C "$staging"
    [ -x "$staging/$name/mothr" ] || die "the release archive doesn't contain the mothr binary"
    local previous=""
    [ -L "$root/current" ] && previous="$(basename "$(readlink "$root/current")")"
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

    say ""
    if [ -n "$previous" ] && [ "$previous" != "$version" ]; then
        say "Updated Mothr $previous → $version."
    else
        say "Installed Mothr $version."
    fi
    case ":$PATH:" in
        *":$bin_dir:"*) say "Start it from your app menu, or run: mothr" ;;
        *)
            say "Start it from your app menu, or run: $bin_dir/mothr"
            say "To run plain \`mothr\`, add $bin_dir to your PATH (e.g. in ~/.profile):"
            say "  export PATH=\"$bin_dir:\$PATH\""
            ;;
    esac
    say "Mothr updates itself from now on; see Settings → Updates."
}

usage() {
    cat <<'USAGE'
Install Mothr on Linux (x86_64), for the current user.

  curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash -s -- [options]

Options:
  --version <x.y.z>   install this release instead of the newest
  --dir <path>        where the app goes (default ~/.local/lib/mothr, or $MOTHR_INSTALL_DIR)
  --bin-dir <path>    where the `mothr` command goes (default ~/.local/bin, or $MOTHR_BIN_DIR)
  --no-menu-entry     don't add Mothr to the application menu
  --uninstall         remove Mothr (your chats and settings stay)
  -h, --help          show this help
USAGE
}

say() { printf '%s\n' "$*"; }
die() { printf 'mothr install: %s\n' "$*" >&2; exit 1; }

fetch() {
    curl -fL --proto '=https,http' --retry 3 --connect-timeout 15 -sS -o "$2" "$1"
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

# Mothr draws with Vulkan and talks to Wayland or X11; say what to install if a library is missing.
check_libraries() {
    command -v ldd >/dev/null || return 0
    local missing
    missing="$(ldd "$1" 2>/dev/null | awk '/not found/ {print $1}' | tr '\n' ' ')"
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

remove() {
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

main "$@"
