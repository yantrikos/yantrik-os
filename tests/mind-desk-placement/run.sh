#!/usr/bin/env bash
# Where the Mind desk's bar lands, measured on pixels, in the labwc the OS runs.
#
# The bar is placed by a window rule in config/labwc-mind/rc.xml, and what a rule does is labwc's
# to say: a unit test can hold the rule to the file, and cannot say where a window ends up. This
# starts labwc 0.8.x (Debian 13's, unpacked by fetch-trixie.py) on wlroots' headless backend with
# that rc.xml, starts the desk against a stand-in shell (fake_shell.py), resizes the display as a
# Mind View window is resized (wlr-randr), takes a picture of the display with grim, and has
# measure.py say where the bar is:
#
#   rows 0..160 (the margin and the pixel over the apps), as wide as the display
#
# at 896x560 (Mind View's first size), at 1280x690 (the same window maximised on the person's
# screen), back at 896x560, and again after the desk has been killed and started anew (the shell
# restarts it, and a rule runs at every map).
#
#   tests/mind-desk-placement/run.sh
#
# Environment:
#   PREFIX  where fetch-trixie.py unpacked Debian 13       (default $HOME/mvtest; fetched if absent)
#   DESK    the desk binary                                (default target/debug/yantrik-mind-desk)
#   RC      the rc.xml under test                          (default config/labwc-mind/rc.xml)
#   OUT     where the pictures go                          (default a new directory, printed)
#   QUICK   set: only the first size (for trying a rule)
#
# Exit status 0 when every picture has the bar where it belongs.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
PREFIX=${PREFIX:-$HOME/mvtest}
DESK=${DESK:-${CARGO_TARGET_DIR:-$repo/target}/debug/yantrik-mind-desk}
RC=${RC:-$repo/config/labwc-mind/rc.xml}
OUT=${OUT:-$(mktemp -d)}
BAR=${BAR:-161}
mkdir -p "$OUT"

[ -d "$PREFIX/root" ] || python3 "$here/fetch-trixie.py" "$PREFIX"
root=$PREFIX/root
loader=
for candidate in "$root"/lib64/ld-linux-x86-64.so.2 "$root"/usr/lib64/ld-linux-x86-64.so.2 "$root"/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2; do
  [ -e "$candidate" ] && { loader=$candidate; break; }
done
[ -n "$loader" ] || { echo "no loader under $root" >&2; exit 2; }
libs=$root/usr/lib/x86_64-linux-gnu:$root/lib/x86_64-linux-gnu
# Debian's programs, through Debian's loader and libraries; the library path is an argument of the
# loader and not in the environment, so what labwc starts (the desk) keeps the machine's own.
deb() { local program=$1; shift; "$loader" --library-path "$libs" "$root/usr/bin/$program" "$@"; }

runtime=$(mktemp -d)
chmod 700 "$runtime"
export XDG_RUNTIME_DIR=$runtime
pids=()
# This run's desks: the one labwc started and the one started again by hand, found by the runtime
# directory they were given, so no other copy of the binary on the machine is ever touched.
our_desks() {
  for p in $(pgrep -f "^$DESK" 2>/dev/null || true); do
    if tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | grep -qx "XDG_RUNTIME_DIR=$runtime"; then echo "$p"; fi
  done
}
cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
  for p in $(our_desks); do kill "$p" 2>/dev/null || true; done
  rm -rf "$runtime"
}
trap cleanup EXIT

mkdir -p "$runtime/conf"
cp "$RC" "$runtime/conf/rc.xml"
python3 "$here/fake_shell.py" > "$OUT/fake-shell.log" 2>&1 &
pids+=($!)

# What the nested labwc's startup script does for the desk, minus the part the shell does now: the
# desk is a process of the shell's, marked as drawn for a Mind, on the nested display. No GPU here,
# so its software renderer.
cat > "$runtime/seat.sh" <<EOF
#!/bin/sh
SLINT_BACKEND=winit-software YANTRIK_FOR_MIND=1 exec "$DESK"
EOF
chmod +x "$runtime/seat.sh"

export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_HEADLESS_OUTPUTS=1
export XKB_CONFIG_ROOT=$root/usr/share/X11/xkb
# labwc refuses to start without an Xwayland binary at hand; it is only started when an X client
# connects, which nothing here does.
export WLR_XWAYLAND=$root/usr/bin/Xwayland
deb labwc -C "$runtime/conf" -s "$runtime/seat.sh" > "$OUT/labwc.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 50); do
  socket=$(ls "$runtime" | grep -E '^wayland-[0-9]+$' | head -1 || true)
  [ -n "$socket" ] && break
  sleep 0.2
done
[ -n "${socket:-}" ] || { echo "labwc did not start; see $OUT/labwc.log" >&2; exit 2; }
export WAYLAND_DISPLAY=$socket
output=$(deb wlr-randr | awk 'NR == 1 { print $1 }')

status=0
shoot() { # name WxH
  deb wlr-randr --output "$output" --custom-mode "$2" >/dev/null
  sleep 4
  deb grim -o "$output" "$OUT/$1.png"
  echo "── $1 ($2)"
  python3 "$here/measure.py" "$OUT/$1.png" "$BAR" || status=1
}

shoot first-896x560 896x560
[ -z "${QUICK:-}" ] || { echo "pictures: $OUT"; exit $status; }
shoot maximised-1280x690 1280x690
shoot restored-896x560 896x560

# The desk ends and the shell starts it again: a new window, a rule run at its map.
desk=$(our_desks | head -1)
if [ -n "$desk" ]; then
  kill "$desk"
  sleep 1
  SLINT_BACKEND=winit-software YANTRIK_FOR_MIND=1 "$DESK" > "$OUT/desk-again.log" 2>&1 &
  pids+=($!)
fi
shoot after-restart-1280x690 1280x690

echo "pictures: $OUT"
exit $status
