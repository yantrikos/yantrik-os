#!/bin/bash
# Checks for harness-isolation.sh (#411): bash deploy/yantrik-os/harness-isolation-selftest.sh
#
# Part A runs anywhere: the unit, the tree program, the person's `hermes` command, the command list,
# and how the updater and the installer are wired to the library.
#
# Part B needs root and creates two throwaway accounts (a "person" and a "service" account that
# stands in for yantrik-mind, the same way: a different uid, a group the person is not in), then
# runs the library's own functions against them. It proves, with fake keys only:
#   - the service account cannot read a file in the person's home, nor list it, nor write in it;
#   - the person cannot read the service account's home, where the copied keys land;
#   - what moves is counted twice and identical, is private (0600) and is the service account's;
#   - the person's own copy is untouched until the move has been proved, and is kept after it;
#   - a failed install leaves the person's Hermes as it was and the person's gateway running again;
#   - every step can be run again.
# systemd is not touched: `systemctl` is a recorder here. Skipped, saying so, without root.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
fails=0
is() { # <name> <expected> <actual>
  if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"; else printf '  FAIL %s\n       want: %s\n       got:  %s\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
}
has() { # <name> <needle> <haystack>
  case "$3" in *"$2"*) printf '  ok   %s\n' "$1" ;; *) printf '  FAIL %s\n       missing: %s\n' "$1" "$2"; fails=$((fails + 1)) ;; esac
}
lacks() { # <name> <needle> <haystack>
  case "$3" in *"$2"*) printf '  FAIL %s\n       found: %s\n' "$1" "$2"; fails=$((fails + 1)) ;; *) printf '  ok   %s\n' "$1" ;; esac
}
yes() { if "${@:2}" >/dev/null 2>&1; then printf '  ok   %s\n' "$1"; else printf '  FAIL %s\n' "$1"; fails=$((fails + 1)); fi; }
no() { if "${@:2}" >/dev/null 2>&1; then printf '  FAIL %s\n' "$1"; fails=$((fails + 1)); else printf '  ok   %s\n' "$1"; fi; }

work="$(mktemp -d)"
chmod 755 "$work"
cleanup() {
  if [ "$(id -u)" = 0 ]; then
    userdel -r yk411p >/dev/null 2>&1; userdel -r yk411m >/dev/null 2>&1; groupdel yk411g >/dev/null 2>&1
  fi
  rm -rf "$work"
}
trap cleanup EXIT

# What the updater gives the library.
PREFIX="$work/opt"; BIN_DIR="$PREFIX/bin"
say() { :; }; info() { printf '%s\n' "$*" >> "$work/said"; }; warn() { printf '%s\n' "$*" >> "$work/said"; }
fail() { printf '%s\n' "FAIL: $*" >> "$work/said"; exit 1; }
tree_is_roots() { return 0; }
desktop_owner() { printf 'yk411p'; }
migrate_if_needed() { :; }
elevate() { :; }
reconcile_minds() { :; }; reconcile_mind_egress_units() { :; }; reconcile_mind_units() { :; }
reconcile_egress() { :; }; reconcile_mind_egress() { :; }
export YANTRIK_SELFTEST=1 HI_UNIT_DIR="$work/units" HI_DOOR="$work/door" HI_BASE="$work/mind/harness" HI_USER=yk411m HI_GROUP=yk411g HI_STATE="$work/state" HI_LOCK="$work/lock" HI_PROC="$work/proc"
# shellcheck source=deploy/yantrik-os/harness-isolation.sh
. "$here/harness-isolation.sh"

echo "== A. what ships"
unit="$repo/config/systemd/hermes-gateway.service"
live="$(grep -v '^[[:space:]]*#' "$unit")"
for want in 'User=yantrik-mind' 'Group=yantrik-minds' 'Environment=HOME=/var/lib/yantrik-mind/harness/hermes' 'Environment=YANTRIK_MIND_RUN=/run/yantrik-minds' 'Requires=yantrik-mind-egress.service' 'nft list table inet yantrik_mind_egress' 'ProtectHome=yes' 'TemporaryFileSystem=/var/lib/yantrik-mind:ro' 'BindPaths=/var/lib/yantrik-mind/harness/hermes' 'NoNewPrivileges=yes' 'UMask=0077' 'WantedBy=multi-user.target' 'PrivatePIDs=yes'; do has "the gateway unit has $want" "$want" "$live"; done
lacks "... reading no EnvironmentFile as root" "EnvironmentFile" "$live"
lacks "... and nothing of a person's home named" "/home/" "$live"
has "... started by the launcher, not by a venv path that moves" ".local/bin/hermes gateway run" "$live"
has "the Mind cannot see the harnesses' directory" "InaccessiblePaths=-/var/lib/yantrik-mind/harness" "$(grep -v '^[[:space:]]*#' "$repo/config/systemd/yantrik-mind.service")"
block="$(sed -n '/^# sandbox:begin/,/^# sandbox:end/p' "$unit" | grep -v '^#' | grep -v '^$')"
has "... in the sandbox block that every run of account code shares" "PrivatePIDs=yes" "$block"
has "... with the mount half beside it" "TemporaryFileSystem=/var/lib/yantrik-mind:ro" "$block"
lacks "... and none of the unit's own start-up lines in it" "ExecStart" "$block"
has "a systemd without PrivatePIDs does not start it (fail closed)" "has no PrivatePIDs=, so it is not started" "$live"
has "a home that is a link does not start it" "is not the account own plain directory" "$live"
mind_live="$(grep -v '^[[:space:]]*#' "$repo/config/systemd/yantrik-mind.service")"
has "the Mind's unit has its own PID namespace too, so it holds both ways" "PrivatePIDs=yes" "$mind_live"
has "... with its stop bounded" "TimeoutStopSec=30" "$mind_live"
has "... and the unit says what was measured, and where" "staging (systemd 257.8" "$(cat "$repo/config/systemd/yantrik-mind.service")"
has "... and says the cost of its stop is accepted until its bundle handles SIGTERM" "accepted until its bundle" "$(cat "$repo/config/systemd/yantrik-mind.service")"
lacks "... nor tell anyone not to chase the 'not our child' line" "do not chase" "$(cat "$repo/config/systemd/yantrik-mind.service" "$unit" "$repo/harnesses/hermes/README.md")"
has "... the Mind's unit hides /dev/pts from the Mind (the hermes model pty), ignoring a missing path" "InaccessiblePaths=-/dev/pts" "$mind_live"
has "... and /dev/shm is private and writable" "TemporaryFileSystem=/dev/shm:mode=1777" "$block"
has "the gateway refuses to start beside a Mind unit that does not isolate (rollback)" "does not mask harness/ and have its own PID namespace" "$live"
lacks "... and nowhere claims a stop waits out the timeout" "waits out" "$(cat "$repo/config/systemd/yantrik-mind.service" "$repo/config/systemd/hermes-gateway.service" "$repo/harnesses/hermes/README.md" "$here/harness-isolation.sh")"
has "... and says the 'not our child' journal line is expected" "not our child" "$(cat "$repo/config/systemd/yantrik-mind.service")"
lacks "... no TODO left" "TODO(#411)" "$(cat "$repo/config/systemd/yantrik-mind.service")"
lacks "... and no version gate that could leave a machine without a Mind" "systemctl --version" "$mind_live"
lacks "... and no start condition on the harness directory or anything of Hermes's" "harness/hermes" "$(printf '%s\n' "$mind_live" | grep -E '^(Condition|ExecStartPre|Requires|After)')"
lacks "the harness unit's comments no longer say the Mind can read it" "can still read" "$(cat "$unit")"
if command -v systemd-analyze >/dev/null 2>&1; then
  v="$(systemd-analyze verify --man=no "$unit" 2>&1 | grep -iE "unknown (key|section)|invalid|bad (unit|setting)|failed to parse" || true)"
  sdv="$(systemd-analyze --version 2>/dev/null | head -n 1 | cut -d' ' -f2)"
  if [ "${sdv:-0}" -lt 257 ] 2>/dev/null; then
    # An older systemd IGNORES the line (this is the silent failure the unit's start-up check and
    # the installer exist for): it must be the only thing it does not know.
    unknown="$(printf '%s' "$v" | grep -o "Unknown key name '[A-Za-z]*'" | cut -d"'" -f2 | sort -u | paste -sd' ')"
    is "systemd $sdv (older than 257) ignores PrivatePIDs= and nothing else in the unit" "PrivatePIDs" "$unknown"
  else
    is "systemd-analyze finds no unknown or bad setting in the unit" "" "$v"
  fi
fi

up="$repo/deploy/yantrik-os/yantrik-update"
has "the updater sources the library beside it" '. "$HARNESS_ISOLATION"' "$(cat "$up")"
has "... it is roll-forward only, like the rest it trusts" '[ "$1" = harness-isolation.sh ]' "$(cat "$up")"
has "... the command is dispatched" "harness) type cmd_harness" "$(cat "$up")"
has "... migrate-harnesses too" "migrate-harnesses) type cmd_migrate_harnesses" "$(cat "$up")"
is "... reconcile and the up-to-date apply both bring harnesses along" "2" "$(grep -c 'type reconcile_harness_isolation' "$up")"
has "the build ships the library in bin/" 'harness-isolation.sh" "$ROOT/bin/harness-isolation.sh"' "$(cat "$here/build-release.sh")"
has "the installer knows both phases" "$HI_CAPABILITY" "$(cat "$repo/harnesses/lib/install/hermes.sh")"
has "the installer hands the button over to the updater" "yantrik-update harness install hermes" "$(cat "$repo/harnesses/lib/install/hermes.sh")"
is "a phase it does not know is refused" "yes" "$(HOME="$work" YANTRIK_HARNESS_ISOLATED=bogus sh "$repo/harnesses/lib/install/hermes.sh" "$repo/harnesses/hermes" 2>&1 | grep -q 'setup or prove' && echo yes || echo no)"

echo "== A. the stop and the start of the gateway unit"
lacks "no ExecStop= drain: its premise (a stop kills at once) was wrong on 257.13" "ExecStop" "$(grep -v '^[[:space:]]*#' "$unit")"
tso="$(sed -n 's/^TimeoutStopSec=//p' "$unit")"
rd="$(sed -n 's/^RESTART_DRAIN_SECONDS = //p' "$repo/harnesses/lib/install/hermes_config.py")"
cd_="$(sed -n 's/^CRON_DRAIN_SECONDS = //p' "$repo/harnesses/lib/install/hermes_config.py")"
want=$(( ( (rd > cd_ + 10 ? rd : cd_ + 10) + 30 ) > 60 ? ( (rd > cd_ + 10 ? rd : cd_ + 10) + 30 ) : 60 ))
is "TimeoutStopSec is what Hermes's own resolve_systemd_timeout_stop_sec gives for the drains hermes_config sets" "$want" "$tso"
has "the unit says what the drain keys are and what was measured" "restart_drain_timeout" "$(cat "$unit")"
has "the gateway starts under the reaping init" "ExecStart=/opt/yantrik/bin/yantrik-init " "$live"
has "Hermes is told its own home, so gateway_state.json never records another" "Environment=HERMES_HOME=/var/lib/yantrik-mind/harness/hermes/.hermes" "$block"
has "hermes_config writes the drain keys" "agent[\"restart_drain_timeout\"]" "$(cat "$repo/harnesses/lib/install/hermes_config.py")"
lacks "no comment says a stop is a kill with or without a handler any more" "with or without a SIGTERM handler" "$(cat "$unit" "$repo/config/systemd/yantrik-mind.service" "$repo/harnesses/hermes/README.md")"
has "the Mind's measured stop on the VM is stated" "30.13" "$(cat "$repo/config/systemd/yantrik-mind.service")"
init="$here/yantrik-init"
has "the gateway's init scopes the abstract sockets, and fails closed" "--scope-abstract-unix /var/lib" "$live"
has "... and so does every task" "yantrik-init\" --scope-abstract-unix" "$(cat "$here/harness-sandbox.sh")"
# M1: the person's Xwayland listens on an abstract socket. Either the kernel can scope it (the
# connect is refused) or it cannot (nothing is run, exit 126); never "ran, and connected".
cat > "$work/abs.py" <<'PY'
import os, socket, subprocess, sys
name = "\0yantrik-selftest-%d" % os.getpid()
srv = socket.socket(socket.AF_UNIX)
srv.bind(name)
srv.listen(1)
child = ("import socket, sys\n"
         "s = socket.socket(socket.AF_UNIX)\n"
         "try:\n s.connect(sys.argv[1]); print('CONNECTED')\n"
         "except OSError:\n print('DENIED')\n")
r = subprocess.run([sys.executable, sys.argv[1], "--scope-abstract-unix", sys.executable, "-c", child, name],
                   capture_output=True, text=True)
print(r.returncode, r.stdout.strip())
PY
r="$(python3 "$work/abs.py" "$init" 2>/dev/null)"
case "$r" in "0 DENIED" | "126") r="refused" ;; esac
is "yantrik-init --scope-abstract-unix: the connect is refused, or nothing is run" "refused" "$r"
if unshare --pid --fork --mount-proc true >/dev/null 2>&1; then
  zombies() { unshare --pid --fork --mount-proc "$@" 2>/dev/null; }
  z="$(zombies python3 "$init" sh -c '(sleep 0.3 &) ; sleep 1.5; ps -eo stat= | grep -c "^Z"')"
  is "yantrik-init as PID 1 reaps what its child orphans (no zombie)" "0" "$z"
  z="$(zombies python3 -c 'import os,time
if os.fork() == 0:
    if os.fork() == 0:
        time.sleep(0.2); os._exit(0)
    os._exit(0)
time.sleep(1.5)
import subprocess; print(subprocess.run("ps -eo stat= | grep -c ^Z", shell=True, capture_output=True, text=True).stdout.strip())')"
  is "control: a PID 1 that does not wait leaves the orphan a zombie (the test can see them)" "yes" "$([ "${z:-0}" -ge 1 ] 2>/dev/null && echo yes || echo no)"
  zombies python3 "$init" sh -c 'exit 7'; is "it ends with its child's exit status" "7" "$?"
  zombies python3 "$init" sh -c 'kill -TERM $$'; is "... and a signal as 128 plus it" "143" "$?"
  r="$(zombies python3 "$init" sh -c 'trap "echo got-term; exit 0" TERM; kill -TERM 1 2>/dev/null; sleep 5 & wait' 2>&1 | head -1)"
  is "SIGTERM sent to it is passed on to its child" "got-term" "$r"
else
  echo "  skipped: cannot make a PID namespace here"
fi
mkdir -p "$work/fakebin2"
cat > "$work/fakebin2/systemctl" <<'FAKE'
#!/bin/sh
case "$*" in
  "show -p InaccessiblePaths --value yantrik-mind.service") printf '%s\n' "${FAKE_MASK-/var/lib/yantrik-mind/harness}" ;;
  "show -p PrivatePIDs --value yantrik-mind.service") printf '%s\n' "${FAKE_MIND_PIDS:-yes}" ;;
  "show -p MainPID --value yantrik-mind.service") printf '%s\n' "${FAKE_MIND_MAINPID:-0}" ;;
  "show -p DropInPaths --value hermes-gateway.service") printf '%s\n' "${FAKE_DROPIN:-}" ;;
esac
FAKE
chmod 755 "$work/fakebin2/systemctl"
preline="$(grep '^ExecStartPre=+.*u=yantrik-mind.service' "$unit" | sed -e 's/^ExecStartPre=+//' -e 's/\$\$/$/g')"
gate() { # env assignments...: the unit's check against a Mind unit that answers as told; its exit status
  ( env "$@" PATH="$work/fakebin2:$PATH" sh -c "$(printf '%s' "$preline" | sed -e "s/^\/bin\/sh -c '//" -e "s/'$//")" ) >/dev/null 2>&1
}
yes "the gateway starts beside a Mind unit that masks harness/ and has its own namespace (not running)" gate FAKE_MIND_MAINPID=0
no "... not beside one that does not mask harness/ (a rollback below this release)" gate FAKE_MASK=
no "... not beside one without PrivatePIDs" gate FAKE_MIND_PIDS=no
no "... not beside a running Mind in PID 1's namespace" gate FAKE_MIND_MAINPID=$$
no "... not with a drop-in on its own unit" gate FAKE_DROPIN=/etc/systemd/system/hermes-gateway.service.d/x.conf
# L2: the namespace gate fails closed. /proc as the check reads it: the Mind's link readable, PID 1's not.
mkdir -p "$work/fakeproc/1/ns" "$work/fakeproc/4242/ns"
ln -sf 'pid:[4026539999]' "$work/fakeproc/4242/ns/pid"
gate_proc() { ( env "$@" PATH="$work/fakebin2:$PATH" sh -c "$(printf '%s' "$preline" | sed -e "s/^\/bin\/sh -c '//" -e "s/'$//" -e "s#/proc/#$work/fakeproc/#g")" ) >/dev/null 2>&1; }
no "... not when PID 1's namespace cannot be read (it cannot be told apart from the Mind's)" gate_proc FAKE_MIND_MAINPID=4242
ln -sf 'pid:[4026531836]' "$work/fakeproc/1/ns/pid"
yes "... control: with both readable and different it starts" gate_proc FAKE_MIND_MAINPID=4242
rm -f "$work/fakeproc/4242/ns/pid"
no "... not when the Mind's own namespace cannot be read" gate_proc FAKE_MIND_MAINPID=4242

echo "== A. the person's hermes command"
mkdir -p "$work/shimbin"
hi_shim_text > "$work/shimbin/hermes"; chmod 755 "$work/shimbin/hermes"
yes "the command is valid sh" sh -n "$work/shimbin/hermes"
printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> "%s/sudo.log"\n' "$work" > "$work/shimbin/sudo"; chmod 755 "$work/shimbin/sudo"
rm -f "$work/sudo.log"
PATH="$work/shimbin:$PATH" "$work/shimbin/hermes" model >/dev/null 2>&1
has "model goes to the updater, as the mind account" "harness run hermes model" "$(cat "$work/sudo.log" 2>/dev/null)"
PATH="$work/shimbin:$PATH" "$work/shimbin/hermes" gateway install >"$work/shim.out" 2>&1; rc=$?
is "a second gateway cannot be made through it (exit)" "2" "$rc"
is "... and nothing was asked of sudo for it" "1" "$(wc -l < "$work/sudo.log" | tr -d ' ')"
has "... it says what it does do" "model, status, --version" "$(cat "$work/shim.out")"

echo "== A. what a person can ask for as the mind account"
orig_migrated="$(declare -f hi_migrated)"
hi_migrated() { return 0; }
for bad in "sh" "-c" "gateway" "config" "chat" ""; do
  out="$( ( hi_run hermes $bad ) 2>&1 )"; rc=$?
  is "run hermes '$bad' is refused" "1" "$rc"
done
for extra in "model --anything" "model model" "status x" "--version -h" "model ; id"; do
  out="$( ( hi_run hermes $extra ) 2>&1 )"; rc=$?
  is "run hermes '$extra' is refused (the verb takes no arguments)" "1" "$rc"
done
eval "$orig_migrated"
is "an unknown harness is refused" "1" "$( (hi_install nope >/dev/null 2>&1); echo $?)"

echo "== A. what moves"
t="$work/tree"; mkdir -p "$t/.hermes/sessions" "$t/.hermes/hermes-agent" "$t/.hermes/plugins/yantrik" "$t/.hermes/plugins/other" \
  "$t/.hermes/tools" "$t/.local/share/yantrik/memory"
printf 'model: x\n' > "$t/.hermes/config.yaml"
printf 'OPENAI_API_KEY=FAKE-KEY-yk411-not-a-secret\n' > "$t/.hermes/.env"
printf 'one' > "$t/.hermes/sessions/a.json"; printf 'db' > "$t/.hermes/state.db"; printf 'wal' > "$t/.hermes/state.db-wal"
printf 'code' > "$t/.hermes/hermes-agent/run.py"; printf 'p' > "$t/.hermes/plugins/yantrik/plugin.yaml"
printf 'q' > "$t/.hermes/plugins/other/plugin.yaml"; printf 'u' > "$t/.hermes/tools/uv"
printf 'm' > "$t/.local/share/yantrik/memory/hermes.db"; printf 'm' > "$t/.local/share/yantrik/memory/hermes.db-wal"
printf 'm' > "$t/.local/share/yantrik/memory/pi.db"
ln -s /nonexistent "$t/.hermes/dangling"
python3 -c "import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])" "$t/.hermes/gateway.sock"
listed="$(python3 -I -c "$HI_PY_TREE" list "$t" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)" | tr '\0' ' ') "
for want in .hermes/config.yaml .hermes/.env .hermes/state.db .hermes/state.db-wal .hermes/sessions/a.json \
  .hermes/plugins/other/plugin.yaml .hermes/dangling .local/share/yantrik/memory/hermes.db .local/share/yantrik/memory/hermes.db-wal; do
  has "moves $want" "$want " "$listed"
done
for not in hermes-agent/run.py plugins/yantrik/plugin.yaml .hermes/tools/uv gateway.sock memory/pi.db; do
  lacks "does not move $not" "$not" "$listed"
done
d1="$(python3 -I -c "$HI_PY_TREE" digest "$t" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"
printf 'changed' >> "$t/.hermes/sessions/a.json"
d2="$(python3 -I -c "$HI_PY_TREE" digest "$t" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"
no "a changed byte changes the digest" test "$d1" = "$d2"
printf 'code2' >> "$t/.hermes/hermes-agent/run.py"
d3="$(python3 -I -c "$HI_PY_TREE" digest "$t" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"
is "... and a change in the code does not" "$d2" "$d3"

echo "== B. real accounts"
if [ "$(id -u)" != 0 ] || ! command -v useradd >/dev/null 2>&1 || ! command -v setpriv >/dev/null 2>&1; then
  echo "  skipped: needs root, useradd and setpriv (CI runs this with sudo)"
  [ "$fails" = 0 ] && { echo "all selftest checks passed (part B skipped)"; exit 0; }
  echo "$fails selftest check(s) failed"; exit 1
fi
mkdir -p "$work/home" "$work/mind"; chmod 755 "$work/home" "$work/mind"
groupadd yk411g
useradd -M -d "$work/home/yk411p" -s /bin/sh yk411p || { echo "cannot make the test accounts"; exit 1; }
useradd -M -d "$work/mind" -g yk411g -s /usr/sbin/nologin yk411m
mkdir -p "$work/home/yk411p"; chown yk411p "$work/home/yk411p"; chmod 700 "$work/home/yk411p"
chown yk411m:yk411g "$work/mind"; chmod 750 "$work/mind"
mkdir -p "$HI_UNIT_DIR" "$HI_DOOR" "$PREFIX/share/harnesses/lib/install" "$PREFIX/share/harnesses/hermes"
cp "$unit" "$HI_UNIT_DIR/"
mkdir -p "$BIN_DIR"; cp "$here/yantrik-init" "$BIN_DIR/yantrik-init"; chmod 755 "$BIN_DIR/yantrik-init"
printf '# %s\n' "$HI_CAPABILITY" > "$PREFIX/share/harnesses/lib/install/hermes.sh"
chmod -R a+rX "$work/opt" 2>/dev/null

P="$work/home/yk411p"
sudo_p() { setpriv --reuid="$(id -u yk411p)" --regid="$(id -g yk411p)" --clear-groups -- "$@"; }
sudo_m() { setpriv --reuid="$(id -u yk411m)" --regid="$(id -g yk411m)" --clear-groups -- "$@"; }
mkdir -p "$work/fakebin" "$work/log"
cat > "$work/fakebin/systemd-run" <<'FAKE'
#!/bin/bash
# Stands in for systemd-run: records its arguments, then runs the command as the account named by
# -p User= in the directory named by -p WorkingDirectory= (no sandbox: that is systemd's to give).
printf '%s\n' "$@" >> "$FAKE_RUN_LOG"
user= wd=
while [ $# -gt 0 ]; do
  case "$1" in
    -p) case "$2" in User=*) user="${2#User=}" ;; WorkingDirectory=*) wd="${2#*=}" ;; esac; shift 2 ;;
    --) shift; break ;;
    *) shift ;;
  esac
done
exec setpriv --reuid="$(id -u "$user")" --regid="$(id -g "$user")" --clear-groups -- env -i HOME="$wd" PATH=/usr/bin:/bin "$@"
FAKE
chmod 755 "$work/fakebin/systemd-run"
printf '#!/bin/sh\n[ "${FAKE_NFT:-ok}" = ok ]\n' > "$work/fakebin/nft"; chmod 755 "$work/fakebin/nft"
# /proc as the check reads it: PID 1 in the host's PID namespace, the unit's main process in another.
mkdir -p "$work/proc/1/ns" "$work/proc/4242/ns"
ln -s 'pid:[4026531836]' "$work/proc/1/ns/pid"; ln -s 'pid:[4026539999]' "$work/proc/4242/ns/pid"
export FAKE_RUN_LOG="$work/log/run.args" PATH="$work/fakebin:$PATH"

sudo_p sh -c 'umask 077; mkdir -p "$1/.hermes/sessions" "$1/.hermes/hermes-agent" "$1/.hermes/plugins/yantrik" "$1/.local/share/yantrik/memory" "$1/.local/bin" "$1/.config/systemd/user"
  printf "model: x\n" > "$1/.hermes/config.yaml"
  printf "OPENAI_API_KEY=FAKE-KEY-yk411-not-a-secret\n" > "$1/.hermes/.env"
  printf "FAKE-CANARY-yk411\n" > "$1/notes.txt"
  head -c 4096 /dev/urandom > "$1/.hermes/state.db"; printf wal > "$1/.hermes/state.db-wal"
  printf s1 > "$1/.hermes/sessions/a.json"; printf code > "$1/.hermes/hermes-agent/run.py"
  printf mem > "$1/.local/share/yantrik/memory/hermes.db"
  mkdir -p "$1/.cache/yantrikdb/models/potion-base-8M"; printf FAKE-MODEL > "$1/.cache/yantrikdb/models/potion-base-8M/model.safetensors"
  printf "{\"hermes_home\": \"%s/.hermes\"}" "$1" > "$1/.hermes/gateway_state.json"
  printf "#!/bin/sh\necho old launcher\n" > "$1/.local/bin/hermes"; chmod 755 "$1/.local/bin/hermes"
  printf "[Service]\nExecStart=/bin/true\n" > "$1/.config/systemd/user/hermes-gateway.service"' sh "$P"
before="$(python3 -I -c "$HI_PY_TREE" digest "$P" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"
hashes="$(sudo_p sh -c 'cd "$1/.hermes" && sha256sum config.yaml .env state.db sessions/a.json' sh "$P")"
echo "-- the service account cannot reach the person"
no "it cannot read a file in the person's home" sudo_m cat "$P/notes.txt"
no "... nor the person's keys" sudo_m cat "$P/.hermes/.env"
no "... nor list the home" sudo_m ls "$P"
no "... nor write in it" sudo_m sh -c "echo x > '$P/planted'"
yes "(the person can read their own file: the test is not vacuous)" sudo_p cat "$P/notes.txt"
yes "(and the account can use its own home)" sudo_m sh -c "ls '$work/mind' >/dev/null"
is "hi_as_mind sees the same refusal through the library" "1" "$(hi_as_mind cat "$P/notes.txt" >/dev/null 2>&1; echo $?)"
echo "-- how the account is run"
export LEAK_yk411="FAKE-KEY-yk411-env"
seen="$(hi_as_mind sh -c 'printf "%s|%s|%s|%s" "${LEAK_yk411:-none}" "$HOME" "$YANTRIK_MIND_RUN" "$(id -un)"')"
is "a clean environment: nothing of root's passes, the door and the account's home do" "none|$HI_BASE|$HI_DOOR|yk411m" "$seen"
unset LEAK_yk411
t0=$SECONDS
out="$(hi_run_timed 2 yk411m "$work/mind" sh -c 'printf "line one\033[31m red\n"; sleep 60' 2>&1)"; rc=$?
is "a harness that will not stop is ended at its time limit" "yes" "$([ "$rc" != 0 ] && [ $((SECONDS - t0)) -lt 20 ] && echo yes || echo no)"
has "... what it printed is relayed (to info)" "line one" "$(cat "$work/said")"
lacks "... without terminal escapes" "$(printf '\033')" "$(cat "$work/said")"
is "it has no terminal of the caller's (own session)" "yes" "$(hi_as_mind sh -c 'ps -o sid= -p $$' | awk -v me="$(ps -o sid= -p $$ | tr -d ' ')" '{print ($1 != me) ? "yes" : "no"}')"
echo "-- the copy"
mkdir -p "$work/log"; : > "$work/log/systemctl"
systemctl() { # a recorder; answers what the library asks of it
  printf '%s\n' "$*" >> "$work/log/systemctl"
  case "$*" in
    --version) printf 'systemd %s (%s.8-1)\n' "${FAKE_SYSTEMD:-257}" "${FAKE_SYSTEMD:-257}" ;;
    "show -p InaccessiblePaths --value yantrik-mind.service") printf '%s\n' "${FAKE_MASK-$HI_BASE}" ;;
    "show -p PrivatePIDs --value yantrik-mind.service") printf '%s\n' "${FAKE_MIND_PIDS:-yes}" ;;
    "show -p MainPID --value yantrik-mind.service") printf '%s\n' "${FAKE_MIND_MAINPID:-0}" ;;
    "is-active --quiet yantrik-mind.service") [ "${FAKE_MIND_ACTIVE:-0}" = 1 ] ;;
    "show -p DropInPaths --value "*) printf '%s\n' "${FAKE_DROPIN:-}" ;;
    "show -p PrivatePIDs --value "*) printf '%s\n' "${FAKE_PRIVPIDS:-yes}" ;;
    "show -p MainPID --value "*) printf '%s\n' "${FAKE_MAINPID:-4242}" ;;
    *--user*is-active*) [ -e "$work/log/user-active" ] ;;
    *) true ;;
  esac
}
echo "-- the harnesses' directory exists before the Mind is restarted (Blocker 2)"
rm -rf "$HI_BASE"
hi_mask_dir
is "made by reconcile_minds' step, empty, the account's, 0700" "yk411m 700 0" "$(stat -c '%U %a' "$HI_BASE" | tr '\n' ' ')$(ls -A "$HI_BASE" | wc -l | tr -d ' ')"
hi_mask_dir
is "... and again changes nothing" "yk411m 700" "$(stat -c '%U %a' "$HI_BASE")"
mv "$HI_BASE" "$HI_BASE.real"; ln -s "$HI_BASE.real" "$HI_BASE"
no "... a link in its place is refused, not followed" hi_mask_dir
rm -f "$HI_BASE"; mv "$HI_BASE.real" "$HI_BASE"
for fn in cmd_reconcile cmd_migrate_minds cmd_apply; do
  body="$(sed -n "/^$fn()/,/^}/p" "$up")"
  a="$(printf '%s\n' "$body" | grep -n 'reconcile_minds' | head -1 | cut -d: -f1)"; b="$(printf '%s\n' "$body" | grep -n 'reconcile_mind_units' | head -1 | cut -d: -f1)"
  is "$fn makes the directory (reconcile_minds) before it can restart the Mind (reconcile_mind_units)" "yes" "$([ -n "$a" ] && [ -n "$b" ] && [ "$a" -lt "$b" ] && echo yes || echo no)"
done
has "reconcile_minds is where the directory is made" "hi_mask_dir" "$(sed -n '/^reconcile_minds()/,/^}/p' "$up")"
hi_ready hermes
yes "the mind-side directory exists and is the account's" test "$(stat -c %U "$(hi_home hermes)")" = yk411m
hi_copy_data hermes yk411p "$P" || echo "copy failed: $(cat "$work/said")"
M="$(hi_home hermes)"
after="$(hi_as_mind python3 -I -c "$HI_PY_TREE" digest "$M" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"
is "the copy is counted twice, the same files and the same bytes" "$before" "$after"
is "the keys are the service account's" "yk411m" "$(stat -c %U "$M/.hermes/.env")"
is "... and private (0600)" "600" "$(stat -c %a "$M/.hermes/.env")"
is "... and the directory is private (0700)" "700" "$(stat -c %a "$M/.hermes")"
no "... the person cannot read them in their new home" sudo_p cat "$M/.hermes/.env"
no "... nor list it" sudo_p ls "$M/.hermes"
yes "... the service account can" sudo_m cat "$M/.hermes/.env"
no "Hermes's code was not carried over" test -e "$M/.hermes/hermes-agent"
is "the embedder model its memory cached is carried, so the account needs no download to remember" "FAKE-MODEL" "$(sudo_m cat "$M/.cache/yantrikdb/models/potion-base-8M/model.safetensors" 2>/dev/null)"
no "a runtime state file that names the person's home is not carried (gateway_state.json)" test -e "$M/.hermes/gateway_state.json"
is "the person's own copy is untouched" "$before" "$(python3 -I -c "$HI_PY_TREE" digest "$P" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"
no "no key was ever said or logged" grep -rq FAKE-KEY "$work/said" "$work/log"
echo "-- setting aside, and doing it again"
hi_set_aside hermes earlier
yes "a retry keeps the unfinished directory, in one slot" test -d "$M.failed"
yes "... and starts with an empty one" test -z "$(sudo_m ls -A "$M")"
hi_copy_data hermes yk411p "$P" && yes "the copy can be made again" true
hi_set_aside hermes earlier; hi_set_aside hermes earlier
is "setting aside again replaces the slot; an empty directory is a no-op: one copy, never a pile" "1" "$(ls -d "$M".failed* | wc -l | tr -d ' ')"
has "... and the person is told what is kept and that their own files were not touched" "kept the last unfinished" "$(cat "$work/said")"
echo "-- the whole install, with a stand-in installer"
rm -rf "$M" "$M".failed "$M".failed.new
hi_ready hermes
installer_calls="$work/log/installer"; : > "$installer_calls"
hi_run_installer() { # <name> <phase> <secs>: a stand-in that makes what the real one makes
  printf '%s\n' "$2" >> "$installer_calls"
  printf 'installer %s\n' "$2" >> "$work/log/systemctl"
  [ "${STAND_IN_KILL:-}" = "$2" ] && { kill -TERM $BASHPID; sleep 1; }
  [ "${STAND_IN_FAILS:-}" = "$2" ] && return 1
  if [ "$2" = setup ]; then
    sudo_m sh -c 'umask 077; mkdir -p "$1/.hermes/hermes-agent" "$1/.local/bin"; printf "#!/bin/sh\n" > "$1/.local/bin/hermes"; chmod 755 "$1/.local/bin/hermes"' sh "$M"
  fi
  return 0
}
export STAND_IN_FAILS=setup
: > "$work/log/user-active"
( hi_install hermes ) >/dev/null 2>&1; rc=$?
is "a failing installer fails the install" "1" "$rc"
is "... the person's Hermes was not moved or renamed" "yes" "$([ -f "$P/.hermes/config.yaml" ] && [ ! -e "$P/.hermes.yantrik-moved" ] && echo yes || echo no)"
is "... it is not marked as moved" "no" "$(hi_migrated hermes && echo yes || echo no)"
no "... and the unit was never enabled for boot" grep -q '^enable ' "$work/log/systemctl"
yes "... the system unit was turned off" grep -q 'disable --now hermes-gateway.service' "$work/log/systemctl"
yes "... the person's own gateway is running again" grep -q -e '--user -M yk411p@ start hermes-gateway.service' "$work/log/systemctl"
yes "... what was half made is kept aside, in the one slot" test -d "$M.failed"
has "... and the reason is named" "its installer stopped" "$(cat "$work/said")"
is "... and the person's copy is the same" "$before" "$(python3 -I -c "$HI_PY_TREE" digest "$P" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"

STAND_IN_FAILS=prove
: > "$work/log/systemctl"
( hi_install hermes ) >/dev/null 2>&1; rc=$?
is "a Hermes that starts but fails its proofs is not committed" "1" "$rc"
no "... and was never enabled for boot (enable waits for the proofs)" grep -q '^enable ' "$work/log/systemctl"
is "... and the person's is still in place" "yes" "$([ -f "$P/.hermes/config.yaml" ] && echo yes || echo no)"
echo "-- stopped half way, a PID namespace that is not there, a systemd that cannot (L1, fail closed)"
: > "$work/log/systemctl"; : > "$installer_calls"
STAND_IN_FAILS=; STAND_IN_KILL=setup
( hi_install hermes ) >/dev/null 2>&1; rc=$?
STAND_IN_KILL=
is "a move that is stopped ends like a failure" "1" "$rc"
yes "... the person's own gateway is running again" grep -q -e '--user -M yk411p@ start hermes-gateway.service' "$work/log/systemctl"
yes "... the system unit is off" grep -q 'disable --now hermes-gateway.service' "$work/log/systemctl"
no "... and was never enabled for boot" grep -q '^enable ' "$work/log/systemctl"
has "... and says it was interrupted" "was interrupted" "$(cat "$work/said")"
: > "$work/log/systemctl"
FAKE_PRIVPIDS=no
( hi_install hermes ) >/dev/null 2>&1; rc=$?
FAKE_PRIVPIDS=yes
is "a unit that runs without its own PID namespace is not accepted" "1" "$rc"
has "... and says so" "PrivatePIDs is 'no'" "$(cat "$work/said")"
no "... the proofs never ran on it" grep -q '^installer prove$' "$work/log/systemctl"
: > "$work/log/systemctl"
FAKE_MAINPID=1
( hi_install hermes ) >/dev/null 2>&1; rc=$?
FAKE_MAINPID=4242
is "a main process in PID 1's namespace is not accepted, whatever systemd reports (L3)" "1" "$rc"
has "... and says so" "shares PID 1's PID namespace" "$(cat "$work/said")"
no "... the proofs never ran on it" grep -q '^installer prove$' "$work/log/systemctl"
: > "$work/log/systemctl"; : > "$installer_calls"
FAKE_SYSTEMD=256
( hi_install hermes ) >/dev/null 2>&1; rc=$?
is "a systemd older than 257 refuses before anything changes" "1" "$rc"
has "... and says why" "older than 257" "$(cat "$work/said")"
is "... no installer ran" "0" "$(wc -l < "$installer_calls" | tr -d ' ')"
no "... and the person's gateway was not even stopped" grep -q -e '--user -M yk411p@ stop' "$work/log/systemctl"
: > "$FAKE_RUN_LOG"
no "... nor does a task start" hi_task 5 0 hermes /bin/true
is "... not even to try (systemd-run never called)" "0" "$(wc -c < "$FAKE_RUN_LOG" | tr -d ' ')"
FAKE_SYSTEMD=257
printf 'forged' > "$M/.yantrik-isolated"; chown yk411m "$M/.yantrik-isolated"
no "a marker the harness can write does not make it moved (L5)" hi_migrated hermes

STAND_IN_FAILS=
: > "$work/log/systemctl"
( hi_install hermes ) >/dev/null 2>&1; rc=$?
is "the install that works succeeds" "0" "$rc"
is "the phases ran in order, the unit between them" "setup prove" "$(grep -E '^(setup|prove)$' "$installer_calls" | tail -2 | tr '\n' ' ' | sed 's/ $//')"
yes "the person's gateway was stopped before the copy" grep -q -e '--user -M yk411p@ stop hermes-gateway.service' "$work/log/systemctl"
is "... and stopped again when it is disabled, not only disabled (L1)" "2" "$(grep -c -e '--user -M yk411p@ stop hermes-gateway.service' "$work/log/systemctl")"
yes "the system unit was enabled and restarted" grep -q 'enable hermes-gateway.service' "$work/log/systemctl"
yes "it is marked as moved, in root's own state file" test "$(stat -c %U "$HI_STATE/hermes.isolated")" = root
no "... which the account cannot write" sudo_m sh -c "echo x > '$HI_STATE/hermes.isolated'"
is "the unit was enabled only after the proofs passed" "yes" "$(awk '/^installer prove$/{p=NR} /^enable hermes-gateway.service$/{e=NR} END{print (p && e && p < e) ? "yes" : "no"}' "$work/log/systemctl")"
has "the person is told their keys are still readable in the old copy" "API keys are still in it" "$(cat "$work/said")"
is "the person's old copy is kept, whole, under its new name" "$hashes" "$(sudo_p sh -c 'cd "$1/.hermes.yantrik-moved" && sha256sum config.yaml .env state.db sessions/a.json' sh "$P")"
no "the old ~/.hermes is gone from the person's home" test -e "$P/.hermes"
yes "the Hermes memory file was kept beside it" sh -c "ls '$P'/.hermes.yantrik-moved/outside-home/hermes.db >/dev/null"
yes "the old launcher is kept and the person's hermes is the shim" grep -q yantrik-harness-shim "$P/.local/bin/hermes"
yes "... the old launcher is in the kept directory" grep -q 'old launcher' "$P/.hermes.yantrik-moved/outside-home/launcher"
is "the old user unit is masked" "/dev/null" "$(readlink "$P/.config/systemd/user/hermes-gateway.service")"
yes "... and the file itself is kept" test -f "$P/.hermes.yantrik-moved/hermes-gateway.service"
yes "Settings reads it as installed" test -f "$P/.local/state/yantrik/harnesses/hermes/installed"
no "the person's account cannot read what moved" sudo_p cat "$M/.hermes/.env"
echo "-- code the account can write runs only in the unit's sandbox (M1, M2)"
: > "$FAKE_RUN_LOG"
hi_task 30 0 hermes /bin/sh -c 'id -un' >/dev/null 2>&1
args="$(cat "$FAKE_RUN_LOG")"
for want in --wait --collect --pipe User=yk411m Group=yk411g RuntimeMaxSec=30 PrivatePIDs=yes TemporaryFileSystem=/var/lib/yantrik-mind:ro After=yantrik-mind-egress.service Requires=yantrik-mind-egress.service; do
  has "a task is a transient unit with $want" "$want" "$args"
done
lacks "... and has no terminal of the caller's" "--pty" "$args"
passed="$(awk '/^-p$/{getline; print}' "$FAKE_RUN_LOG" | grep -vE '^(User|Group|WorkingDirectory|RuntimeMaxSec|TimeoutStopSec|After|Requires)=')"
is "the sandbox it gets is exactly the gateway unit's block (one list, shared)" "$block" "$passed"
: > "$FAKE_RUN_LOG"
( hi_run hermes model ) >/dev/null 2>&1
args="$(cat "$FAKE_RUN_LOG")"
has "hermes model gets a pty of the unit's own (M2)" "--pty" "$args"
lacks "... not the caller's pipe or terminal" "--pipe" "$args"
has "... the account's launcher, with its one word" "$(hi_home hermes)/.local/bin/hermes" "$args"
has "... and every task runs under the reaping init, not as a bare PID 1" "$BIN_DIR/yantrik-init" "$args"
has "... and a time limit and a cgroup that is collected" "--collect" "$args"
: > "$FAKE_RUN_LOG"
echo 'id -un > "$HOME/refreshed"' > "$PREFIX/share/harnesses/lib/install/refresh-plugins.sh"; chmod a+r "$PREFIX/share/harnesses/lib/install/refresh-plugins.sh"
( hi_refresh hermes ) >/dev/null 2>&1
has "the plugin refresh runs in a task too" "refresh-plugins.sh" "$(cat "$FAKE_RUN_LOG")"
: > "$FAKE_RUN_LOG"
mv "$M" "$M.real"; ln -s "$M.real" "$M"
no "a home that has become a link is refused before BindPaths could follow it" hi_task 5 0 hermes /bin/true
no "... for the checked install too" hi_check_home hermes
is "... and systemd-run was not called" "0" "$(wc -c < "$FAKE_RUN_LOG" | tr -d ' ')"
rm -f "$M"; mv "$M.real" "$M"
for f in "$here/harness-isolation.sh"; do
  lacks "no account-writable code is run by hi_run_as any more (installer, refresh, model)" "hi_run_timed 600 \"\$HI_USER\"" "$(cat "$f")"
done
has "the account's own bin is not on the PATH of the fixed tools" 'PATH="/usr/local/bin:/usr/bin:/bin"' "$(cat "$here/harness-isolation.sh")"

: > "$FAKE_RUN_LOG"
export FAKE_NFT=fail
no "a task does not run when the egress table is not loaded (L2)" hi_task 5 0 hermes /bin/true
export FAKE_NFT=ok
is "... systemd-run was not called" "0" "$(wc -c < "$FAKE_RUN_LOG" | tr -d ' ')"
mkdir -p "$HI_UNIT_DIR/hermes-gateway.service.d"; printf '[Service]\nUser=root\n' > "$HI_UNIT_DIR/hermes-gateway.service.d/x.conf"
no "a drop-in on the unit refuses the task (one list or none)" hi_task 5 0 hermes /bin/true
rm -rf "$HI_UNIT_DIR/hermes-gateway.service.d"
yes "... and without it the task runs" hi_task 5 0 hermes /bin/true
( exec 8>"$HI_LOCK"; flock -n 8; sleep 6 ) &
holder=$!; sleep 1
no "a second move is refused while one runs" bash -c "$(declare -p HI_LOCK HI_STATE HI_UNIT_DIR HI_BASE HI_USER HI_GROUP HI_DOOR HI_PROC PREFIX BIN_DIR work; declare -f); hi_install hermes"
has "... and says another is running" "another harness move is running" "$(cat "$work/said")"
kill "$holder" 2>/dev/null; wait "$holder" 2>/dev/null
printf '\xc2\x9b31m\xc2\x85x\n' | hi_relay
lacks "output relayed to the person has the UTF-8 C1 controls stripped too" "$(printf '\xc2\x9b')" "$(tail -n 1 "$work/said")"
: > "$work/log/systemctl"
( hi_abort hermes active yk411p "late" ) >/dev/null 2>&1
no "an abort after the commit does not start the person's gateway beside the moved one (L1)" grep -q -e '--user -M yk411p@ start' "$work/log/systemctl"
no "... nor turn the working system unit off" grep -q 'disable --now' "$work/log/systemctl"
echo "-- Mind and harness under one uid: /proc (H1)"
if unshare --mount --pid --fork --mount-proc true >/dev/null 2>&1; then
  h1="$work/h1"; mkdir -p "$h1"; chown yk411m:yk411g "$h1"; chmod 700 "$h1"
  sudo_m sh -c 'printf "FAKE-KEY-h1-secret\n" > "$1/secret"; chmod 600 "$1/secret"' sh "$h1"
  # The "Mind": same uid, a file open, a token in its environment.
  sudo_m env MIND_TOKEN=FAKE-KEY-h1-env sh -c 'exec 3< "$1/secret"; echo $$ > "$2"; exec sleep 100' sh "$h1" "$h1/pid" &
  n=0; while [ ! -s "$h1/pid" ] && [ "$n" -lt 50 ]; do sleep 0.1; n=$((n + 1)); done
  mindpid="$(cat "$h1/pid")"
  cat > "$work/probe.sh" <<'PROBE'
n=0
cat "/proc/$1/root/$2/secret" 2>/dev/null | grep -q FAKE-KEY-h1-secret && n=$((n + 1))
cat "/proc/$1/fd/3" 2>/dev/null | grep -q FAKE-KEY-h1-secret && n=$((n + 1))
tr '\0' '\n' < "/proc/$1/environ" 2>/dev/null | grep -q FAKE-KEY-h1-env && n=$((n + 1))
kill -0 "$1" 2>/dev/null && n=$((n + 1))
echo "$n"
PROBE
  probe() { # [pid-namespace flags...]: reads that got through, from a Hermes-like unit (its own mounts hide the file)
    unshare --mount "$@" sh -c 'mount -t tmpfs tmpfs "$3" && exec setpriv --reuid="$4" --regid="$5" --clear-groups -- sh "$1" "$2" "$3"' \
      sh "$work/probe.sh" "$mindpid" "$h1" "$(id -u yk411m)" "$(id -g yk411m)" 2>/dev/null
  }
  is "control: with only a mount namespace, every route reads the Mind (root, fd, environ, signal)" "4" "$(probe)"
  is "with a PID namespace too (PrivatePIDs=yes), no route reads or signals it" "0" "$(probe --pid --fork --mount-proc)"
  kill "$mindpid" 2>/dev/null; wait 2>/dev/null
else
  echo "  skipped: cannot make namespaces here (unshare); the VM gate runs the real unit"
fi
echo "-- the hermes model pty is not the Mind's to open (M1)"
if unshare --mount true >/dev/null 2>&1; then
  python3 -c 'import os, pty, sys, time
m, sl = pty.openpty(); n = os.ttyname(sl); os.chown(n, int(sys.argv[1]), -1); os.chmod(n, 0o620)
open(sys.argv[2], "w").write(n); time.sleep(60)' "$(id -u yk411m)" "$work/pty.name" &
  ptypid=$!; n=0; while [ ! -s "$work/pty.name" ] && [ "$n" -lt 50 ]; do sleep 0.1; n=$((n + 1)); done
  ptyname="$(cat "$work/pty.name")"
  open_pty() { setpriv --reuid="$(id -u yk411m)" --regid="$(id -g yk411m)" --clear-groups -- python3 -c 'import os, sys; os.close(os.open(sys.argv[1], os.O_RDWR | os.O_NOCTTY))' "$ptyname"; }
  yes "control: the account can open a pty slave that is its own (this is the route)" open_pty
  no "with /dev/pts hidden from the unit (emulating InaccessiblePaths=/dev/pts) it cannot" unshare --mount sh -c 'mount -t tmpfs -o mode=000 tmpfs /dev/pts && exec setpriv --reuid="$1" --regid="$2" --clear-groups -- python3 -c "import os, sys; os.close(os.open(sys.argv[1], os.O_RDWR | os.O_NOCTTY))" "$3"' sh "$(id -u yk411m)" "$(id -g yk411m)" "$ptyname"
  kill "$ptypid" 2>/dev/null; wait "$ptypid" 2>/dev/null
else
  echo "  skipped: cannot make a mount namespace here"
fi
echo "-- a rollback to a build whose Mind unit does not isolate (M2, L7)"
unset FAKE_MASK FAKE_MIND_PIDS FAKE_MIND_ACTIVE FAKE_MIND_MAINPID
yes "the Mind's unit as this build ships it counts as isolating" hi_mind_isolated
FAKE_MASK=;           no "... a unit that does not mask harness/ does not" hi_mind_isolated;  unset FAKE_MASK
FAKE_MIND_PIDS=no;    no "... nor one without PrivatePIDs" hi_mind_isolated;  unset FAKE_MIND_PIDS
FAKE_MIND_ACTIVE=1 FAKE_MIND_MAINPID=1; no "... nor a running Mind that is in PID 1's namespace (restart failed or cut short)" hi_mind_isolated
FAKE_MIND_MAINPID=4242; yes "... a running Mind in a namespace of its own does" hi_mind_isolated;  unset FAKE_MIND_ACTIVE FAKE_MIND_MAINPID
FAKE_MASK=; ( hi_ready hermes ) >/dev/null 2>&1; rc=$?; unset FAKE_MASK
is "the install refuses against such a Mind" "1" "$rc"
has "... and names why" "would be readable from it" "$(cat "$work/said")"
: > "$work/log/systemctl"; : > "$installer_calls"
FAKE_MASK=; ( hi_migrate_all ) >/dev/null 2>&1
is "an update on a machine in that state runs no installer" "0" "$(wc -l < "$installer_calls" | tr -d ' ')"
yes "... stops and disables the running gateway" grep -q 'disable --now hermes-gateway.service' "$work/log/systemctl"
no "... and does not enable or refresh anything" grep -q '^enable ' "$work/log/systemctl"
no "... the row no longer says Hermes is installed" test -e "$P/.local/state/yantrik/harnesses/hermes/installed"
has "... and the updater says its keys must be rotated" "rotate them" "$(cat "$work/said")"
unset FAKE_MASK
hi_mark_person_installed hermes "$P"
echo "-- the rest of the delta's lows"
: > "$FAKE_RUN_LOG"
FAKE_DROPIN=/etc/systemd/system/hermes-gateway.service.d/x.conf
no "a drop-in anywhere (systemd's own list) refuses a task" hi_task 5 0 hermes /bin/true
unset FAKE_DROPIN
printf '\xc2\xc2\x9b\x9b\n' | hi_relay
lacks "nested C1 bytes do not rebuild one after the strip" "$(printf '\xc2\x9b')" "$(tail -n 1 "$work/said")"
( hi_restart hermes ) >/dev/null 2>&1
has "a restart says what a stop does: running turns finish, up to 60 s" "up to 60 s" "$(cat "$work/said")"
has "... and what happens to one still running then" "interrupted" "$(cat "$work/said")"
: > "$work/log/systemctl"
HI_NOW="hermes yk411p active"; saved_state="$HI_STATE"; HI_STATE="$work/state-is-a-file"; : > "$HI_STATE"
( hi_commit hermes yk411p "$P" ) >/dev/null 2>&1; rc=$?
HI_STATE="$saved_state"
is "a marker that cannot be written ends like any other failure (L5)" "1" "$rc"
yes "... the person's gateway is given back" grep -q -e '--user -M yk411p@ start hermes-gateway.service' "$work/log/systemctl"
# (the abort set the working home aside, as it should when nothing committed: put it back for what follows)
rm -rf "$M"; mv "$M.failed" "$M"
echo "-- again, and an update on a machine that has moved"
before_files="$(find "$P" -maxdepth 2 | sort | md5sum)"
( hi_finish hermes yk411p "$P" ) >/dev/null 2>&1; ( hi_finish hermes yk411p "$P" ) >/dev/null 2>&1
is "finishing twice changes nothing" "$before_files" "$(find "$P" -maxdepth 2 | sort | md5sum)"
: > "$installer_calls"; : > "$work/log/systemctl"
echo 'id -un > "$HOME/refreshed"' > "$PREFIX/share/harnesses/lib/install/refresh-plugins.sh"
chmod a+r "$PREFIX/share/harnesses/lib/install/refresh-plugins.sh"
( hi_migrate_all ) >/dev/null 2>&1
is "an update on a moved machine runs no installer" "0" "$(wc -l < "$installer_calls" | tr -d ' ')"
is "... its plugins are refreshed, as the service account, in its own home" "yk411m" "$(cat "$M/refreshed" 2>/dev/null)"
is "... and does not stop or restart the gateway" "no" "$(grep -qE '^(restart|stop|disable) ' "$work/log/systemctl" && echo yes || echo no)"
( hi_install hermes ) >/dev/null 2>&1
is "pressing Install again works, and copies nothing from the person's home" "0" "$?"
is "... the kept copy was not copied over the live one" "yes" "$(sudo_m test -f "$M/.hermes/.env" && echo yes || echo no)"
no "no key was ever said or logged (whole run)" grep -rq FAKE-KEY "$work/said" "$work/log"

echo
if [ "$fails" = 0 ]; then echo "all selftest checks passed"; exit 0; fi
echo "$fails selftest check(s) failed"; exit 1
