#!/usr/bin/env bash
# Headless screenshots of the Mothr desktop app, for a look-fix-look loop without a monitor.
#
# Mothr runs inside a headless gamescope (its own Xwayland, nothing appears on the user's
# screen) on a throwaway MOTHR_HOME; gamescopectl captures what gamescope composites.
#
#   ui-shot.sh shot <out.png> [VAR=value ...]   build, start, wait, capture, stop (one-shot)
#   ui-shot.sh start [VAR=value ...]            start and keep running (for several shots)
#   ui-shot.sh snap <out.png>                   capture the running instance
#   ui-shot.sh stop                             stop it and delete its MOTHR_HOME
#   ui-shot.sh display                          print its X display (for `DISPLAY=… xdotool`)
#
# VAR=value pairs are passed to Mothr, e.g. MOTHR_OPEN=settings or MOTHR_PREFILL=hello.
# Env knobs: UI_SHOT_SIZE=1440x900, UI_SHOT_WAIT=6 (seconds before a one-shot capture),
# UI_SHOT_BIN=<binary> (default target/debug/mothr, built first), UI_SHOT_NO_BUILD=1,
# UI_SHOT_SETTINGS=<settings.toml> seeds the profile (e.g. `theme = "light"`, `project_roots = []`).
set -euo pipefail

root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
state="${XDG_RUNTIME_DIR:-/tmp}/mothr-ui-shot.state"
size="${UI_SHOT_SIZE:-1440x900}"
w="${size%x*}" h="${size#*x}"
bin="${UI_SHOT_BIN:-$root/target/debug/mothr}"

die() { echo "ui-shot: $*" >&2; exit 1; }
load() { [ -f "$state" ] || die "not running (ui-shot.sh start)"; . "$state"; }

start() {
  command -v gamescope >/dev/null && command -v gamescopectl >/dev/null || die "needs gamescope (3.14+, with gamescopectl)"
  [ -f "$state" ] && die "already running; ui-shot.sh stop first"
  if [ -z "${UI_SHOT_BIN:-}" ] && [ -z "${UI_SHOT_NO_BUILD:-}" ]; then
    (cd "$root" && cargo build -q -p mothr-app) || die "build failed"
  fi
  [ -x "$bin" ] || die "no binary at $bin"
  local home log sock
  home="$(mktemp -d "${TMPDIR:-/tmp}/mothr-ui-shot.XXXXXX")"
  log="$home/gamescope.log"
  if [ -n "${UI_SHOT_SETTINGS:-}" ]; then
    mkdir -p "$home/config" && cp "$UI_SHOT_SETTINGS" "$home/config/settings.toml"
  fi
  # gamescope can reuse a display name a killed instance left behind, so look for the socket
  # created after this point rather than for a new name.
  touch "$home/.started"
  # WAYLAND_DISPLAY is unset so GPUI picks gamescope's Xwayland instead of the user's session.
  setsid gamescope --backend headless -w "$w" -h "$h" -W "$w" -H "$h" --force-windows-fullscreen -- \
    env -u WAYLAND_DISPLAY MOTHR_HOME="$home" "$@" "$bin" >"$log" 2>&1 < /dev/null &
  local pid=$!
  # Up to 30 s: gamescope starts slowly while the machine is busy (e.g. building).
  for _ in $(seq 1 300); do
    sock="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'gamescope-[0-9]*' ! -name '*-ei' -newer "$home/.started" -printf '%f\n' 2>/dev/null | head -n1)"
    [ -n "$sock" ] && grep -q 'Starting Xwayland on' "$log" && break
    kill -0 "$pid" 2>/dev/null || { cat "$log" >&2; rm -rf "$home"; die "gamescope exited"; }
    sleep 0.1
  done
  [ -n "$sock" ] || { kill -- -"$pid" 2>/dev/null; tail -n 20 "$log" >&2; rm -rf "$home"; die "gamescope didn't come up"; }
  local display; display="$(sed -n 's/.*Starting Xwayland on \(:[0-9]*\).*/\1/p' "$log" | head -n1)"
  printf 'PGID=%s\nSOCK=%s\nHOME_DIR=%s\nXDISPLAY=%s\n' "$pid" "$sock" "$home" "$display" > "$state"
  echo "started: display $display, MOTHR_HOME $home, log $log"
}

snap() {
  load
  local out; out="$(realpath -m "$1")"
  rm -f "$out"
  GAMESCOPE_WAYLAND_DISPLAY="$SOCK" gamescopectl screenshot "$out" >/dev/null 2>&1 || die "gamescopectl failed"
  # The capture is written asynchronously on the next composited frame.
  for _ in $(seq 1 50); do [ -s "$out" ] && { echo "$out"; return; }; sleep 0.1; done
  die "no screenshot written (is the window drawing? see $HOME_DIR/gamescope.log)"
}

stop() {
  [ -f "$state" ] || return 0
  . "$state"
  kill -- -"$PGID" 2>/dev/null || true
  for _ in $(seq 1 30); do kill -0 "$PGID" 2>/dev/null || break; sleep 0.1; done
  kill -9 -- -"$PGID" 2>/dev/null || true
  rm -rf "$HOME_DIR" "$state"
}

cmd="${1:-}"; shift || true
case "$cmd" in
  start) start "$@" ;;
  snap) [ $# -ge 1 ] || die "usage: ui-shot.sh snap <out.png>"; snap "$1" ;;
  stop) stop ;;
  display) load; echo "$XDISPLAY" ;;
  shot)
    [ $# -ge 1 ] || die "usage: ui-shot.sh shot <out.png> [VAR=value ...]"
    out="$1"; shift
    trap stop EXIT
    start "$@" >/dev/null
    sleep "${UI_SHOT_WAIT:-6}"
    snap "$out"
    ;;
  *) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
