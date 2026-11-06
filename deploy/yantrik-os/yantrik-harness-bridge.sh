#!/bin/bash
# Sourced, not run: giving the mind back the bridge it is supposed to be running on.
#
# yos-mcp is Python, it reads its code when it starts, and it is where the desktop's taint and
# approval rules live. A Hermes that has been up across `yantrik-update apply` is therefore
# still enforcing the rules of the build it started under, and a security fix to that bridge
# reaches the mind only when hermes-gateway restarts. yantrik-update already found the process
# (stale_processes) and, until now, only printed a hint about how to fix it by hand (#1102).
#
# Restarting a gateway is not free: it kills a turn in flight, which Hermes then replays into
# the next turn as a "Recovered reply" -- the lesson that took restarts out of
# refresh-plugins.sh in #808. So this waits for idle, and asks the machine, which does know: the
# shell writes every run it hands a harness into its run store, so an unfinished run of Hermes's
# is a turn in flight. That is the question below, not a proxy for it.
#
# Two rules hold here.
#   - The bridge is found by pid, through systemd's own record of what the gateway holds (the
#     cgroup a process runs in), never by process name. That is what keeps the Mind's bridge out
#     of reach: it runs in its own system unit, so its last path element is not this one.
#   - A failed restart never fails the update. It says plainly, in the update's own output and in
#     `yantrik-update status`, that the mind is on the old bridge and why.

# What to wait on and where to remember it, all overridable so the selftest can point them at a
# fake desktop instead of the one this script is running on.
: "${HERMES_GATEWAY_UNIT:=hermes-gateway.service}"
: "${HARNESS_BRIDGE_WAIT_SECONDS:=300}"
# The shell's run store, and /proc: the same knobs yantrik-update's selftest already uses to ask
# about a fake machine (shell_session reads YANTRIK_PROC the same way). The store by name, since
# the selftest's is a file of its own and this account's home is not where it is.
: "${YANTRIK_PROC:=/proc}"
: "${HARNESS_BRIDGE_RUNS_DB:=}"

# Whose gateway this is. As root, the desktop's account and never root itself; as the person,
# this account. Fails when there is no desktop to ask.
harness_bridge_owner() {
  local owner
  if [ "$(id -u)" = 0 ]; then
    owner="$(desktop_owner)"
    case "$owner" in ''|root|*[!a-zA-Z0-9_-]*) return 1 ;; esac
    printf '%s' "$owner"
  else
    id -un
  fi
}

# Ask that account's own manager. `systemctl --user -M <owner>@` reaches the local user manager
# of <owner>, which is how the mind migration already stops a stray unit in someone's session;
# run as that person already, it needs only the runtime dir this process may not have inherited.
harness_bridge_userctl() {
  local owner; owner="$(harness_bridge_owner)" || return 1
  if [ "$owner" != "$(id -un)" ]; then
    systemctl --user -M "$owner@" "$@"
    return $?
  fi
  XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" systemctl --user "$@"
}

# Did the gateway hold this pid? systemd says so in the process's cgroup: the last element of the
# path is the unit it runs in. By pid, never by name.
harness_bridge_holds_pid() {
  local unit
  unit="$(sed -n 's#^0::/\(.*\)$#\1#p' "$YANTRIK_PROC/$1/cgroup" 2>/dev/null | sed 's#.*/##')"
  [ "$unit" = "$HERMES_GATEWAY_UNIT" ]
}

# Of the processes an update left behind, which are the gateway's bridge. <stale> is
# stale_processes' answer, and its yos-mcp entries carry the pid: that is where the rule about
# what counts as stale lives, so it is what this reads rather than a second copy of it.
harness_bridge_stale_pids() {
  local pid out=""
  for pid in $(printf '%s' " $1" | tr ' ' '\n' | sed -n 's/^yos-mcp(pid-\([0-9][0-9]*\))$/\1/p'); do
    harness_bridge_holds_pid "$pid" && out="$out $pid"
  done
  printf '%s' "${out# }"
}

# The store to ask: $XDG_DATA_HOME/yantrik/runs.db of the desktop's account (the path the shell
# opens, crates/yantrik-ui/src/wire/harness.rs), which no Yantrik session moves off the default.
harness_bridge_runs_db() {
  local owner home
  [ -n "$HARNESS_BRIDGE_RUNS_DB" ] && { printf '%s' "$HARNESS_BRIDGE_RUNS_DB"; return 0; }
  owner="$(harness_bridge_owner)" || return 1
  home="$(getent passwd "$owner" 2>/dev/null | cut -d: -f6)"
  [ -n "$home" ] && [ -d "$home" ] || return 1
  printf '%s' "$home/.local/share/yantrik/runs.db"
}

# How many of Hermes's runs are still open. The shell writes every run it hands a harness into
# the store, unfinished ones as running or waiting-on-person (crates/yantrik-harness/src/run_store.rs),
# and under `harness` the id that harness announced -- "hermes" for this plugin
# (harnesses/hermes/adapter.py). So a run of hers still open is a turn in flight, and another
# harness's open run is not this gateway's business: restarting it cannot interrupt them. A run the
# shell orphaned when it started is already final, so a crash does not hold this shut; a run
# waiting on the person is still live, so it does, and the bound below is what ends that wait.
# Read as the person, so a store kept in their home is read only by someone entitled to it, and
# read-only by URI, so SQLite takes in the WAL the running shell writes through. Fails when no
# answer came: a store that will not open is not a store with nothing in it.
harness_bridge_open_runs() {
  local db owner reader=() out
  db="$1"
  owner="$(harness_bridge_owner)" || return 1
  [ "$owner" = "$(id -un)" ] || reader=(runuser -u "$owner" --)
  out="$("${reader[@]}" python3 -I -c '
import sqlite3, sys
try:
    c = sqlite3.connect("file:%s?mode=ro" % sys.argv[1], uri=True)
    print(c.execute("select count(*) from runs"
                    " where state in (?,?) and harness=?",
                    ("running", "waiting-on-person", "hermes")).fetchone()[0])
except Exception as e:
    sys.stderr.write("%s\n" % e); sys.exit(1)
' "$db" 2>/dev/null)" || return 1
  case "$out" in *[!0-9]* | '') return 1 ;; esac
  printf '%s' "$out"
}

# Is Hermes idle? One word: `idle`, `busy`, or `unknown` when the store could not be asked --
# which includes no store at all, since a store that is not there is not a store with nothing in
# it. `unknown` is never a reason to restart: not being able to ask is not being idle.
harness_bridge_idle() {
  local db n
  db="$(harness_bridge_runs_db)" || { printf 'unknown'; return 0; }
  [ -f "$db" ] || { printf 'unknown'; return 0; }
  n="$(harness_bridge_open_runs "$db")" || { printf 'unknown'; return 0; }
  [ "$n" = 0 ] && printf 'idle' || printf 'busy'
}

# Wait for Hermes to finish what it is doing, and leave the reason it did not in
# $HARNESS_BRIDGE_IDLE as `busy` or `unknown`. Only `busy` is waited on: the two ways to come back
# `unknown` -- no store where one should be, or a store that will not open -- are not things a
# minute of waiting fixes, and the store opens read-only through the WAL precisely so that a
# writing shell does not make it unreadable. The bound is what ends a wait that a run sitting in
# waiting-on-person would otherwise hold shut forever, and a knob nobody can be trusted with is
# treated as the default rather than as an instruction to loop for good.
harness_bridge_wait_idle() {
  local waited=0 bound="$HARNESS_BRIDGE_WAIT_SECONDS" answer=busy
  case "$bound" in ''|*[!0-9]*) bound=300 ;; esac
  while { answer="$(harness_bridge_idle)"; [ "$answer" != idle ]; }; do
    if [ "$answer" = unknown ]; then HARNESS_BRIDGE_IDLE=unknown; return 1; fi
    if [ "$waited" = 0 ]; then
      say "Waiting for Hermes to finish what it is doing before giving the mind the new bridge"
      info "up to $((bound / 60)) minutes; nothing else is touched"
    fi
    [ "$waited" -ge "$bound" ] && { HARNESS_BRIDGE_IDLE=busy; return 1; }
    sleep 10
    waited=$((waited + 10))
  done
}


# Restart the gateway as the person. In selftest mode, <HARNESS_BRIDGE_RESTART> stands in for it:
# `fail` for a restart that does not happen, anything else a file the ask is written to, so the
# selftest can read which unit was restarted and as whom -- the two things that must not be wrong.
harness_bridge_restart() {
  if selftest_standins && [ -n "${HARNESS_BRIDGE_RESTART:-}" ]; then
    [ "$HARNESS_BRIDGE_RESTART" = fail ] && return 1
    printf 'restart %s as %s\n' "$HERMES_GATEWAY_UNIT" "$(harness_bridge_owner)" >> "$HARNESS_BRIDGE_RESTART"
    return 0
  fi
  harness_bridge_userctl restart "$HERMES_GATEWAY_UNIT"
}

# The update's one call, at the point where it found an old bridge; <stale> is the same
# stale_processes answer report_stale just printed. Says what it is doing and what came of it, and
# never fails the update: what it could not do is said here and left as a note for `status`, which
# is where the person finds the mind is still on the old rules and why.
harness_bridge_update() {
  local why="" bound="${HARNESS_BRIDGE_WAIT_SECONDS:-300}"
  case "$bound" in ''|*[!0-9]*) bound=300 ;; esac
  # Only a bridge this gateway holds: an old yos-mcp in the Mind's unit is not this restart's to
  # make (rule 3), and waiting five minutes for a gateway that owns nothing would be a delay with
  # no subject. What is left of that case is reported by the ordinary stale warning.
  [ -n "$(harness_bridge_stale_pids "$1")" ] || return 0
  # Once a run: an apply reaches the point where an old bridge is reported twice, and a wait is
  # five minutes that a person is watching.
  [ -n "${HARNESS_BRIDGE_TRIED:-}" ] && return 0
  HARNESS_BRIDGE_TRIED=1
  if ! harness_bridge_wait_idle; then
    # Two different reasons for the same refusal, and the person has to be able to tell them
    # apart: one says come back later, the other says the run store was never found at all.
    [ "${HARNESS_BRIDGE_IDLE:-busy}" = unknown ] \
      && why="could not tell whether Hermes was busy (its run store was not readable)" \
      || why="Hermes was still working after $((bound / 60)) minutes"
  elif ! harness_bridge_restart; then
    why="restarting $HERMES_GATEWAY_UNIT failed"
  fi
  [ -n "$why" ] || { harness_bridge_note_clear
    say "Gave the mind the new bridge: restarted $HERMES_GATEWAY_UNIT"; return 0; }
  why="$why (as of $(date -u '+%Y-%m-%d %H:%MZ'))"
  harness_bridge_note_write "$why"
  warn "the mind is still on the old bridge: $why -- restart $HERMES_GATEWAY_UNIT when Hermes is idle"
}

# Why the bridge is still the old one, as the person reading `status` needs it. Recorded by the
# update that could not restart the gateway, and put away by the next update that could, or by a
# status that finds the bridge current again. A note, not a verdict: the check is live.
harness_bridge_note_file() { printf '%s' "${HARNESS_BRIDGE_STATE:-$UPDATE_STATE_DIR/hermes-bridge}"; }

harness_bridge_note_write() {
  local f d; f="$(harness_bridge_note_file)"; d="$(dirname "$f")"
  # A directory that is already there is not this note's to re-mode: as $UPDATE_STATE_DIR it is
  # root's record of accepted updates, whose mode yantrik-update-verify checks, and 0750 on it
  # would hide the note from the person running `status`. So create when missing, world-traversable
  # like the serial files root already keeps there, and otherwise leave it exactly as it is.
  [ -d "$d" ] || install -d -m 0755 "$d" 2>/dev/null
  printf '%s\n' "$1" > "$f" 2>/dev/null && chmod 0644 "$f" 2>/dev/null
}

harness_bridge_note_clear() { rm -f "$(harness_bridge_note_file)" 2>/dev/null || true; }

harness_bridge_note() {
  local f; f="$(harness_bridge_note_file)"
  [ -f "$f" ] && sed -n '1p' "$f"
  return 0
}

# What `status` says about the bridge, one line: `current`, or `not done: ...` with why. Live: it asks what
# is running now, so a person who restarts Hermes clears this themselves, and a rollback that
# left the gateway on a bridge newer than the restored build is caught too.
harness_bridge_state() {
  local stale pid why
  stale="$(stale_processes)"
  pid="$(harness_bridge_stale_pids "$stale")"
  [ -n "$pid" ] || { harness_bridge_note_clear; printf 'current'; return 0; }
  why="$(harness_bridge_note)"
  [ -n "$why" ] || why="no restart of $HERMES_GATEWAY_UNIT has happened since the last update"
  printf 'not done: Hermes is on the old bridge: %s (pid %s)' "$why" "$pid"
}
