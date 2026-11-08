#!/bin/bash
# Sourced by `yantrik-update selftest`, never run, and not installed: the selftest checks for the
# mind's bridge (#1102), kept out of yantrik-update so that the updater does not grow by them.
#
# Every check here runs the behaviour and reads what it did. What stands in is only the outside
# world: /proc's cgroup for which unit holds a pid, the shell's run store (a real sqlite db, the
# real query), systemctl, and the shell answering its socket. The rules under test -- pid not name,
# idle before restarting and again right before, never the Mind's unit, a person being waited on only
# briefly, the update ending "not done" with a reason, and the environment steering nothing outside
# selftest -- are the helper's and the updater's own.
#
# Runs inside cmd_selftest and uses its `is`, `fails`, `root` and `me`.

selftest_bridge() {
  local hb="$root/bridge" hb_pid=4242 hb_owner hb_uid; hb_owner="$(id -un)"; hb_uid="$(id -u)"
  mkdir -p "$hb/proc/$hb_pid" "$hb/proc/5150" "$hb/proc/6161" "$hb/rt/yantrik" "$hb/bin" "$hb/backups/b1"
  # $hb_uid is the uid, as systemd spells it in a user's own slice. The unit is the last element of the path, in the owner's manager
  # is read -- the unit a process runs in -- but a fixture should look like the real thing.
  printf '0::/user.slice/user-%s.slice/user@%s.service/app.slice/hermes-gateway.service\n' "$hb_uid" "$hb_uid" > "$hb/proc/$hb_pid/cgroup"
  # The Mind's bridge, in its own system unit: of this restart's making, never.
  printf '0::/system.slice/yantrik-mind.service\n' > "$hb/proc/5150/cgroup"
  printf '0::/user.slice/user-%s.slice/user@%s.service/app.slice/yantrik-shell.service\n' "$hb_uid" "$hb_uid" > "$hb/proc/6161/cgroup"
  # A shell that answers: a socket that exists and a `yos` that says yes.
  python3 -I -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' "$hb/rt/yantrik/app-shell.sock" 2>/dev/null
  printf '#!/bin/sh\nexit 0\n' > "$hb/bin/yos"; chmod +x "$hb/bin/yos"

  hb_db() { # hb_db <harness=state ...>: the shell's run store, its own schema
    rm -f "$hb/runs.db"
    python3 -I - "$hb/runs.db" "$@" <<'PY'
import sqlite3, sys, time
c = sqlite3.connect(sys.argv[1])
c.execute("create table runs (run_id integer primary key, harness text not null,"
          " conversation text not null, owner text not null, state text not null,"
          " started_at integer not null, updated_at integer not null)")
for spec in sys.argv[2:]:
    harness, _, state = spec.partition("=")
    # state "orphaned:N" is a run the shell orphaned N seconds ago (what orphan_unfinished leaves)
    state, _, age = state.partition(":")
    at = int(time.time() * 1000) - int(age) * 1000 if age else 1
    c.execute("insert into runs (harness, conversation, owner, state, started_at, updated_at)"
              " values (?,?,?,?,?,?)", (harness, "conv", "owner", state, 1, at))
c.commit()
PY
  }
  # The environment of a fake desktop: what the helper may read in selftest, and nothing a bound of
  # 0 would not end at once. HB_WAIT / HB_PERSON are the two bounds, in seconds.
  hb_env() {
    export YANTRIK_SELFTEST=1 YANTRIK_PROC="$hb/proc" HARNESS_BRIDGE_RUNS_DB="$hb/runs.db" \
           HARNESS_BRIDGE_STATE="$hb/note" HERMES_GATEWAY_UNIT=hermes-gateway.service \
           HARNESS_BRIDGE_WAIT_SECONDS="${HB_WAIT:-0}" HARNESS_BRIDGE_PERSON_SECONDS="${HB_PERSON:-0}"
  }
  hb_exec() { # hb_exec <restart stand-in: a file, or fail> <stale list> <runs...>: that update, in its words
    ( unset HARNESS_BRIDGE_TRIED
      # The stand-in and the note are emptied so that what a run did is that run's doing and not
      # the fixtures'; the pair read across runs is written as a pair, below.
      [ "$1" = fail ] || rm -f "$1"; rm -f "$hb/note"
      hb_db "${@:3}" 2>/dev/null
      hb_env; export HARNESS_BRIDGE_RESTART="$1"
      [ -z "${HB_FLIP:-}" ] || harness_bridge_idle() { # idle once, then a turn begins
        if [ -e "$hb/flipped" ]; then printf busy; else : > "$hb/flipped"; printf idle; fi; }
      rm -f "$hb/flipped"
      # HB_RELOAD=1: a unit that can reload. HB_FRESH=1: and the bridge is new once it has been asked.
      [ -z "${HB_RELOAD:-}" ] || { export HARNESS_BRIDGE_CANRELOAD=yes; hb_rf="$1"; hb_old="$2"
        stale_processes() { if [ -n "${HB_FRESH:-}" ] && [ -s "$hb_rf" ]; then :; else printf '%s' "$hb_old"; fi; }; }
      harness_bridge_update "$2" 2>&1
    )
  }
  # What a run did, on one line: the account of the restart (read from the stand-in, not trusted
  # from the output) and the first five letters of the note left behind; `none` when never written.
  hb_did() {
    ( hb_exec "$@" >/dev/null
      printf 'did=%s|%s\n' "$(cat "$1" 2>/dev/null)" \
             "$( [ -f "$hb/note" ] && cut -c1-5 "$hb/note" || echo none)"
    )
  }
  hb_said() { # hb_said <restart> <stale> <the words> <runs...>: yes or no
    hb_exec "$1" "$2" "${@:4}" | grep -qF -- "$3" && printf yes || printf no
  }
  # `status`' line about that desktop, given the processes it finds old. The clock is cut out of the
  # note: what is asked is the reason and the pid.
  hb_ask() {
    ( unset HARNESS_BRIDGE_TRIED; hb_env
      hb_found="$1"; stale_processes() { printf '%s' "$hb_found"; }
      harness_bridge_state | sed 's/ (as of [^)]*Z)//'
    )
  }
  hb_note() { [ -f "$hb/note" ] && printf there || printf gone; }
  local hb_stale="yos-mcp(pid-$hb_pid)" hb_sh="yantrik-shell(pid-6161)"

  hb_db hermes=done pi=done
  is "bridge: no bridge of the gateway's: nothing to restart, nothing said" \
    "did=|none" "$(hb_did "$hb/restart" "$hb_sh")"
  is "bridge: ... and status calls it current" "current" "$(hb_ask "$hb_sh")"
  is "bridge: no yos-mcp running: nothing to restart, nothing said" "did=|none" "$(hb_did "$hb/restart" "")"
  is "bridge: ... and nothing of the bridge is said" "no" "$(hb_said "$hb/restart" "" bridge)"

  is "bridge: idle Hermes (a closed run of hers, another harness's open): restarted, as the person" \
    "did=restart hermes-gateway.service as $hb_owner|none" \
    "$(hb_did "$hb/restart" "$hb_stale" hermes=done pi=running)"
  is "bridge: ... and says so in the update's own words" "yes" \
    "$(hb_said "$hb/restart" "$hb_stale" "Gave the mind the new bridge" hermes=done)"
  is "bridge: a turn in flight: left alone, and a note left" "did=|Herme" \
    "$(hb_did "$hb/restart" "$hb_stale" hermes=running)"
  is "bridge: ... with the reason and what to do about it in the output" "yes" \
    "$(hb_said "$hb/restart" "$hb_stale" "restart hermes-gateway.service when Hermes is idle" hermes=running)"
  is "bridge: ... and status reports it not done, with that reason and the pid" \
    "not done: Hermes is on the old bridge: Hermes was still working after 0 seconds (pid $hb_pid)" \
    "$( hb_did "$hb/restart" "$hb_stale" hermes=running >/dev/null; hb_ask "$hb_stale" )"

  # A run waiting on the person is not a turn that finishes by itself: waited on briefly, and it is
  # said plainly that it is the person being waited on. The long bound is 300s here; if it were
  # used, this check would take five minutes instead of returning at once.
  is "bridge: a run waiting on the person: left alone, after the short bound and not the long one" \
    "did=|Herme" "$(HB_WAIT=300 HB_PERSON=0 hb_did "$hb/restart" "$hb_stale" hermes=waiting-on-person)"
  is "bridge: ... said as waiting for the person to answer" "yes" \
    "$(HB_WAIT=300 HB_PERSON=0 hb_said "$hb/restart" "$hb_stale" "waiting for the person to answer" hermes=waiting-on-person)"
  is "bridge: ... and status carries that reason" \
    "not done: Hermes is on the old bridge: Hermes is waiting for the person to answer, so it was left running (pid $hb_pid)" \
    "$( HB_WAIT=300 hb_did "$hb/restart" "$hb_stale" hermes=waiting-on-person >/dev/null; hb_ask "$hb_stale" )"
  is "bridge: a turn in flight beside a waiting one is a turn in flight" "yes" \
    "$(hb_said "$hb/restart" "$hb_stale" "still working" hermes=waiting-on-person hermes=running)"

  # An update restarts the shell, which orphans every unfinished run; Hermes is still working on it.
  is "bridge: a run the shell just orphaned is a turn in flight: left alone, a note left" "did=|Herme" \
    "$(hb_did "$hb/restart" "$hb_stale" hermes=orphaned:5)"
  is "bridge: ... said as still working" "yes" \
    "$(hb_said "$hb/restart" "$hb_stale" "still working" hermes=orphaned:5)"
  is "bridge: a run orphaned long ago does not hold the restart" \
    "did=restart hermes-gateway.service as $hb_owner|none" "$(hb_did "$hb/restart" "$hb_stale" hermes=orphaned:3600)"
  is "bridge: another harness's recent orphan does not hold it either" \
    "did=restart hermes-gateway.service as $hb_owner|none" "$(hb_did "$hb/restart" "$hb_stale" pi=orphaned:5)"

  # A unit that can reload: Hermes drains its own work first, so nothing is waited on or looked up;
  # the update asks, then confirms the bridge is new.
  is "bridge: a unit that can reload is reloaded, not restarted, even with a turn in flight; note put away"     "did=reload hermes-gateway.service as $hb_owner|none" "$(HB_RELOAD=1 HB_FRESH=1 hb_did "$hb/restart" "$hb_stale" hermes=running)"
  is "bridge: ... and says the mind has the new bridge" "yes"     "$(HB_RELOAD=1 HB_FRESH=1 hb_said "$hb/restart" "$hb_stale" "hermes-gateway.service reloaded" hermes=running)"
  is "bridge: a reload after which the bridge is still old is not done, with a note" "did=reload hermes-gateway.service as $hb_owner|asked"     "$(HB_RELOAD=1 hb_did "$hb/restart" "$hb_stale" hermes=done)"
  is "bridge: ... said as Hermes restarting once it has finished" "yes"     "$(HB_RELOAD=1 hb_said "$hb/restart" "$hb_stale" "restarts once it finishes what it is doing" hermes=done)"
  is "bridge: a reload that fails is said as that" "yes"     "$(HB_RELOAD=1 hb_said fail "$hb_stale" "asking hermes-gateway.service to reload failed" hermes=done)"
  # An orphaned row is final: the long wait could only end in the note, so it is not taken.
  is "bridge: only an orphaned run holds it: no wait (300s bound would stall), and it is said" "did=|Herme"     "$(HB_WAIT=300 hb_did "$hb/restart" "$hb_stale" hermes=orphaned:5)"

  # Only the desktop account's own manager holds the gateway this restart reaches.
  mkdir -p "$hb/proc/7001" "$hb/proc/7002"
  printf '0::/system.slice/hermes-gateway.service\n' > "$hb/proc/7001/cgroup"
  printf '0::/user.slice/user-0.slice/user@0.service/app.slice/hermes-gateway.service\n' > "$hb/proc/7002/cgroup"
  is "bridge: a same-named unit in the system slice or in another user's manager is not the gateway" "did=|none" \
    "$(hb_did "$hb/restart" "yos-mcp(pid-7001) yos-mcp(pid-7002)" hermes=done)"

  # With the helper missing the updater cannot say the bridge is current.
  is "bridge: without the helper, the done-check says why it cannot tell" "1|yes" \
    "$( ( sed -n '/^  harness_bridge_verdict() {/p' "$SELF_PATH" > "$hb/fb.sh"; . "$hb/fb.sh"
          o="$(harness_bridge_verdict)"; r=$?; case "$o" in *"is missing"*) w=yes ;; *) w=no ;; esac
          echo "$r|$w" ) )"

  # Idle a moment ago, a turn now: the restart must not kill it.
  is "bridge: a turn that begins between the idle answer and the restart is left alone" "did=|Herme" \
    "$(HB_FLIP=1 hb_did "$hb/restart" "$hb_stale" hermes=done)"
  is "bridge: ... said as such" "yes" \
    "$(HB_FLIP=1 hb_said "$hb/restart" "$hb_stale" "started a turn just as the restart was due" hermes=done)"

  is "bridge: the Mind's bridge is never this restart's, and no note is left for it" \
    "did=|none" "$(hb_did "$hb/restart" "yos-mcp(pid-5150)")"
  is "bridge: a pid systemd does not place in the gateway is not the gateway's bridge" \
    "did=|none" "$(hb_did "$hb/restart" "yos-mcp(pid-9999)")"
  is "bridge: no run store at all is not idle" "did=|could" \
    "$( ( hb_env; export HARNESS_BRIDGE_RUNS_DB="$hb/nope.db" HARNESS_BRIDGE_RESTART="$hb/restart"; rm -f "$hb/restart" "$hb/note"
         unset HARNESS_BRIDGE_TRIED; harness_bridge_update "$hb_stale" >/dev/null 2>&1
         printf 'did=%s|%s\n' "$(cat "$hb/restart" 2>/dev/null)" "$(cut -c1-5 "$hb/note" 2>/dev/null)" ) )"

  # A restart that does not happen: the update's work stands (it returns), the reason is said as the
  # failed restart it was, and it is left for `status`.
  is "bridge: a restart that fails leaves a reason, and the update's work standing" \
    "did=|resta" "$(hb_did fail "$hb_stale" hermes=done)"
  is "bridge: ... said as the failed restart, not as Hermes being busy" "yes" \
    "$(hb_said fail "$hb_stale" "restarting hermes-gateway.service failed" hermes=done)"
  is "bridge: ... and status carries that reason, and the pid" \
    "not done: Hermes is on the old bridge: restarting hermes-gateway.service failed (pid $hb_pid)" \
    "$( hb_did fail "$hb_stale" hermes=done >/dev/null; hb_ask "$hb_stale" )"

  # The note's life is across runs: left by the update that could not restart, put away by the next
  # one that could, and by a `status` that finds the gateway current.
  is "bridge: the note a failed update leaves is put away by the next update that could restart" \
    "there then gone" \
    "$( hb_exec fail "$hb_stale" hermes=done >/dev/null 2>&1
       printf '%s then ' "$(hb_note)"
       hb_exec "$hb/restart2" "$hb_stale" hermes=done >/dev/null 2>&1
       printf '%s' "$(hb_note)" )"
  is "bridge: a status finding the gateway current puts the note away too" "current then gone" \
    "$( hb_exec fail "$hb_stale" hermes=done >/dev/null 2>&1
       printf '%s then ' "$(hb_ask "$hb_sh")"
       printf '%s' "$(hb_note)" )"
  is "bridge: no note and the bridge still old: status is still not done, in its own words" \
    "not done: Hermes is on the old bridge: no restart of hermes-gateway.service has happened since the last update (pid $hb_pid)" \
    "$(hb_ask "$hb_stale")"

  # The later try: `check` runs it, without waiting, only when an update left the note.
  hb_retry() { # hb_retry <restart stand-in> <runs...>: the retry of a machine that has the note
    ( unset HARNESS_BRIDGE_TRIED; hb_env; rm -f "$1" "$hb/note"; hb_db "${@:2}" 2>/dev/null
      export HARNESS_BRIDGE_RESTART="$1"
      hb_found="$hb_stale"; stale_processes() { printf '%s' "$hb_found"; }
      [ -z "${HB_NONOTE:-}" ] && printf 'earlier failure\n' > "$hb/note"
      harness_bridge_retry >/dev/null 2>&1
      printf 'did=%s|%s\n' "$(cat "$1" 2>/dev/null)" "$(hb_note)" )
  }
  is "bridge: the retry restarts once Hermes is idle, and puts the note away" \
    "did=restart hermes-gateway.service as $hb_owner|gone" "$(hb_retry "$hb/restart" hermes=done)"
  is "bridge: ... leaves a busy Hermes alone, with the note still there" "did=|there" \
    "$(hb_retry "$hb/restart" hermes=running)"
  is "bridge: ... and does nothing on a machine no update left a note on" "did=|gone" \
    "$(HB_NONOTE=1 hb_retry "$hb/restart" hermes=done)"

  # The environment steers nothing outside selftest: the updater is root's. With every stand-in
  # named but selftest not on, none is read.
  is "bridge: outside selftest, the stand-ins are not read" "did=|none" \
    "$( ( unset HARNESS_BRIDGE_TRIED YANTRIK_SELFTEST; hb_db hermes=done 2>/dev/null; rm -f "$hb/restart" "$hb/note"
          export YANTRIK_PROC="$hb/proc" HARNESS_BRIDGE_RUNS_DB="$hb/runs.db" HARNESS_BRIDGE_STATE="$hb/note" \
                 HERMES_GATEWAY_UNIT=hermes-gateway.service HARNESS_BRIDGE_RESTART="$hb/restart"
          harness_bridge_update "$hb_stale" >/dev/null 2>&1
          printf 'did=%s|%s\n' "$(cat "$hb/restart" 2>/dev/null)" "$( [ -f "$hb/note" ] && echo there || echo none)" ) )"
  is "bridge: ... the note stays in the updater's state directory" "$UPDATE_STATE_DIR/hermes-bridge" \
    "$( ( unset YANTRIK_SELFTEST; export HARNESS_BRIDGE_STATE=/tmp/elsewhere; harness_bridge_note_file ) )"
  is "bridge: ... and the unit and /proc are the real ones" "hermes-gateway.service /proc" \
    "$( ( unset YANTRIK_SELFTEST; export HERMES_GATEWAY_UNIT=other.service YANTRIK_PROC=/tmp/x
          printf '%s %s' "$(harness_bridge_unit)" "$(harness_bridge_proc)" ) )"

  # The update's own done-check and `status --porcelain`, through the real functions. A shell that
  # answers on a fake socket takes wait_for_shell (what apply and rollback both end in) to its
  # closing line; restart_and_verify and cmd_rollback are run whole, with only systemd stood in for.
  hb_tail() { # hb_tail <stale list> <runs...>: the updater's closing line for that desktop, and its exit
    ( unset HARNESS_BRIDGE_TRIED; hb_env; rm -f "$hb/note"; hb_db "${@:2}" 2>/dev/null
      export HARNESS_BRIDGE_RESTART=fail
      hb_found="$1"; stale_processes() { printf '%s' "$hb_found"; }
      restart_stale_services() { :; }
      BIN_DIR="$hb/bin"; c_grn=""; c_red=""; c_off=""
      wait_for_shell "$hb/rt" 2>&1 | grep -E 'update (complete|applied)'; exit "${PIPESTATUS[0]}" )
  }
  is "bridge: the update ends complete when the bridge is current" "healthy - update complete" \
    "$(hb_tail "$hb_sh" hermes=done)"
  is "bridge: ... and ends not done, with the reason, when it could not be given" \
    "shell healthy - update applied, but NOT done: Hermes is on the old bridge: restarting hermes-gateway.service failed (pid $hb_pid)" \
    "$(hb_tail "$hb_stale" hermes=done | sed 's/ (as of [^)]*Z)//')"
  is "bridge: ... and still applies: the exit is 0" "0" \
    "$( hb_tail "$hb_stale" hermes=running >/dev/null; echo $? )"
  is "bridge: a rollback ends the same way, through restart_and_verify" \
    "shell healthy - update applied, but NOT done: Hermes is on the old bridge: Hermes was still working after 0 seconds (pid $hb_pid)" \
    "$( ( unset HARNESS_BRIDGE_TRIED; hb_env; rm -f "$hb/note"; hb_db hermes=running 2>/dev/null
          export HARNESS_BRIDGE_RESTART="$hb/restart"
          hb_found="$hb_stale"; stale_processes() { printf '%s' "$hb_found"; }
          restart_stale_services() { :; }; elevate() { :; }; restore_backup() { :; }
          refresh_session_chrome() { :; }; stop_mind_view_browsers() { :; }
          session_runtime_dir() { printf '%s' "$hb/rt"; }
          systemctl() { return 0; }; sudo() { shift; "$@"; }
          BIN_DIR="$hb/bin"; BACKUP_DIR="$hb/backups"; UPDATES_DIR="$hb/updates"; c_grn=""; c_red=""; c_off=""
          cmd_rollback 2>&1 | grep -E 'update (complete|applied)' | sed 's/ (as of [^)]*Z)//' ) )"

  hb_porcelain() { # hb_porcelain <stale list> <runs...>: the bridge lines of `status --porcelain`
    ( unset HARNESS_BRIDGE_TRIED; hb_env; rm -f "$hb/note"; hb_db "${@:2}" 2>/dev/null
      hb_found="$1"; stale_processes() { printf '%s' "$hb_found"; }
      BACKUP_DIR="$hb/backups"
      cmd_status --porcelain | sed -n '/^backups=/,$p' | tr '\n' ' ' | sed 's/ $//' )
  }
  is "bridge: status --porcelain keeps its fields and adds bridge=current" "backups=1 bridge=current bridge_reason=" \
    "$(hb_porcelain "$hb_sh" hermes=done)"
  is "bridge: ... and bridge=not_done with the reason when it is" \
    "backups=1 bridge=not_done bridge_reason=Hermes is on the old bridge: no restart of hermes-gateway.service has happened since the last update (pid $hb_pid)" \
    "$(hb_porcelain "$hb_stale" hermes=done)"
  is "bridge: the About screen's reader gets every key it read before" "channel channel_source host scheme conf conf_writable version git" \
    "$( ( hb_env; stale_processes() { :; }; BACKUP_DIR="$hb/backups"; cmd_status --porcelain \
          | sed 's/=.*//' | grep -xE 'channel|channel_source|host|scheme|conf|conf_writable|version|git' | tr '\n' ' ' | sed 's/ $//' ) )"

  # The updater without its helper still runs and says the bridge is not handled: tested by running
  # a copy of it where the helper is not.
  is "bridge: an updater run without the helper still runs, and says the bridge is not handled" "yes" \
    "$( d="$root/nohelper"; mkdir -p "$d"; cp "$SELF_PATH" "$d/yantrik-update"
        out="$(YANTRIK_PREFIX="$root/nohelper-prefix" bash "$d/yantrik-update" status --porcelain 2>&1)"
        case "$out" in *'yantrik-harness-bridge.sh is missing'*'channel='*) echo yes ;; *) echo no ;; esac )"
  rm -f "$hb/note" "$hb/restart" "$hb/restart2"
}
