#!/bin/bash
# run-scratch.sh: run a built a11y-service as a scratch instance beside the installed one, time
# it, and leave nothing running. The scratch instance has its own runtime directory, so it takes
# no lock and no socket the desktop's own service uses.
#
# Put a11y-service, sight-bench.py and sight-demo-app.py in $SIGHT_BENCH_DIR (default
# $XDG_RUNTIME_DIR/sight-demo, which only the caller can write to) first. The report is
# $SIGHT_BENCH_DIR/report.json. Needs python3 with GTK 3 (python3-gi) and wlrctl, and a session
# with the accessibility bus on.
#
# The directory is run from and parts of it are removed, so it must be the caller's own and not a
# symlink: a fixed path in a shared directory such as /tmp is one anyone could have made first.
set -u
D=${SIGHT_BENCH_DIR:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/sight-demo}
if [ -L "$D" ] || { [ -e "$D" ] && { [ ! -d "$D" ] || [ ! -O "$D" ]; }; }; then
  echo "refusing $D: it is not a directory of your own, or it is a symlink" >&2
  exit 1
fi
mkdir -p "$D" && chmod 700 "$D" || exit 1
RT=$D/rt
rm -rf "$RT"; mkdir -p "$RT"; chmod 700 "$RT"
SESSION=/run/user/$(id -u)
export DBUS_SESSION_BUS_ADDRESS=unix:path=$SESSION/bus

BIN=${1:-a11y-service}
OUT=${2:-report.json}
# The scratch service reads Private mode from a home of its own, so the bench can turn it on and
# off without touching the person's.
SCRATCH_HOME=$D/home
rm -rf "$SCRATCH_HOME"; mkdir -p "$SCRATCH_HOME/.config/yantrik"
export SIGHT_PRIVACY_FILE=$SCRATCH_HOME/.config/yantrik/privacy.json
echo '{"private": false, "since": 0}' > "$SIGHT_PRIVACY_FILE"
HOME=$SCRATCH_HOME XDG_RUNTIME_DIR=$RT "$D/$BIN" > "$D/service.log" 2>&1 &
SVC=$!
APPS=""
cleanup() {
  kill "$SVC" 2>/dev/null
  for p in $APPS; do kill "$p" 2>/dev/null; done
}
trap cleanup EXIT

for _ in $(seq 1 50); do [ -S "$RT/yantrik/a11y.sock" ] && break; sleep 0.1; done
[ -S "$RT/yantrik/a11y.sock" ] || { echo "no socket"; cat "$D/service.log"; exit 1; }
sleep 2

XDG_RUNTIME_DIR=$SESSION WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0} \
  python3 "$D/sight-bench.py" "$RT/yantrik/a11y.sock" "$D/sight-demo-app.py" "$SVC" > "$D/$OUT" 2> "$D/bench.err"
echo "bench rc=$?"

kill "$SVC" 2>/dev/null; wait "$SVC" 2>/dev/null
sleep 0.5
echo "--- left running (expect none):"
ps -u "$(id -u)" -o pid,cmd | grep -E "sight-demo|$D/a11y-service" | grep -v -E "grep|run-scratch" || true
echo "--- service log:"
tail -15 "$D/service.log"
echo "--- bench stderr:"
tail -15 "$D/bench.err"
