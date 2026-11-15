#!/bin/bash
# run-scratch.sh: run a built a11y-service as a scratch instance beside the installed one, time
# it, and leave nothing running. The scratch instance has its own runtime directory, so it takes
# no lock and no socket the desktop's own service uses.
#
# Put a11y-service, sight-bench.py and sight-demo-app.py in $SIGHT_BENCH_DIR (default
# /tmp/sight-demo) first. The report is $SIGHT_BENCH_DIR/report.json. Needs python3 with GTK 3
# (python3-gi) and wlrctl, and a session with the accessibility bus on.
set -u
D=${SIGHT_BENCH_DIR:-/tmp/sight-demo}
RT=$D/rt
rm -rf "$RT"; mkdir -p "$RT"; chmod 700 "$RT"
export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus

XDG_RUNTIME_DIR=$RT "$D/a11y-service" > "$D/service.log" 2>&1 &
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

XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 \
  python3 "$D/sight-bench.py" "$RT/yantrik/a11y.sock" "$D/sight-demo-app.py" "$SVC" > "$D/report.json" 2> "$D/bench.err"
echo "bench rc=$?"

kill "$SVC" 2>/dev/null; wait "$SVC" 2>/dev/null
sleep 0.5
echo "--- left running (expect none):"
ps -u "$(id -u)" -o pid,cmd | grep -E "sight-demo|$D/a11y-service" | grep -v -E "grep|run-scratch" || true
echo "--- service log:"
tail -15 "$D/service.log"
echo "--- bench stderr:"
tail -15 "$D/bench.err"
