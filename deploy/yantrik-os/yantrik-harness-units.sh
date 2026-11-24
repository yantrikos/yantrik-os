#!/bin/bash
# Sourced, not run: giving the harnesses the code an update installed (#1394).
#
# A harness (Pi, DeepSeek, OpenClaw) is Python the person's user unit started, and Python reads
# its code once. An update replaces share/harnesses/<id>/ and share/harnesses/lib/ under it and
# the running process carries on with what it started with, so a fix to a harness -- a security
# fix too -- reached a machine only at the next login, while the update said "healthy". The
# bridge rule (yantrik-harness-bridge.sh, #1102) is the model and this reuses its machinery: the
# owner and their manager (harness_bridge_userctl), the run store and its idle answer
# (harness_bridge_idle <id>), the bounds (HARNESS_BRIDGE_WAIT_SECONDS, _PERSON_SECONDS, _ORPHAN_SECONDS)
# and the knobs that only selftest may steer. Its limits are the bridge rule's: the store sees the
# desktop's turns only, and an orphaned run counts as a turn in flight for 15 minutes.
#
# Three rules hold here, the same three.
#   - A harness is found by its unit's cgroup (the last path element, inside the desktop account's
#     own manager), never by process name. Which units there are is what ships: every
#     share/harnesses/<id>/yantrik-<id>.service.
#   - Changed means the files the unit runs are newer than the unit. Its code changed when the
#     newest inode change time (ctime) under share/harnesses/<id> and share/harnesses/lib is later
#     than the start of the unit's oldest process (/proc/<pid>'s own time, as stale_processes
#     reads it). ctime, not mtime: the updater's rsync keeps the bundle's mtimes, but a file it
#     writes gets its ctime when it is written, and a file it left alone (--checksum) keeps its
#     old one, so a harness whose code did not change is not touched. No stamp is written, and
#     the answer is live, so a person who restarts the unit clears it themselves.
#   - A busy harness is left running, an update that cannot restart one still applies, and the
#     update then ends "not done" with the reason (the note, `status`) until a later try
#     (`check` retries, without waiting) or the person restarts the unit. A unit that can reload
#     (ExecReload) is asked to, and is neither waited on nor looked up, as Hermes's is.
#   HARNESS_UNITS_SHARE   selftest: the folder holding the harnesses (default $SHARE_DIR/harnesses)
#   HARNESS_UNITS_STATE   selftest: the note (default $UPDATE_STATE_DIR/harness-units)
#   HARNESS_BRIDGE_RESTART selftest: the stand-in of the bridge rule, which records "<verb> <unit> as <owner>"

harness_units_dir() { harness_bridge_knob HARNESS_UNITS_SHARE "${SHARE_DIR:-/opt/yantrik/share}/harnesses"; }
harness_units_note_file() { harness_bridge_knob HARNESS_UNITS_STATE "$UPDATE_STATE_DIR/harness-units"; }

# The units that ship, one "<id> <unit>" a line.
harness_units_list() {
  local f d; d="$(harness_units_dir)"
  for f in "$d"/*/yantrik-*.service; do
    [ -f "$f" ] || continue
    printf '%s %s\n' "$(basename "$(dirname "$f")")" "$(basename "$f")"
  done
}

# The pids of a user unit, from systemd's own record: the process's cgroup ends in the unit and
# sits in the desktop account's manager. By pid, never by name.
harness_units_pids() { # <unit>
  local owner uid d path out=""
  owner="$(harness_bridge_owner)" && uid="$(id -u "$owner" 2>/dev/null)" || return 0
  for d in "$(harness_bridge_proc)"/[0-9]*; do
    path="$(sed -n 's#^0::/\(.*\)$#\1#p' "$d/cgroup" 2>/dev/null)"
    case "/$path" in */user@"$uid".service/*) ;; *) continue ;; esac
    [ "${path##*/}" = "$1" ] && out="$out ${d##*/}"
  done
  printf '%s' "${out# }"
}

# The units running code older than what is installed, one "<id> <unit> <pid>" a line (the oldest pid).
harness_units_stale() {
  local id unit pids pid started oldest code
  while read -r id unit; do
    [ -n "$id" ] || continue
    pids="$(harness_units_pids "$unit")"; [ -n "$pids" ] || continue
    oldest=""; started=""
    for pid in $pids; do
      started="$(stat -c %Y "$(harness_bridge_proc)/$pid" 2>/dev/null || echo 0)"
      if [ -z "$oldest" ] || [ "$started" -lt "$oldest" ]; then oldest="$started"; fi
    done
    code="$(find "$(harness_units_dir)/$id" "$(harness_units_dir)/lib" -printf '%C@\n' 2>/dev/null | sort -n | tail -n1 | cut -d. -f1)"
    [ -n "$code" ] && [ "$oldest" -lt "$code" ] && printf '%s %s %s\n' "$id" "$unit" "${pids%% *}"
  done < <(harness_units_list)
  return 0
}

# Reload or restart a user unit as the person. In selftest <HARNESS_BRIDGE_RESTART> stands in, as
# for the bridge: `fail`, or a file the ask is appended to.
harness_units_signal() { # <reload|restart> <unit>
  if selftest_standins && [ -n "${HARNESS_BRIDGE_RESTART:-}" ]; then
    [ "$HARNESS_BRIDGE_RESTART" = fail ] && return 1
    printf '%s %s as %s\n' "$1" "$2" "$(harness_bridge_owner)" >> "$HARNESS_BRIDGE_RESTART"
    return 0
  fi
  harness_bridge_userctl "$1" "$2"
}
harness_units_can_reload() { # <unit>
  if selftest_standins && [ -n "${HARNESS_BRIDGE_RESTART:-}" ]; then
    [ "$(harness_bridge_knob HARNESS_BRIDGE_CANRELOAD no)" = yes ]; return
  fi
  [ "$(harness_bridge_userctl show -p CanReload --value "$1" 2>/dev/null)" = yes ]
}

# The bridge's own verdict, retry and state (yantrik-harness-bridge.sh) call harness_units_verdict
# and harness_units_retry, so `status`, `status --porcelain`, the update's closing line and `check`
# carry the harnesses with no second path through the updater.

# The note: one line a unit, "<unit> <reason>". Root's, like the bridge's, so a `check` run as the
# person may restart a unit and not be able to put its line away; the verdict is the live check.
harness_units_note_get() { sed -n "s/^$1 //p" "$(harness_units_note_file)" 2>/dev/null | sed -n '1p'; }
harness_units_note_put() { # <unit> <reason>, or <unit> alone to put its line away
  local f d t; f="$(harness_units_note_file)"; d="$(dirname "$f")"
  [ -d "$d" ] || install -d -m 0755 "$d" 2>/dev/null
  t="$(grep -v "^$1 " "$f" 2>/dev/null || true)"
  [ -z "${2:-}" ] || t="$t${t:+
}$1 $2"
  if [ -z "$t" ]; then rm -f "$f" 2>/dev/null; else printf '%s\n' "$t" > "$f" 2>/dev/null && chmod 0644 "$f" 2>/dev/null; fi
  return 0
}

# Wait for <id> to be idle, as harness_bridge_wait_idle does for Hermes (same bounds); on failure the
# reason is in $HARNESS_UNITS_IDLE.
harness_units_wait_idle() { # <id> <bound>
  local waited=0 bound="$2" person limit answer
  person="$(harness_bridge_seconds HARNESS_BRIDGE_PERSON_SECONDS 30)"
  while :; do
    answer="$(harness_bridge_idle "$1")"
    [ "$answer" = idle ] && return 0
    HARNESS_UNITS_IDLE="$answer"
    case "$answer" in unknown|orphaned) return 1 ;; esac
    limit="$bound"
    [ "$answer" = waiting ] && [ "$person" -lt "$bound" ] && limit="$person"
    if [ "$waited" = 0 ] && [ "$limit" -gt 0 ]; then
      say "Waiting for $1 to finish what it is doing before giving it the new code"
      info "up to $(harness_bridge_span "$limit"); nothing else is touched"
    fi
    [ "$waited" -ge "$limit" ] && return 1
    sleep 10
    waited=$((waited + 10))
  done
}

# Give one harness its new code; says what came of it and records the reason when it could not.
harness_units_one() { # <id> <unit> <bound>
  local id="$1" unit="$2" bound="$3" why=""
  if harness_units_can_reload "$unit"; then
    if harness_units_signal reload "$unit"; then
      [ -z "$(harness_units_stale | grep -F " $unit ")" ] \
        || why="asked $unit to reload; it restarts once it finishes what it is doing, and its code is not new yet"
    else why="asking $unit to reload failed"; fi
  elif ! harness_units_wait_idle "$id" "$bound"; then
    case "${HARNESS_UNITS_IDLE:-busy}" in
      orphaned) why="$id was still working on a turn the update cut off, so it was left running" ;;
      unknown) why="could not tell whether $id was busy (the shell's run store was not readable)" ;;
      waiting) why="$id is waiting for the person to answer, so it was left running" ;;
      *) why="$id was still working after $(harness_bridge_span "$bound")" ;;
    esac
  elif [ "$(harness_bridge_idle "$id")" != idle ]; then
    why="$id started a turn just as the restart was due, so it was left running"
  elif ! harness_units_signal restart "$unit"; then
    why="restarting $unit failed"
  fi
  if [ -z "$why" ]; then
    harness_units_note_put "$unit"; say "Gave $id its new code: $unit restarted"; return 0
  fi
  harness_units_note_put "$unit" "$why (as of $(date -u '+%Y-%m-%d %H:%MZ'))"
  warn "$id is still on its old code: $why -- restart $unit when it is idle"
}

# The update's call, after an apply: every harness unit running older code than is installed. Never
# fails (the update has been applied); once a run, since an apply reports twice. <bound> as for the
# bridge (0 from `check`: never waits).
harness_units_update() { # [bound]
  [ -z "${HARNESS_UNITS_TRIED:-}" ] || return 0
  local id unit pid bound="${1:-}"
  case "$bound" in ''|*[!0-9]*) bound="$(harness_bridge_seconds HARNESS_BRIDGE_WAIT_SECONDS 300)" ;; esac
  while read -r id unit pid; do
    [ -n "$id" ] || continue
    HARNESS_UNITS_TRIED=1
    harness_units_one "$id" "$unit" "$bound"
  done < <(harness_units_stale)
  return 0
}

# A later try from `check`, never waiting. A reload is asked for once in half an hour (its note,
# written when asked, says when), as the bridge's is: the unit is draining and asking again cannot hurry it.
harness_units_retry() {
  local id unit pid f; f="$(harness_units_note_file)"
  [ -n "$(harness_units_stale)" ] || return 0
  if [ -n "$(find "$f" -mmin -30 2>/dev/null)" ] && grep -q ' asked ' "$f" 2>/dev/null; then return 0; fi
  HARNESS_UNITS_TRIED=""
  harness_units_update 0
}

# Silent and 0 when every running harness has the installed code; else the reasons, 1.
harness_units_verdict() {
  local id unit pid why out=""
  while read -r id unit pid; do
    [ -n "$id" ] || continue
    why="$(harness_units_note_get "$unit")"
    [ -n "$why" ] || why="no restart of $unit has happened since the last update"
    out="$out${out:+; }$id is on old code: $why (pid $pid)"
  done < <(harness_units_stale)
  [ -z "$out" ] && return 0
  printf '%s' "$out"; return 1
}
harness_units_state() {
  local why; why="$(harness_units_verdict)" && { printf 'current'; return 0; }
  printf 'not done: %s' "$why"
}
