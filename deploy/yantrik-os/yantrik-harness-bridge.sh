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
# refresh-plugins.sh in #808. Hermes has its own answer: its unit's ExecReload sends SIGUSR1, and the
# gateway then refuses new turns, drains every platform (desktop, Telegram, Slack, IRC), scheduled
# job and API run, and restarts itself (systemd relaunches it), within its own cap
# (restart_after_turn_timeout, 30 minutes). So when the unit can reload, that is what this asks for,
# and it does not wait or look at the run store: Hermes's own count of work is the live answer.
#
# A unit that cannot reload (an older one) keeps the guarded restart, which kills what is in
# flight, so it waits for idle and asks the machine: the shell writes every run it hands a harness
# into its run store, so an unfinished run of Hermes's is a turn in flight. Its limits, said
# plainly. (1) That store sees the desktop's turns only, not Telegram, Slack, IRC or scheduled jobs.
# (2) An update restarts the shell, and a starting shell marks every unfinished run orphaned, yet
# Hermes keeps working on it. So a Hermes run orphaned within the last HARNESS_BRIDGE_ORPHAN_SECONDS
# (15 minutes) counts as a turn in flight, and is not waited on (an orphaned row is final: nothing
# changes it while waiting), just left; a turn that runs longer than that is not protected.
#
# Three rules hold here.
#   - The bridge is found by pid, through systemd's own record of what the gateway holds (the
#     cgroup a process runs in), never by process name. That is what keeps the Mind's bridge out
#     of reach: it runs in its own system unit, so its last path element is not this one.
#   - A restart that cannot happen never stops the update from applying (`apply` still applies; a
#     machine gets out of a bad update with it). But the update does not end "done": its closing
#     line, `status` and `status --porcelain` all say "not done" with the reason, until a later
#     try (`check` retries, without waiting) or the person gives the mind the bridge.
#   - Nothing here is steered by the environment outside selftest: the updater runs as root.

# What to wait on and where to remember it. None of these is read from the environment outside
# selftest (selftest_standins, never true for root): they are stand-ins for the selftest to point at
# a fake desktop, not a way for a caller to steer a root-run restart or a root-owned note.
#   HERMES_GATEWAY_UNIT              the user unit holding the bridge
#   HARNESS_BRIDGE_WAIT_SECONDS      how long a turn in flight is waited on (300), restart path
#   HARNESS_BRIDGE_RELOAD_SECONDS    how long after a reload the bridge is awaited fresh (60)
#   HARNESS_BRIDGE_CANRELOAD         selftest: whether the unit can reload (yes | no)
#   HARNESS_BRIDGE_PERSON_SECONDS    how long a run waiting on the person is waited on (30)
#   HARNESS_BRIDGE_ORPHAN_SECONDS    how long a run orphaned by a shell restart still counts as in flight (900)
#   YANTRIK_PROC, HARNESS_BRIDGE_RUNS_DB, HARNESS_BRIDGE_STATE, HARNESS_BRIDGE_STAMP   /proc, the run store, the note, the signal stamp
harness_bridge_knob() { # harness_bridge_knob <var> <default>
  local v=""
  selftest_standins && v="$(printenv "$1" 2>/dev/null)"
  printf '%s' "${v:-$2}"
}
harness_bridge_unit() { harness_bridge_knob HERMES_GATEWAY_UNIT hermes-gateway.service; }
harness_bridge_proc() { harness_bridge_knob YANTRIK_PROC /proc; }
harness_bridge_seconds() { # harness_bridge_seconds <var> <default>: a number, or else the default
  local v; v="$(harness_bridge_knob "$1" "$2")"
  case "$v" in ''|*[!0-9]*) v="$2" ;; esac
  printf '%s' "$v"
}

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
# It must also be in the desktop account's own manager (.../user@<uid>.service/...): root's own
# user manager, or a system unit with the same name, is not the gateway this restart reaches.
harness_bridge_holds_pid() {
  local path owner uid
  path="$(sed -n 's#^0::/\(.*\)$#\1#p' "$(harness_bridge_proc)/$1/cgroup" 2>/dev/null)"
  owner="$(harness_bridge_owner)" && uid="$(id -u "$owner" 2>/dev/null)" || return 1
  case "/$path" in */user@"$uid".service/*) ;; *) return 1 ;; esac
  [ "${path##*/}" = "$(harness_bridge_unit)" ]
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
  owner="$(harness_bridge_knob HARNESS_BRIDGE_RUNS_DB "")"
  [ -n "$owner" ] && { printf '%s' "$owner"; return 0; }
  owner="$(harness_bridge_owner)" || return 1
  home="$(getent passwd "$owner" 2>/dev/null | cut -d: -f6)"
  [ -n "$home" ] && [ -d "$home" ] || return 1
  printf '%s' "$home/.local/share/yantrik/runs.db"
}

# How many of Hermes's runs are still open, as "<running> <waiting-on-person> <orphaned>". The shell writes
# every run it hands a harness into the store, unfinished ones as running or waiting-on-person
# (crates/yantrik-harness/src/run_store.rs), and under `harness` the id that harness announced --
# "hermes" for this plugin (harnesses/hermes/adapter.py). So a run of hers still open is a turn in
# flight, and another harness's open run is not this gateway's business. A run the shell orphaned
# when it started counts as one for a while (see the header): the update that restarted the shell
# is what orphaned it, and Hermes keeps working on it. Older than that, a crash does not hold this shut.
# Read as the person, so a store kept in their home is read only by someone entitled to it, and
# read-only by URI, so SQLite takes in the WAL the running shell writes through. Fails when no
# answer came: a store that will not open is not a store with nothing in it.
harness_bridge_open_runs() {
  local db owner reader=() out orphan
  db="$1"; orphan="$(harness_bridge_seconds HARNESS_BRIDGE_ORPHAN_SECONDS 900)"
  owner="$(harness_bridge_owner)" || return 1
  [ "$owner" = "$(id -un)" ] || reader=(runuser -u "$owner" --)
  out="$("${reader[@]}" python3 -I -c '
import sqlite3, sys, time
try:
    c = sqlite3.connect("file:%s?mode=ro" % sys.argv[1], uri=True)
    since = int(time.time() * 1000) - int(sys.argv[2]) * 1000
    # An orphaned run is one the shell gave up on when it started; Hermes is still on it.
    r = c.execute("select coalesce(sum(state=?),0), coalesce(sum(state=?),0), coalesce(sum(state=?),0)"
                  " from runs where harness=? and (state in (?,?) or (state=? and updated_at>=?))",
                  ("running", "waiting-on-person", "orphaned", "hermes",
                   "running", "waiting-on-person", "orphaned", since)).fetchone()
    print("%d %d %d" % (r[0], r[1], r[2]))
except Exception as e:
    sys.stderr.write("%s\n" % e); sys.exit(1)
' "$db" "$orphan" 2>/dev/null)" || return 1
  case "$out" in [0-9]*" "[0-9]*" "[0-9]*) ;; *) return 1 ;; esac
  case "$out" in *[!0-9" "]*) return 1 ;; esac
  printf '%s' "$out"
}

# Is Hermes idle? One word: `idle`; `busy` (a turn in flight); `orphaned` (only a run the shell
# orphaned, which may still be going and will not change by waiting); `waiting` (no turn in flight,
# but a run is open waiting for the person to answer, which can stay open for hours); or `unknown` when
# the store could not be asked -- which includes no store at all, since a store that is not there is
# not a store with nothing in it. `unknown` is never a reason to restart.
harness_bridge_idle() {
  local db n running waiting orphaned
  db="$(harness_bridge_runs_db)" || { printf 'unknown'; return 0; }
  [ -f "$db" ] || { printf 'unknown'; return 0; }
  n="$(harness_bridge_open_runs "$db")" || { printf 'unknown'; return 0; }
  read -r running waiting orphaned <<< "$n"
  if [ "$running" != 0 ]; then printf 'busy'
  elif [ "$orphaned" != 0 ]; then printf 'orphaned'
  elif [ "$waiting" != 0 ]; then printf 'waiting'
  else printf 'idle'; fi
}

# "N minutes" or "N seconds", for a bound a person reads.
harness_bridge_span() {
  if [ "$1" -ge 60 ]; then printf '%s minutes' "$(($1 / 60))"; else printf '%s seconds' "$1"; fi
}

# Wait for Hermes to finish what it is doing; on failure leave the reason in $HARNESS_BRIDGE_IDLE
# (`busy`, `waiting`, `orphaned` or `unknown`). A turn in flight is waited on for <bound> seconds. A run waiting
# on the person is waited on for a short bound only: it is the person being waited on, not a turn
# that will finish by itself, and every update would otherwise stand five minutes in front of
# someone who may be the very person it is waiting for. `unknown` is not waited on: no store, or
# one that will not open, is not something a minute fixes. Nor is an orphaned run: its row is final.
harness_bridge_wait_idle() {
  local waited=0 bound="$1" person limit answer
  person="$(harness_bridge_seconds HARNESS_BRIDGE_PERSON_SECONDS 30)"
  while :; do
    answer="$(harness_bridge_idle)"
    [ "$answer" = idle ] && return 0
    HARNESS_BRIDGE_IDLE="$answer"
    case "$answer" in unknown|orphaned) return 1 ;; esac
    limit="$bound"
    if [ "$answer" = waiting ] && [ "$person" -lt "$bound" ]; then limit="$person"; fi
    if [ "$waited" = 0 ] && [ "$limit" -gt 0 ]; then
      if [ "$answer" = waiting ]; then
        say "Hermes is waiting for the person to answer; giving it $(harness_bridge_span "$limit") before leaving the old bridge"
      else
        say "Waiting for Hermes to finish what it is doing before giving the mind the new bridge"
        info "up to $(harness_bridge_span "$limit"); nothing else is touched"
      fi
    fi
    [ "$waited" -ge "$limit" ] && return 1
    sleep 10
    waited=$((waited + 10))
  done
}

# Reload or restart the gateway as the person (<verb>: reload | restart). In selftest mode,
# <HARNESS_BRIDGE_RESTART> stands in for it: `fail` for one that does not happen, anything else a
# file the ask is written to, so the selftest can read which verb, unit and account -- the things
# that must not be wrong.
#
# Two failures, told apart by the number: 1 when the ask itself did not happen, 2 when the ask
# happened and only the stamp of it did not. They mean different things to whoever is reporting the
# update. A stamp is what the half hour of the retry is read from, so its loss is worth saying out
# loud -- and so a caller that waits on the guard needs to know it did not hold -- but a gateway
# that was asked and did not answer with a stamp is a mind given the new bridge all the same, and
# must not be reported as one still on the old.
harness_bridge_signal() {
  harness_bridge_signal_do "$@" || return 1
  harness_bridge_stamp_write || return 2
}
harness_bridge_signal_do() {
  if selftest_standins && [ -n "${HARNESS_BRIDGE_RESTART:-}" ]; then
    [ "$HARNESS_BRIDGE_RESTART" = fail ] && return 1
    printf '%s %s as %s\n' "$1" "$(harness_bridge_unit)" "$(harness_bridge_owner)" >> "$HARNESS_BRIDGE_RESTART"
    return 0
  fi
  harness_bridge_userctl "$1" "$(harness_bridge_unit)"
}
harness_bridge_restart() { harness_bridge_signal restart; }

# When the gateway was last signalled, for the half-hour guard of the retry. The note cannot carry it
# on its own: it is root's, and a `check` run as the person (the shell runs it) can neither write
# it nor find one on a machine whose old updater left none. So the stamp lives in the desktop
# account's runtime directory (/run/user/<uid>), which that person writes and root reads, and
# which a reboot clears along with the stale gateway it guards. Its mtime is the time.
harness_bridge_stamp_file() {
  local v owner uid; v="$(harness_bridge_knob HARNESS_BRIDGE_STAMP "")"
  [ -n "$v" ] && { printf '%s' "$v"; return 0; }
  owner="$(harness_bridge_owner)" || return 1
  # Whose runtime directory this is: the owner's, so that the person's write and root's read of the
  # mtime agree on one file. Turning the owner's name back into a number was the step that went
  # wrong -- a `check` under the shell's sudo rule came out as /run/user/0, and the person read the
  # refused write that followed -- so a caller that already is the owner uses its own number, which
  # the kernel knows without asking anywhere. Only when they differ is the owner looked up: root,
  # whose own directory is never this one, and root reads the file whatever the mode.
  uid="$(id -u)"
  [ "$(id -un)" = "$owner" ] || uid="$(id -u "$owner" 2>/dev/null)" || return 1
  printf '/run/user/%s/yantrik-hermes-bridge-signalled' "$uid"
}
# Written as the desktop's account, never as root: the directory is that person's, so anything in
# it (a link to /etc/shadow, say) is theirs to plant, and root following it would hand them a root
# write (review of #1110). As them, `touch` can reach only what they could already touch. No chmod:
# root reads the mtime whatever the mode. Fails, and says so in words that mean it, unless the file
# was left newer than it was: the half hour of the guard is read off that mtime, and a write that
# did not happen but was never mentioned is how a retry came back and asked for a second reload
# while the first was still draining. touch keeps its own reason off the person's screen; what it
# means is said here, because a stamp is written by nothing else and nowhere else.
harness_bridge_stamp_write() {
  local f owner was now
  f="$(harness_bridge_stamp_file 2>/dev/null)"
  if [ -n "$f" ]; then
    # Whole nanoseconds, not seconds: the selftest signals twice within one, and a stamp that really
    # was rewritten must not read as one that was not.
    was="$(stat -c '%.9Y' -- "$f" 2>/dev/null)"
    if [ "$(id -u)" = 0 ]; then
      owner="$(harness_bridge_owner)" || owner=""
      [ -n "$owner" ] && runuser -u "$owner" -- touch -- "$f" 2>/dev/null
    else
      touch -- "$f" 2>/dev/null
    fi
    now="$(stat -c '%.9Y' -- "$f" 2>/dev/null)" && [ "$now" != "$was" ] && return 0
  fi
  printf '  no stamp of the new bridge at %s -- the half hour the retry waits on will not hold\n' \
    "${f:-unknown}" >&2
  return 1
}
# Was the gateway signalled, or the note written, within the last 30 minutes?
harness_bridge_signalled_recently() {
  local f
  for f in "$(harness_bridge_note_file)" "$(harness_bridge_stamp_file 2>/dev/null)"; do
    [ -n "$f" ] && [ -n "$(find "$f" -mmin -30 2>/dev/null)" ] && return 0
  done
  return 1
}

# Can the unit reload, which for Hermes is its drain-aware restart.
harness_bridge_can_reload() {
  if selftest_standins && [ -n "${HARNESS_BRIDGE_RESTART:-}" ]; then
    [ "$(harness_bridge_knob HARNESS_BRIDGE_CANRELOAD no)" = yes ]; return
  fi
  [ "$(harness_bridge_userctl show -p CanReload --value "$(harness_bridge_unit)" 2>/dev/null)" = yes ]
}

# Is no gateway-held bridge old any more? Looked at again for up to <seconds>.
harness_bridge_fresh_within() {
  local waited=0
  while :; do
    [ -z "$(harness_bridge_stale_pids "$(stale_processes)")" ] && return 0
    [ "$waited" -ge "$1" ] && return 1
    sleep 2
    waited=$((waited + 2))
  done
}

# The update's one call, at the point where it found an old bridge; <stale> is the same
# stale_processes answer report_stale just printed, <wait> an optional bound in seconds on waiting
# for a turn in flight (the retry from `check` passes 0: it never makes anyone wait). Says what it
# is doing and what came of it. It does not fail: the update has been applied, and what could not
# be done is said here and left as a note, which makes `status` and the update's closing line
# report "not done" with the reason until a later try, or the person, gives the mind the bridge.
harness_bridge_update() {
  local why="" unit bound="${2:-}" now rc=0
  unit="$(harness_bridge_unit)"
  case "$bound" in ''|*[!0-9]*) bound="$(harness_bridge_seconds HARNESS_BRIDGE_WAIT_SECONDS 300)" ;; esac
  # Only a bridge this gateway holds: an old yos-mcp in the Mind's unit is not this restart's to
  # make, and waiting for a gateway that owns nothing would be a delay with no subject.
  [ -n "$(harness_bridge_stale_pids "$1")" ] || return 0
  # Once a run: an apply reaches the point where an old bridge is reported twice.
  [ -n "${HARNESS_BRIDGE_TRIED:-}" ] && return 0
  HARNESS_BRIDGE_TRIED=1
  if harness_bridge_can_reload; then
    # Hermes drains its own work before it restarts, so there is nothing to wait for here, only to
    # confirm: the bridge is new once it has restarted, which may be up to its own cap from now.
    local fresh; fresh="$(harness_bridge_seconds HARNESS_BRIDGE_RELOAD_SECONDS 60)"
    [ "${2:-}" = 0 ] && fresh=0
    # Only 1 is a reload that did not happen. 2 is the stamp of it, which says a line already and
    # leaves the mind given the new bridge all the same -- reporting that as a failed reload would
    # send the person to restart a unit that has reloaded.
    rc=0; harness_bridge_signal reload || rc=$?
    if [ "$rc" = 1 ]; then why="asking $unit to reload failed"
    elif harness_bridge_fresh_within "$fresh"; then
      harness_bridge_note_clear; say "Gave the mind the new bridge: $unit reloaded"; return 0
    else
      why="asked $unit to reload; Hermes restarts once it finishes what it is doing, and the bridge is not new yet"
    fi
  elif ! harness_bridge_wait_idle "$bound"; then
    case "${HARNESS_BRIDGE_IDLE:-busy}" in
      orphaned) why="Hermes was still working on a turn the update cut off, so it was left running" ;;
      unknown) why="could not tell whether Hermes was busy (its run store was not readable)" ;;
      waiting) why="Hermes is waiting for the person to answer, so it was left running" ;;
      *) why="Hermes was still working after $(harness_bridge_span "$bound")" ;;
    esac
  else
    # Idle a moment ago is not idle now: a turn that started since would be killed by the restart,
    # which is the "Recovered reply" harm this exists to avoid. Ask again, right before.
    now="$(harness_bridge_idle)"
    if [ "$now" != idle ]; then
      why="Hermes started a turn just as the restart was due, so it was left running"
    else
      rc=0; harness_bridge_restart || rc=$?
      [ "$rc" = 1 ] && why="restarting $unit failed"
    fi
  fi
  [ -n "$why" ] || { harness_bridge_note_clear
    say "Gave the mind the new bridge: restarted $unit"; return 0; }
  why="$why (as of $(date -u '+%Y-%m-%d %H:%MZ'))"
  harness_bridge_note_write "$why"
  warn "the mind is still on the old bridge: $why -- restart $unit when Hermes is idle"
}

# A later try, for whatever already runs the updater. Nothing runs it on a timer; `check` is run by
# the shell once after boot and on every visit to About, so that is where this is called from. Only
# when a gateway's bridge is old (not when a note says so); never waits.
harness_bridge_retry() {
  local stale; stale="$(stale_processes)"
  # The live test `status` uses, not the note: an update applied by an older updater never ran the
  # bridge step and left no note, and neither does a manual install or a crash mid-apply.
  [ -n "$(harness_bridge_stale_pids "$stale")" ] || return 0
  # A reload asks Hermes to drain first, which can take up to its own 30-minute cap; signalling it
  # again on every About visit cannot make that faster, and its handling of a second request while
  # one is pending is not ours to rely on (review of #1110). One retry per half hour, by the
  # newer of the note and the stamp of the last signal.
  ! harness_bridge_signalled_recently || return 0
  HARNESS_BRIDGE_TRIED=""
  harness_bridge_update "$stale" 0
  # Nothing old is left running: the note is out of date, put it away.
  harness_bridge_verdict >/dev/null || true
}

# Why the bridge is still the old one, as the person reading `status` needs it. Recorded by the
# update that could not restart the gateway, and put away by the next update that could, or by a
# status that finds the bridge current again. A note, not a verdict: the check is live. It is
# root's file: a `check` retry run as the person can restart the gateway but can neither write nor
# remove it, so after such a retry `status` may show the earlier reason until the next update or
# root's `status`; whether the bridge is reported current is the live check either way.
harness_bridge_note_file() { harness_bridge_knob HARNESS_BRIDGE_STATE "$UPDATE_STATE_DIR/hermes-bridge"; }

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

# Is the mind on the current bridge? Silent and 0 when it is (or there is none to be on); otherwise
# prints why not and returns 1. Live: it asks what is running now, so a person who restarts Hermes
# clears it themselves, and a rollback that left the gateway on a bridge newer than the restored
# build is caught too. The one answer behind `status`, `status --porcelain` and the update's
# closing line.
harness_bridge_verdict() {
  local pid why
  pid="$(harness_bridge_stale_pids "$(stale_processes)")"
  [ -n "$pid" ] || { harness_bridge_note_clear; return 0; }
  why="$(harness_bridge_note)"
  [ -n "$why" ] || why="no restart of $(harness_bridge_unit) has happened since the last update"
  printf 'Hermes is on the old bridge: %s (pid %s)' "$why" "$pid"
  return 1
}

# One line for `status`: `current`, or `not done: <reason>`.
harness_bridge_state() {
  local why
  why="$(harness_bridge_verdict)" && { printf 'current'; return 0; }
  printf 'not done: %s' "$why"
}
