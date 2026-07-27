#!/usr/bin/env bash
# first-boot-check.sh — assert a clean first boot is actually usable.
#
# Every defect this guards against was invisible to static review and fatal
# to a user: the shell booted, looked correct, and could not answer a single
# question. A developer never hits them because a developer never starts from
# an empty profile.
#
# Run headless (Xvfb) in CI, or against a real display locally:
#   scripts/first-boot-check.sh /path/to/config.yaml
#
# Exits non-zero on the first failed assertion.

set -uo pipefail

CONFIG="${1:-}"
BIN="${YANTRIK_UI_BIN:-target/release/yantrik-ui}"
BOOT_TIMEOUT="${BOOT_TIMEOUT:-90}"
PROFILE_HOME="${PROFILE_HOME:-$(mktemp -d)}"
LOG="$PROFILE_HOME/ui.log"

if [ -z "$CONFIG" ] || [ ! -f "$CONFIG" ]; then
  echo "usage: $0 <config.yaml>   (config must exist)" >&2
  exit 2
fi
if [ ! -x "$BIN" ]; then
  echo "FAIL: $BIN not found or not executable (build with -p yantrik-ui)" >&2
  exit 2
fi

fail() { echo "FAIL: $*" >&2; FAILED=1; }
ok()   { echo "ok: $*"; }
FAILED=0

echo "first-boot-check: clean profile at $PROFILE_HOME"

# A true first boot: no config dir, no database, no onboarding marker.
# db_path is absolute in config.yaml and therefore lives OUTSIDE $HOME, so
# overriding HOME alone leaves the recipes table behind and the "clean boot"
# is not clean. Clear it explicitly or this check silently tests a warm start.
export HOME="$PROFILE_HOME"
mkdir -p "$PROFILE_HOME"
rm -rf "$PROFILE_HOME/.config/yantrik" "$PROFILE_HOME/.yantrik"

DB_PATH=$(grep -oE 'db_path: *"?[^"]+' "$CONFIG" | head -1 | sed 's/.*: *"*//' | tr -d '"')
if [ -n "$DB_PATH" ]; then
  rm -f "$DB_PATH" "$DB_PATH-shm" "$DB_PATH-wal"
  echo "cleared database at $DB_PATH"
fi

"$BIN" "$CONFIG" > "$LOG" 2>&1 &
UI_PID=$!
trap 'kill "$UI_PID" 2>/dev/null' EXIT

# Wait for the shell to come up.
for _ in $(seq "$BOOT_TIMEOUT"); do
  grep -q "Starting Yantrik OS desktop shell" "$LOG" 2>/dev/null && break
  kill -0 "$UI_PID" 2>/dev/null || { echo "FAIL: process died during boot"; tail -30 "$LOG"; exit 1; }
  sleep 1
done
grep -q "Starting Yantrik OS desktop shell" "$LOG" || { fail "shell never started"; tail -30 "$LOG"; exit 1; }
ok "shell started"

# Give instincts/recipes a window to misbehave.
sleep 20

# ── Assertion 1 ────────────────────────────────────────────────────────────
# Built-in recipe *definitions* are stored with status 'pending'. They must
# never be treated as interrupted work and replayed. When they were, a clean
# boot executed ~50 templates with their {{variables}} unsubstituted and
# saturated the single companion worker, so the user's first question was
# never serviced.
# grep -c prints 0 and exits 1 on no match, so `|| echo 0` would append a
# second line and make the numeric test below error out instead of evaluate.
RESUMED=$(grep -c "Resuming recipe from previous session" "$LOG" 2>/dev/null)
RESUMED=${RESUMED:-0}
if [ "$RESUMED" -gt 0 ]; then
  fail "$RESUMED recipes resumed on a first boot (expected 0) — recipe storm regressed"
else
  ok "no recipes resumed on first boot"
fi

# Unsubstituted template variables must never reach the model.
if grep -q '{{' "$LOG" 2>/dev/null; then
  fail "unsubstituted {{template}} variables were sent to the LLM"
else
  ok "no template placeholders sent to the LLM"
fi

# ── Assertion 2 ────────────────────────────────────────────────────────────
# The runtime probe must test the endpoint the companion is configured to
# use. Hardcoding localhost reported "Ollama not found" on the documented
# remote-Ollama deployment while the companion was using it successfully.
if grep -q "Onboarding: scan complete" "$LOG" 2>/dev/null; then
  ok "hardware scan completed"
  CONFIGURED=$(grep -oE 'api_base_url: *"?[^" ]+' "$CONFIG" | head -1 | sed 's/.*: *"*//')
  if [ -n "$CONFIGURED" ] && grep -q "LLM runtime reachable" "$LOG"; then
    if grep "LLM runtime reachable" "$LOG" | grep -q "localhost" \
       && ! echo "$CONFIGURED" | grep -q "localhost"; then
      fail "runtime probe used localhost while config points at $CONFIGURED"
    else
      ok "runtime probe used the configured endpoint"
    fi
  fi
else
  fail "hardware scan never completed"
fi

# The scan must not report hardware it did not measure.
if grep -q "no system snapshot within timeout" "$LOG" 2>/dev/null; then
  fail "hardware scan timed out instead of reading hardware directly"
fi

# ── Assertion 3 ────────────────────────────────────────────────────────────
# Nothing should be panicking on the happy path.
if grep -qE "panicked at|RUST_BACKTRACE" "$LOG" 2>/dev/null; then
  fail "panic during first boot"
  grep -A5 "panicked at" "$LOG" | head -20
else
  ok "no panics"
fi

echo
if [ "$FAILED" -ne 0 ]; then
  echo "first-boot-check: FAILED (log: $LOG)"
  exit 1
fi
echo "first-boot-check: PASSED"
