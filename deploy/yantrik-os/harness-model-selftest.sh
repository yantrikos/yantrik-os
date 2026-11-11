#!/bin/bash
# Checks for the two root-side verbs a moved harness needs from the desktop (#411), no root and no
# systemd needed (both are recorders here): bash deploy/yantrik-os/harness-model-selftest.sh
#   - `harness model hermes apply|revert`: "Use Yantrik models" for a Hermes the person's desktop cannot
#     write. The settings reach the account's tool on stdin and never in an argument list, nothing is
#     run unless Hermes has moved, only the two words are accepted, and a failed tool changes nothing else.
#   - `harness reload hermes`: the drain-aware restart the bridge update asks for.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
fails=0
is() { if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"; else printf '  FAIL %s\n       want: %s\n       got:  %s\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi; }
has() { case "$3" in *"$2"*) printf '  ok   %s\n' "$1" ;; *) printf '  FAIL %s\n       missing: %s\n' "$1" "$2"; fails=$((fails + 1)) ;; esac; }
lacks() { case "$3" in *"$2"*) printf '  FAIL %s\n       found: %s\n' "$1" "$2"; fails=$((fails + 1)) ;; *) printf '  ok   %s\n' "$1" ;; esac; }

work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
PREFIX="$work/opt"; BIN_DIR="$PREFIX/bin"
say() { :; }; info() { printf 'info: %s\n' "$*" >> "$work/said"; }; warn() { printf 'warn: %s\n' "$*" >> "$work/said"; }
fail() { printf 'FAIL: %s\n' "$*" >> "$work/said"; exit 1; }
id() { if [ "${1:-}" = -u ] && [ -z "${2:-}" ]; then printf '%s' "${FAKE_UID:-0}"; else command id "$@"; fi; }
selftest_standins() { true; }
export YANTRIK_SELFTEST=1 HI_STATE="$work/state" HI_BASE="$work/mind/harness" HI_USER=yk411m
mkdir -p "$work/state" "$PREFIX/share/harnesses/lib/install" "$work/mind/harness/hermes"
: > "$PREFIX/share/harnesses/lib/install/hermes-model.sh"; : > "$PREFIX/share/harnesses/lib/install/hermes-guard.sh"
# shellcheck source=deploy/yantrik-os/harness-isolation.sh
. "$here/harness-isolation.sh"
# shellcheck source=deploy/yantrik-os/yantrik-harness-bridge.sh
. "$here/yantrik-harness-bridge.sh"
hi_relay() { cat >/dev/null; }
# The account's tool: records what it was run with and what it read on stdin.
hi_task() {
  case "$*" in
    *hermes-guard.sh*) printf '%s\n' "$*" >> "$work/guard-args"; cat >/dev/null; return "${FAKE_GUARD_RC:-0}" ;;
  esac
  printf '%s\n' "$*" > "$work/task-args"; cat > "$work/task-stdin"; return "${FAKE_TASK_RC:-0}"
}
systemctl() { printf '%s\n' "$*" >> "$work/systemctl"; return "${FAKE_SYSTEMCTL_RC:-0}"; }

TOKEN='ygw-SECRET-token-123'
BODY="{\"default\":\"picked\",\"provider\":\"custom\",\"base_url\":\"http://127.0.0.1:7460/v1\",\"api_key\":\"$TOKEN\"}"
run() { ( "$@" ) >/dev/null 2>&1; echo $?; }
reset() { rm -f "$work/task-args" "$work/task-stdin" "$work/systemctl" "$work/said" "$work/guard-args"; }

echo "== harness model hermes"
reset
is "not moved: refused, nothing run" "1 no" "$(printf '%s' "$BODY" | run hi_model hermes apply) $([ -e "$work/task-args" ] && echo yes || echo no)"
: > "$work/state/hermes.isolated"
reset
is "moved: the settings are applied" "0" "$(printf '%s' "$BODY" | run hi_model hermes apply)"
is "... the account's tool got them on stdin" "$BODY" "$(cat "$work/task-stdin")"
args="$(cat "$work/task-args")"
lacks "... and not in any argument" "$TOKEN" "$args"
has "... run as the account's tool, apply" "hermes-model.sh apply" "$args"
has "... with HERMES_HOME the account's own" "HERMES_HOME=$work/mind/harness/hermes/.hermes" "$args"
is "... then Hermes drains and restarts onto it" "reload-or-restart hermes-gateway.service" "$(cat "$work/systemctl")"
lacks "... and nothing said holds the token" "$TOKEN" "$(cat "$work/said")"
reset
is "a reload that fails is not reported as done: the command fails" "1" "$(printf '%s' "$BODY" | FAKE_SYSTEMCTL_RC=1 run hi_model hermes apply)"
printf '%s' "$BODY" | FAKE_SYSTEMCTL_RC=1 run hi_model hermes apply >/dev/null
has "... and says the settings were saved but Hermes keeps the old model" "keeps the old model" "$(cat "$work/said")"
lacks "... and not that it finishes its turns and restarts" "finishes its turns and restarts" "$(cat "$work/said")"
reset
is "revert asks for revert, with no settings" "0 hermes-model.sh revert" "$(run hi_model hermes revert </dev/null) $(grep -o 'hermes-model.sh revert' "$work/task-args")"
reset
is "a tool that fails: the settings are not applied, Hermes is not restarted" "1 no" \
  "$(FAKE_TASK_RC=1 bash -c 'exit 0' >/dev/null; FAKE_TASK_RC=1; export FAKE_TASK_RC; printf '%s' "$BODY" | run hi_model hermes apply) $([ -e "$work/systemctl" ] && echo yes || echo no)"
unset FAKE_TASK_RC
is "another word is refused" "1" "$(printf '%s' "$BODY" | run hi_model hermes run)"
is "another harness is refused" "1" "$(printf '%s' "$BODY" | run hi_model pi apply)"
is "an empty body is refused for apply" "1" "$(run hi_model hermes apply </dev/null)"
is "not root: refused" "1" "$(FAKE_UID=1000; export FAKE_UID; printf '%s' "$BODY" | run hi_model hermes apply)"
is "an oversized body is cut at 8 KiB, not passed whole" "8192" "$(reset; head -c 20000 /dev/zero | tr '\0' 'a' | ( hi_model hermes apply ) >/dev/null 2>&1; wc -c < "$work/task-stdin")"
is "the verb list has model" "yes" "$(grep -q 'install|restart|reload|model|run|status' "$here/harness-isolation.sh" && echo yes || echo no)"

echo "== L4: the toolset allowlist is held after anything that could rewrite the file"
reset
printf '%s' "$BODY" | run hi_model hermes apply >/dev/null
is "apply: the guard ran after the model change, as the account's tool" "1" "$(grep -c 'hermes-guard.sh after' "$work/guard-args")"
reset
is "apply, allowlist could not be held: refused, the gateway is stopped and not reloaded" "1 stop hermes-gateway.service" \
  "$(printf '%s' "$BODY" | FAKE_GUARD_RC=1 run hi_model hermes apply) $(cat "$work/systemctl")"
reset
printf '%s' "$BODY" | FAKE_GUARD_RC=3 run hi_model hermes apply >/dev/null
is "apply, allowlist had been widened and is put right: still reloaded" "reload-or-restart hermes-gateway.service" "$(cat "$work/systemctl")"
reset
( hi_run hermes model ) >/dev/null 2>&1
is "run hermes model: the guard ran after it" "1" "$(grep -c 'hermes-guard.sh after' "$work/guard-args")"
reset
( FAKE_GUARD_RC=3 hi_run hermes model ) >/dev/null 2>&1
is "... widened by it and put right: the gateway is reloaded onto the right file" "try-reload-or-restart hermes-gateway.service" "$(cat "$work/systemctl")"
reset
is "... not put right: the gateway is stopped, and the command fails" "1 stop hermes-gateway.service" "$(FAKE_GUARD_RC=1 run hi_run hermes model) $(cat "$work/systemctl")"
reset
( FAKE_GUARD_RC=3 FAKE_SYSTEMCTL_RC=1 hi_run hermes model ) >/dev/null 2>&1
has "... and when that reload fails it says so" "would not reload onto the corrected file" "$(cat "$work/said")"
reset
( hi_run hermes status ) >/dev/null 2>&1
is "run hermes status: no guard (it cannot write the file)" "no" "$([ -e "$work/guard-args" ] && echo yes || echo no)"
unit_text="$(cat "$here/../../config/systemd/hermes-gateway.service")"
has "the gateway checks the allowlist before every start, as the account, in the scope" "ExecStartPre=/opt/yantrik/bin/yantrik-init --scope-abstract-unix /bin/sh /opt/yantrik/share/harnesses/lib/install/hermes-guard.sh start" "$unit_text"

echo "== harness reload hermes"
reset
export HARNESS_BRIDGE_SHAPE=system HARNESS_BRIDGE_RESTART="$work/signalled" HARNESS_BRIDGE_STAMP="$work/stamp"
is "moved: asks the unit to reload, as root" "0 reload hermes-gateway.service as root" "$(run hi_reload hermes) $(cat "$work/signalled")"
is "... and leaves root's stamp" "yes" "$([ -e "$work/stamp" ] && echo yes || echo no)"
rm -f "$work/signalled"
rm -f "$work/state/hermes.isolated"
is "not moved: refused, nothing asked" "1 no" "$(run hi_reload hermes) $([ -e "$work/signalled" ] && echo yes || echo no)"
: > "$work/state/hermes.isolated"
is "not root: refused" "1" "$(FAKE_UID=1000; export FAKE_UID; run hi_reload hermes)"

if [ "$fails" = 0 ]; then echo "all checks passed"; else echo "$fails FAILED"; exit 1; fi
