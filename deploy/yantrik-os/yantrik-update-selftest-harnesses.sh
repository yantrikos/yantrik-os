#!/bin/bash
# Sourced by `yantrik-update selftest` (from yantrik-update-selftest-bridge.sh), never run, and not
# installed: the selftest for the harness units an update restarts (#1394), in the style of the
# bridge's. What stands in is the outside world only: /proc's cgroup and start time for which unit
# runs and since when, the shell's run store (a real sqlite db, the real query), and systemctl. The
# rules under test -- found by cgroup and not by name, restarted only when its code is newer than
# the process, left alone when a turn is in flight, an update that cannot restart still applies and
# says why -- are yantrik-harness-units.sh's own.
#
# Runs inside cmd_selftest and uses its `is`, `root`; hb_db is the bridge selftest's (same store).

selftest_harness_units() {
  local hu="$root/hunits" hu_owner hu_uid pid_pi=4301 pid_ds=4302 pid_lookalike=4303 pid_other=4304
  hu_owner="$(id -un)"; hu_uid="$(id -u)"
  mkdir -p "$hu/proc/$pid_pi" "$hu/proc/$pid_ds" "$hu/proc/$pid_lookalike" "$hu/proc/$pid_other" \
           "$hu/share/harnesses/pi" "$hu/share/harnesses/deepseek" "$hu/share/harnesses/lib"
  printf 'unit\n' | tee "$hu/share/harnesses/pi/yantrik-pi.service" "$hu/share/harnesses/deepseek/yantrik-deepseek.service" >/dev/null
  printf 'code\n' > "$hu/share/harnesses/pi/yantrik_pi.py"; printf 'code\n' > "$hu/share/harnesses/lib/shared.py"
  printf '0::/user.slice/user-%s.slice/user@%s.service/app.slice/yantrik-pi.service\n' "$hu_uid" "$hu_uid" > "$hu/proc/$pid_pi/cgroup"
  printf '0::/user.slice/user-%s.slice/user@%s.service/app.slice/yantrik-deepseek.service\n' "$hu_uid" "$hu_uid" > "$hu/proc/$pid_ds/cgroup"
  # Named like Pi (its command line says so) but in someone else's session: not the unit's.
  printf '0::/user.slice/user-0.slice/user@0.service/app.slice/yantrik-pi.service\n' > "$hu/proc/$pid_lookalike/cgroup"
  printf 'python3\0/opt/yantrik/share/harnesses/pi/yantrik_pi.py\0' > "$hu/proc/$pid_lookalike/cmdline"
  printf '0::/user.slice/user-%s.slice/user@%s.service/app.slice/yantrik-shell.service\n' "$hu_uid" "$hu_uid" > "$hu/proc/$pid_other/cgroup"
  printf 'python3\0/opt/yantrik/share/harnesses/pi/yantrik_pi.py\0' > "$hu/proc/$pid_other/cmdline"

  # hu_run <started: old|new> <runs...> [HU_* knobs in the environment]: an update's step for the
  # harnesses, what it asked systemd (read from the stand-in) and the note it left.
  # `old`: the unit started an hour before the files were installed. `new`: it started after them.
  hu_run() {
    ( unset HARNESS_UNITS_TRIED
      rm -f "$hu/restart" "$hu/note"
      export YANTRIK_SELFTEST=1 YANTRIK_PROC="$hu/proc" HARNESS_BRIDGE_RUNS_DB="$root/bridge/runs.db" HARNESS_UNITS_SHARE="$hu/share/harnesses" \
             HARNESS_UNITS_STATE="$hu/note" HARNESS_BRIDGE_WAIT_SECONDS="${HU_WAIT:-0}" HARNESS_BRIDGE_PERSON_SECONDS=0 \
             HARNESS_BRIDGE_RESTART="${HU_RESTART:-$hu/restart}"
      [ -z "${HU_RELOAD:-}" ] || export HARNESS_BRIDGE_CANRELOAD=yes
      [ "$1" = old ] && when='-1 hour' || when='+1 hour'
      touch -d "$when" "$hu/proc/$pid_pi" "$hu/proc/$pid_ds" "$hu/proc/$pid_lookalike" "$hu/proc/$pid_other"
      [ -z "${HU_NO_DS:-}" ] || rm -f "$hu/proc/$pid_ds/cgroup"
      hb_db "${@:2}" 2>/dev/null
      "${HU_STEP:-harness_units_update}" >"$hu/out" 2>&1
      printf 'did=%s|%s\n' "$(tr '\n' ',' < "$hu/restart" 2>/dev/null)" "$( [ -f "$hu/note" ] && cut -c1-16 "$hu/note" | tr '\n' ',' || echo none)"
    )
  }
  hu_status() { # the verdict of that desktop, through the bridge's own entry point
    ( export YANTRIK_SELFTEST=1 YANTRIK_PROC="$hu/proc" HARNESS_UNITS_SHARE="$hu/share/harnesses" HARNESS_UNITS_STATE="$hu/note"
      stale_processes() { :; }
      harness_bridge_state | sed 's/ (as of [^)]*Z)//' )
  }
  local me="as $hu_owner"

  is "harness units: Pi started before the files were installed, idle: restarted as the person; DeepSeek, not running, left" \
    "did=restart yantrik-pi.service $me,|none" "$(HU_NO_DS=1 hu_run old pi=done)"
  is "harness units: ... and says so" "yes" \
    "$( hu_run old pi=done >/dev/null; grep -q 'Gave pi its new code' "$hu/out" && echo yes || echo no )"
  is "harness units: a unit running code newer than its files (or the same) is not touched" "did=|none" \
    "$(hu_run new pi=done)"
  is "harness units: ... and status calls it current" "current" "$(hu_run new pi=done >/dev/null; hu_status)"
  is "harness units: a turn in flight: left running, and a note left" "did=|yantrik-pi.servi," \
    "$(hu_run old pi=running)"
  is "harness units: ... said as still working, with what to do" "yes" \
    "$( hu_run old pi=running >/dev/null; grep -q 'pi is still on its old code: pi was still working after 0 seconds -- restart yantrik-pi.service' "$hu/out" && echo yes || echo no )"
  is "harness units: ... and status says not done, with the reason and the pid" \
    "not done: pi is on old code: pi was still working after 0 seconds (pid $pid_pi)" \
    "$( hu_run old pi=running >/dev/null; hu_status )"
  is "harness units: a run waiting on the person is not killed either" "did=|yantrik-pi.servi," "$(hu_run old pi=waiting-on-person)"
  is "harness units: a run the shell just orphaned is a turn in flight" "did=|yantrik-pi.servi," "$(hu_run old pi=orphaned:5)"
  is "harness units: Hermes's open run is not Pi's turn, and Pi's open run is not Hermes's" \
    "did=restart yantrik-pi.service $me,|none" "$(hu_run old hermes=running pi=done)"
  is "harness units: no run store at all is not idle" "did=|yantrik-pi.servi," \
    "$( ( export YANTRIK_SELFTEST=1 YANTRIK_PROC="$hu/proc" HARNESS_BRIDGE_RUNS_DB="$hu/nope.db" HARNESS_UNITS_SHARE="$hu/share/harnesses" \
                 HARNESS_UNITS_STATE="$hu/note" HARNESS_BRIDGE_RESTART="$hu/restart" HARNESS_BRIDGE_WAIT_SECONDS=0; rm -f "$hu/restart" "$hu/note"
          harness_units_update >/dev/null 2>&1; printf 'did=%s|%s\n' "$(tr '\n' ',' < "$hu/restart" 2>/dev/null)" "$(cut -c1-16 "$hu/note" | tr '\n' ',')" ) )"
  is "harness units: a unit that can reload is reloaded, with a turn in flight, and not restarted" \
    "did=reload yantrik-pi.service $me,|yantrik-pi.servi," "$(HU_RELOAD=1 hu_run old pi=running)"
  is "harness units: a restart that fails is said as that, and the update's work stands (it returns)" \
    "did=|yantrik-pi.servi," "$(HU_RESTART=fail hu_run old pi=done)"
  is "harness units: ... with the reason in status" \
    "not done: pi is on old code: restarting yantrik-pi.service failed (pid $pid_pi)" \
    "$( HU_RESTART=fail hu_run old pi=done >/dev/null; hu_status )"
  is "harness units: the lookalike in another session and a bridge-less process are never signalled" "did=restart yantrik-pi.service $me,|none" \
    "$(hu_run old pi=done)"

  # The update's own step and `check`'s later try.
  hu_report() { restart_stale_services() { :; }; stale_processes() { :; }; report_stale; }
  is "harness units: the update's report step reaches them (report_stale)" "did=restart yantrik-pi.service $me,|none" \
    "$( HU_STEP=hu_report hu_run old pi=done )"
  hu_retry_idle() { printf 'yantrik-pi.service earlier failure\n' > "$hu/note"; harness_bridge_retry_hermes() { :; }; harness_bridge_retry; }
  is "harness units: a later try (check) restarts once it is idle, and puts the note away" \
    "did=restart yantrik-pi.service $me,|none" "$(HU_STEP=hu_retry_idle hu_run old pi=done)"
  is "harness units: ... and leaves a busy one, the note still there" "did=|yantrik-pi.servi," "$(HU_STEP=hu_retry_idle hu_run old pi=running)"

  # The updater and the helper travel together.
  is "harness units: the updater accepts the helper as its own, and the bundle ships it" "yes" \
    "$( grep -q 'yantrik-harness-units.sh' "$SELF_PATH" && grep -q 'yantrik-harness-units.sh' "$(dirname "$SELF_PATH")/build-release.sh" && echo yes || echo no )"
  rm -f "$hu/note" "$hu/restart" "$hu/out"
}
