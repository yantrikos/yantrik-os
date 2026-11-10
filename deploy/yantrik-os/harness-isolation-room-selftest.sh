#!/bin/bash
# Checks for harness-isolation-room.sh (#411): bash deploy/yantrik-os/harness-isolation-room-selftest.sh
#
# The failure path of a harness move, as 520's gate found it (the disk ran out in the middle):
#   1. room is checked BEFORE anything is copied or installed (a fake `df` makes the disk small);
#   2. an unfinished copy is removed, never kept;
#   3. "your own gateway is running again" is said only when it is true;
#   4. a failed move is not started again by an update or a reconcile, and a rolled-back build that
#      cannot do the move does not start one; `harness install` is the retry, and a success clears it.
# No root and no accounts: the person and the mind account are both this user, `systemctl`, `df`,
# `systemd-run` and `getent` are recorders. Fake data only.
set -u
[ "$(command id -u)" != 0 ] || { echo "run as a normal user (the test fakes root itself)"; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/selftest-lib.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

PREFIX="$work/opt"; BIN_DIR="$PREFIX/bin"
say() { :; }; info() { printf '%s\n' "$*" >> "$work/said"; }; warn() { printf '%s\n' "$*" >> "$work/said"; }
fail() { printf '%s\n' "FAIL: $*" >> "$work/said"; exit 1; }
tree_is_roots() { return 0; }
desktop_owner() { command id -un; }
migrate_if_needed() { :; }; elevate() { :; }
reconcile_minds() { :; }; reconcile_mind_egress_units() { :; }; reconcile_mind_units() { :; }
reconcile_egress() { :; }; reconcile_mind_egress() { :; }
mkdir -p "$work/home" "$work/log" "$work/fakebin" "$PREFIX/share/harnesses/lib/install" "$BIN_DIR"
export YANTRIK_SELFTEST=1 HI_UNIT_DIR="$work/units" HI_DOOR="$work/door" HI_BASE="$work/mind/harness" HI_STATE="$work/state" \
  HI_LOCK="$work/lock" HI_PROC="$work/proc" HI_HOLD=0 HI_DF="$work/fakebin/df"
# A disk of whatever is in $work/free (KB).
printf '#!/bin/sh\nprintf "Filesystem 1024-blocks Used Available Capacity Mounted on\\nfake 100000000 1 %%s 1%% /\\n" "$(cat "%s/free")"\n' "$work" > "$work/fakebin/df"
chmod 755 "$work/fakebin/df"
disk() { printf '%s' "$1" > "$work/free"; }
# shellcheck source=deploy/yantrik-os/harness-isolation.sh
. "$here/harness-isolation.sh"
# The person and the mind account are this user; root is claimed by `id -u`.
hi_as_owner() { "$@"; }; hi_as_mind() { "$@"; }
id() { if [ "${1:-}" = -u ] && [ "$#" = 1 ]; then echo 0; else command id "$@"; fi; }
getent() { [ "$1" = passwd ] && printf '%s:x:1:1::%s:/bin/sh\n' "$2" "$work/home"; }
systemd-run() { printf '%s\n' "$*" >> "$work/log/run"; return 0; }
command() { if [ "${1:-}" = -v ] && [ "${2:-}" = systemd-run ]; then return 0; fi; builtin command "$@"; }
hi_ready() { echo ready >> "$work/log/ready"; mkdir -p "$(hi_home hermes)"; }
mkdir -p "$HI_UNIT_DIR"
printf '# %s\n' "$HI_CAPABILITY" > "$PREFIX/share/harnesses/lib/install/hermes.sh"

H="$work/home"
mkdir -p "$H/.hermes/sessions" "$H/.hermes/hermes-agent"
printf 'model: x\n' > "$H/.hermes/config.yaml"
head -c 2097152 /dev/zero > "$H/.hermes/state.db"          # 2 MB that moves
head -c 8388608 /dev/zero > "$H/.hermes/hermes-agent/big"  # 8 MB of code: reinstalled, not copied, not counted
M="$(hi_home hermes)"
said() { cat "$work/said" 2>/dev/null; }
reset() { : > "$work/said"; : > "$work/log/run"; : > "$work/log/ready"; rm -rf "$HI_STATE" "$HI_BASE"; }

echo "== 1. room is checked before anything changes"
kb="$(hi_data_kb hermes "$H")"
is "the data it would copy is measured as the person (2 MB state.db, config; not the 8 MB of code)" "yes" "$([ "$kb" -ge 2048 ] && [ "$kb" -lt 2300 ] && echo yes || echo no)"
reset; disk 1000
( hi_room_check hermes "$H" yes ) >/dev/null 2>&1
is "a disk with under 1 MB free refuses (the headroom is 3 GB by default)" "yes" "$(grep -q 'FAIL: hermes was not moved' "$work/said" && echo yes || echo no)"
has "... says how much it needs" "it needs about" "$(said)"
has "... and how much is free" "0 MB is free" "$(said)"
has "... and that nothing was changed" "Nothing was changed" "$(said)"
has "... and what to run once there is room" "sudo yantrik-update harness install hermes" "$(said)"
no "... and the mind account's directory was not even made" test -e "$M"
has "... the refusal is noted, with its reason" "not enough room" "$(cat "$HI_STATE/hermes.failed")"
is "... the note is small (one line)" "1" "$(wc -l < "$HI_STATE/hermes.failed" | tr -d ' ')"
reset; disk 99999999
( hi_room_check hermes "$H" yes ) >/dev/null 2>&1; is "a roomy disk passes" "0" "$?"
no "... and leaves no note" test -e "$HI_STATE/hermes.failed"
reset; need=$(( $(hi_data_kb hermes "$H") + HI_HEADROOM_KB + HI_RESERVE_KB ))
disk $((need - 1)); ( hi_room_check hermes "$H" yes ) >/dev/null 2>&1; is "one KB short of what is needed refuses" "1" "$?"
disk "$need";       ( hi_room_check hermes "$H" yes ) >/dev/null 2>&1; is "exactly what is needed passes" "0" "$?"
reset; mkdir -p "$HI_BASE/hermes.failed"; head -c 4194304 /dev/zero > "$HI_BASE/hermes.failed/x"
disk $((need - 1024)); ( hi_room_check hermes "$H" yes ) >/dev/null 2>&1
is "a full copy an older build kept counts as room it is about to give back" "0" "$?"
no "... and is freed by the check itself, on every path (not only where the move discards it)" test -e "$HI_BASE/hermes.failed"
reset; mkdir -p "$HI_BASE/hermes.failed/d"; head -c 4194304 /dev/zero > "$HI_BASE/hermes.failed/d/x"; chmod 555 "$HI_BASE/hermes.failed/d"
disk $((need - 1024)); ( hi_room_check hermes "$H" yes ) >/dev/null 2>&1
is "a copy with a read-only directory is freed too" "0" "$?"
no "... really gone" test -e "$HI_BASE/hermes.failed"
reset; mkdir -p "$HI_BASE/hermes.failed"; head -c 4194304 /dev/zero > "$HI_BASE/hermes.failed/x"
( hi_as_mind() { case "$1" in rm) return 1 ;; esac; "$@"; }; disk $((need - 1024)); hi_room_check hermes "$H" yes ) >/dev/null 2>&1
is "a stale copy that cannot be removed is NOT counted as room: the check refuses" "1" "$?"
has "... with the room sentence" "it needs about" "$(said)"
reset; disk 1000; ( hi_room_check hermes "$H" no ) >/dev/null 2>&1
is "a fresh install (no Hermes of the person's) is checked too, for the install headroom" "1" "$?"
echo "-- through hi_install: before hi_ready, copying or installing"
reset; disk 1000
hi_run_installer() { echo installer >> "$work/log/ready"; }
( hi_install hermes ) >/dev/null 2>&1; is "the install refuses on a small disk" "1" "$?"
is "... before hi_ready (the account's directory, the units) ran" "0" "$(wc -c < "$work/log/ready" | tr -d ' ')"
no "... and nothing of the person's was renamed" test -e "$H/.hermes.yantrik-moved"
yes "... and the person's Hermes is as it was" test -f "$H/.hermes/config.yaml"

echo "== 2. an unfinished copy is removed, not kept"
reset; mkdir -p "$M/.hermes/sessions"; head -c 1048576 /dev/zero > "$M/.hermes/state.db"; mkdir -p "$M.failed"; printf x > "$M.failed/old"
hi_discard_unfinished hermes "its installer stopped"
no "no .failed tree is kept" test -e "$M.failed"
is "the account's home is empty, and still there for the next try" "0" "$(ls -A "$M" | wc -l | tr -d ' ')"
has "the person is told it was removed, and why that is safe" "it was a copy the mind account made, not your data" "$(said)"
has "... and that their own files were not touched" "Your own files were not touched" "$(said)"
is "the person's Hermes is untouched" "yes" "$([ -f "$H/.hermes/state.db" ] && [ -f "$H/.hermes/config.yaml" ] && echo yes || echo no)"

# L-3: the sentence is true
reset; mkdir -p "$M/.hermes/ro"; printf x > "$M/.hermes/ro/f"; chmod 555 "$M/.hermes/ro"
hi_discard_unfinished hermes "its installer stopped"; is "a read-only directory in the copy is removed anyway" "0" "$?"
is "... and the account's home is empty" "0" "$(ls -A "$M" | wc -l | tr -d ' ')"
reset; mkdir -p "$M/.hermes"; printf x > "$M/.hermes/f"
( hi_as_mind() { case "$1" in rm) return 1 ;; esac; "$@"; }; hi_discard_unfinished hermes "its installer stopped" ); is "a removal that fails returns failure" "1" "$?"
has "... and says it is still there" "it is still there and the disk may stay full" "$(said)"
lacks "... and never says removed" "removed the unfinished copy" "$(said)"
yes "... the copy is indeed still there" test -f "$M/.hermes/f"
rm -rf "$M/.hermes"

echo "-- L-4: unmeasurable data fails closed"
reset; disk 99999999
( hi_room_check hermes "$work/nohome" yes ) >/dev/null 2>&1; is "a home that cannot be entered refuses, however roomy the disk" "1" "$?"
has "... with a sentence saying why" "could not be measured" "$(said)"
has "... and that nothing was changed" "Nothing was changed" "$(said)"
has "... and a note" "could not measure" "$(cat "$HI_STATE/hermes.failed")"
reset; mkdir -p "$work/fakepy"; printf '#!/bin/sh\nexit 1\n' > "$work/fakepy/python3"; chmod 755 "$work/fakepy/python3"
( PATH="$work/fakepy:$PATH"; hi_room_check hermes "$H" yes ) >/dev/null 2>&1; is "a failing measurement step (python3) refuses" "1" "$?"
reset; ( hi_room_check hermes "$work/nohome" no ) >/dev/null 2>&1; is "a person with no data is not measured, and passes on a roomy disk" "0" "$?"

echo "-- L-1: a missing helper refuses"
mkdir -p "$work/partial"; cp "$here/harness-isolation.sh" "$here/harness-sandbox.sh" "$work/partial/"
( . "$work/partial/harness-isolation.sh" ) >/dev/null 2>&1; is "sourcing without harness-isolation-room.sh fails (it is not skipped)" "1" "$?"
bash -c '. "$1" >/dev/null 2>&1; type hi_install >/dev/null 2>&1' sh "$work/partial/harness-isolation.sh"; is "... and no move function is left defined" "1" "$?"
reset; disk 99999999
( unset -f hi_room_check; hi_install hermes ) >/dev/null 2>&1; is "hi_install refuses when a helper is undefined" "1" "$?"
has "... saying a helper is missing" "missing a harness-isolation helper" "$(said)"
is "... and installed nothing" "0" "$(wc -c < "$work/log/ready" | tr -d ' ')"
: > "$work/log/run"
( unset -f hi_abort; reconcile_harness_isolation ) >/dev/null 2>&1
is "reconcile starts no move when a helper is missing" "0" "$(wc -c < "$work/log/run" | tr -d ' ')"
reset

echo "== 3. the person's gateway: said to run only when it does"
# A user unit that behaves as the case says.
: > "$work/log/sysctl"
systemctl() {
  printf '%s\n' "$*" >> "$work/log/sysctl"
  case "$*" in
    *--user*" start "*) [ "${GW:-ok}" != nostart ] ;;
    *--user*is-active*) case "${GW:-ok}" in ok|restarts|deadlock) return 0 ;; late) c="$(cat "$work/log/calls" 2>/dev/null || echo 0)"; echo $((c + 1)) > "$work/log/calls"; [ "$c" -lt 2 ] ;; flap) [ -e "$work/log/flapped" ] && return 1; : > "$work/log/flapped"; return 0 ;; *) return 1 ;; esac ;;
    *--user*MainPID*) [ "${MP:-self}" = self ] && echo "$$" || echo 1 ;;
    *--user*NRestarts*) [ "${GW:-ok}" = restarts ] && echo 5 || echo 0 ;;
    *) true ;;
  esac
}
abort() { reset; rm -f "$work/log/flapped" "$work/log/calls"; disk 1000; if [ -n "${NOPID:-}" ]; then rm -f "$H/.hermes/gateway.pid"; else printf '{"pid": %s}\n' "$$" > "$H/.hermes/gateway.pid"; fi; ( hi_abort hermes active "$(command id -un)" "its installer stopped" ) >/dev/null 2>&1; echo $?; }
GW=ok;       is "an abort after a gateway that is up ends as a failure" "1" "$(abort)"
has "... and says it is running again, as it was" "your own hermes gateway is running again, as it was" "$(said)"
lacks "... without a warning" "NOT running" "$(said)"
GW=nostart;  abort >/dev/null
has "a unit that will not start is reported" "your own hermes gateway is NOT running: it would not start" "$(said)"
lacks "... and not called running again" "running again, as it was" "$(said)"
GW=flap;     abort >/dev/null
has "a unit that starts and then stops is reported" "it started and then stopped" "$(said)"
lacks "... and not called running again" "running again, as it was" "$(said)"
GW=restarts; abort >/dev/null
has "a crash loop (restarts counted) is reported" "it keeps restarting (5 times)" "$(said)"
lacks "... and not called running again" "running again, as it was" "$(said)"
GW=down;     abort >/dev/null
has "a unit that never comes active is reported after the bounded wait" "it was not active after 30 s" "$(said)"
GW=ok
reset; disk 1000; printf '{"pid": 2147483000}\n' > "$H/.hermes/gateway.pid"
( hi_abort hermes active "$(command id -un)" "its installer stopped" ) >/dev/null 2>&1
has "a lock that names a process that is not running is reported" "its lock names process 2147483000, which is not running" "$(said)"
lacks "... and not called running again" "running again, as it was" "$(said)"
printf '{"pid": %s}\n' "$$" > "$H/.hermes/gateway.pid"
reset; ( hi_abort hermes active "$(command id -un)" "its installer stopped" ) >/dev/null 2>&1
has "a lock that names the unit's main process counts" "running again, as it was" "$(said)"
MP=other; abort >/dev/null
has "a live pid that is not this unit's (not its MainPID, not in its cgroup) is reported" "which is not this unit's" "$(said)"
lacks "... and not called running again" "running again, as it was" "$(said)"
mkdir -p "$work/proc/$$"; printf '0::/user.slice/user-1.slice/hermes-gateway.service\n' > "$work/proc/$$/cgroup"
abort >/dev/null
has "... but a pid in the unit's cgroup counts" "running again, as it was" "$(said)"
MP=self; rm -rf "$work/proc"
NOPID=1 abort >/dev/null
has "520's case: active, no restarts, and no lock at all is NOT running (could not be checked)" "its lock (gateway.pid) was not there after 0 s, so it could not be checked" "$(said)"
lacks "... and not called running again" "running again, as it was" "$(said)"
GW=late; HI_HOLD=2 abort >/dev/null
has "a unit that is up at first and gone inside the hold is caught by the polling" "it started and then stopped" "$(said)"
lacks "... and not called running again" "running again, as it was" "$(said)"
GW=ok; HI_HOLD=0
is "the default hold is longer than the restart interval 520 measured (29 s)" "yes" "$( (unset HI_HOLD; . "$here/harness-isolation-room.sh"; [ "$HI_HOLD" -ge 30 ] && echo yes || echo no) )"
rm -f "$H/.hermes/gateway.pid"
has "the warning gives the free space it saw" "MB free" "$(GW=nostart; abort >/dev/null; said)"
GW=ok
disk 1000; reset; mkdir -p "$M/.hermes"; head -c 1048576 /dev/zero > "$M/.hermes/state.db"
GW=ok; ( hi_abort hermes active "$(command id -un)" "its installer stopped" ) >/dev/null 2>&1
is "the unfinished copy was removed before the person's gateway started (it needs the room)" "yes" "$([ ! -e "$M/.hermes/state.db" ] && echo yes || echo no)"
is "an idle gateway is not started by an abort" "no" "$( reset; : > "$work/log/sysctl"; ( hi_abort hermes idle "$(command id -un)" "x" ) >/dev/null 2>&1; grep -q -e '--user.* start ' "$work/log/sysctl" && echo yes || echo no)"

echo "== 4. a failed move is not started again by an update"
reset; disk 99999999; GW=ok
( hi_abort hermes active "$(command id -un)" "its installer stopped" ) >/dev/null 2>&1
has "a failed move leaves a note with the reason" "its installer stopped" "$(cat "$HI_STATE/hermes.failed")"
has "... and the time" "failed 20" "$(cat "$HI_STATE/hermes.failed")"
: > "$work/said"
yes "the note is read back as the last failure" hi_auto_blocked hermes
has "... and the sentence says it is not retried by an update, and how to retry" "An update does not retry it" "$(said)"
: > "$work/log/run"; : > "$work/said"
reconcile_harness_isolation
is "reconcile (an update) does not start the move" "0" "$(wc -c < "$work/log/run" | tr -d ' ')"
has "... and says why" "stays the person's" "$(said)"
: > "$work/log/run"
( command() { return 1; }; reconcile_harness_isolation; hi_migrate_all ) >/dev/null 2>&1
no "... and its fallback (no systemd-run) does not run an installer either" test -s "$work/log/ready"
echo "-- explicit asks"
disk 1000
( hi_install hermes ) >/dev/null 2>&1; is "\`harness install\` is not blocked by the note: it tries, and the room check answers" "1" "$?"
has "... with the room sentence, not the note's" "it needs about" "$(said)"
echo "-- a rolled-back build that cannot do the move does not start one"
reset; disk 99999999
: > "$work/log/run"
reconcile_harness_isolation
has "with no note, a person's Hermes is moved in the background (control)" "migrate-harnesses" "$(cat "$work/log/run")"
: > "$work/log/run"; printf '# an older installer\n' > "$PREFIX/share/harnesses/lib/install/hermes.sh"
reconcile_harness_isolation
is "an installer from before the mind account starts nothing" "0" "$(wc -c < "$work/log/run" | tr -d ' ')"
printf '# %s\n' "$HI_CAPABILITY" > "$PREFIX/share/harnesses/lib/install/hermes.sh"
echo "-- a success clears the note"
hi_note_failure hermes "x"; yes "(a note is there)" test -f "$HI_STATE/hermes.failed"
hi_mark_migrated hermes
no "marking it moved clears the note" test -e "$HI_STATE/hermes.failed"
yes "... and it is marked" hi_migrated hermes
rm -f "$HI_STATE/hermes.isolated"
ln -s /etc/passwd "$HI_STATE/hermes.failed" 2>/dev/null
no "a note that is a link is not believed" hi_auto_blocked hermes
hi_note_failure hermes "again"
is "... and is replaced by a plain file" "plain" "$([ -L "$HI_STATE/hermes.failed" ] && echo link || echo plain)"

echo "== 5. what ships"
lacks "no code path keeps a failed copy any more" "mv \"\$home\" \"\$home.failed" "$(cat "$here/harness-isolation.sh" "$here/harness-isolation-room.sh")"
has "the updater trusts the new file as part of itself (roll-forward only)" 'harness-isolation-room.sh' "$(sed -n '/^is_updater_file()/,/^}/p' "$here/yantrik-update")"
has "the build ships it in bin/" 'harness-isolation-room.sh' "$(cat "$here/build-release.sh")"
for f in harness-isolation.sh harness-isolation-room.sh harness-isolation-room-selftest.sh harness-isolation-selftest.sh; do
  is "$f is under 600 lines" "yes" "$([ "$(wc -l < "$here/$f")" -le 600 ] && echo yes || echo no)"
done
no "no key or token in what was said" grep -qi 'token\|api_key' "$work/said"

echo
if [ "$fails" = 0 ]; then echo "all selftest checks passed"; exit 0; fi
echo "$fails selftest check(s) failed"; exit 1
