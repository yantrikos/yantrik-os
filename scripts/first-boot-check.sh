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
# Services must actually SERVE, not merely be reported as started.
#
# The service manager logs "Service started" the moment it spawns the process.
# For a long time every service then died inside the SDK on socket bind, and
# the only evidence was one ERROR line the desktop ignored. "Started" is a
# statement about spawning; it says nothing about whether anything answers.
if grep -q "Service failed" "$LOG" 2>/dev/null; then
  fail "a service failed after start: $(grep 'Service failed' "$LOG" | head -1)"
else
  ok "no service reported failure"
fi

STARTED=$(grep -c "Service started" "$LOG" 2>/dev/null); STARTED=${STARTED:-0}
LISTENING=$(grep -c "RPC server listening" "$LOG" 2>/dev/null); LISTENING=${LISTENING:-0}
if [ "$STARTED" -gt 0 ] && [ "$LISTENING" -lt "$STARTED" ]; then
  fail "$STARTED services started but only $LISTENING bound a socket"
else
  ok "$LISTENING/$STARTED started services bound a socket"
fi

# Round-trip real JSON-RPC over each socket. A socket file existing proves a
# bind, not a server.
#
# The log is written with ANSI colour, so field names are wrapped in escape
# sequences and a literal `socket=` never matches. Strip colour before
# extracting — the first version of this check did not, found no socket
# directory, silently skipped, and still reported PASSED. A required
# assertion that cannot run is a FAILURE, not a pass; anything else is the
# same silent-success bug this whole script exists to catch.
PROBE="$(dirname "$0")/service-rpc-probe.py"
SOCK_DIR=$(sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -oE 'socket=[^ ]+' | head -1 \
             | sed 's/socket=//' | xargs -r dirname)
if [ "$LISTENING" -eq 0 ]; then
  echo "skip: no sockets to probe (no service bound)"
elif ! command -v python3 >/dev/null 2>&1; then
  fail "RPC probe needs python3, which is not installed — liveness UNVERIFIED"
elif [ ! -f "$PROBE" ]; then
  fail "RPC probe script missing at $PROBE — liveness UNVERIFIED"
elif [ -z "$SOCK_DIR" ]; then
  fail "could not determine socket directory from the log — liveness UNVERIFIED"
elif python3 "$PROBE" "$SOCK_DIR" > "$PROFILE_HOME/rpc.txt" 2>&1; then
  ok "all services answered RPC ($(tail -1 "$PROFILE_HOME/rpc.txt"))"
else
  fail "not all services answered RPC"
  sed 's/^/      /' "$PROFILE_HOME/rpc.txt" | head -8
fi

# ── Assertion 4 ────────────────────────────────────────────────────────────
# The whole point: a first-time user can ask something and get an answer.
# Everything above proves the shell boots. None of it proves it is useful.
CLI="${YANTRIK_CLI_BIN:-target/release/yantrik}"
if [ -x "$CLI" ]; then
  ANSWER="$PROFILE_HOME/answer.txt"
  if timeout "${ASK_TIMEOUT:-300}" "$CLI" ask --config "$CONFIG" \
       "which processes are using the most memory right now" > "$ANSWER" 2>&1; then
    # A tool must have run. Without one the model is guessing, and a fluent
    # guess is exactly the failure this check exists to catch.
    if grep -qE "tools used: *[a-z_]" "$ANSWER"; then
      ok "answered end-to-end using $(grep -oE 'tools used: *.*' "$ANSWER" | head -1)"
    else
      fail "answered without invoking any tool (ungrounded response)"
      tail -5 "$ANSWER" | sed 's/^/      /'
    fi
  else
    fail "end-to-end ask failed or timed out"
    tail -8 "$ANSWER" | sed 's/^/      /'
  fi
else
  echo "skip: $CLI not built (set YANTRIK_CLI_BIN)"
fi

# ── Assertion 5 ────────────────────────────────────────────────────────────
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
