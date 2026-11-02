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
export HI_UNIT_DIR="$work/units" HI_DOOR="$work/door" HI_BASE="$work/mind/harness" HI_USER=yk411m
# shellcheck source=deploy/yantrik-os/harness-isolation.sh
. "$here/harness-isolation.sh"

echo "== A. what ships"
unit="$repo/config/systemd/hermes-gateway.service"
live="$(grep -v '^[[:space:]]*#' "$unit")"
has "the gateway is a system unit of the mind account" "User=yantrik-mind" "$live"
has "... in the minds' group" "Group=yantrik-minds" "$live"
has "... with its home under the mind account's" "Environment=HOME=/var/lib/yantrik-mind/harness/hermes" "$live"
has "... reaching the desktop at the mind door" "Environment=YANTRIK_MIND_RUN=/run/yantrik-minds" "$live"
has "... held to the proxy by the kernel's table, required" "Requires=yantrik-mind-egress.service" "$live"
has "... refusing to start without that table" "nft list table inet yantrik_mind_egress" "$live"
lacks "... reading no EnvironmentFile as root" "EnvironmentFile" "$live"
lacks "... and nothing of a person's home named" "/home/" "$live"
has "... with the person's home out of its sight" "ProtectHome=yes" "$live"
has "... the Mind's own files out of its sight" "TemporaryFileSystem=/var/lib/yantrik-mind:ro" "$live"
has "... only its own directory bound back in" "BindPaths=/var/lib/yantrik-mind/harness/hermes" "$live"
has "... never gaining privileges" "NoNewPrivileges=yes" "$live"
has "... a private mask for what it writes" "UMask=0077" "$live"
has "... started by the launcher, not by a venv path that moves" "ExecStart=/var/lib/yantrik-mind/harness/hermes/.local/bin/hermes gateway run" "$live"
has "the Mind cannot see the harnesses' directory" "InaccessiblePaths=-/var/lib/yantrik-mind/harness" "$(grep -v '^[[:space:]]*#' "$repo/config/systemd/yantrik-mind.service")"
has "the unit is enabled for the system, not a login" "WantedBy=multi-user.target" "$live"
if command -v systemd-analyze >/dev/null 2>&1; then
  v="$(systemd-analyze verify --man=no "$unit" 2>&1 | grep -iE "unknown (key|section)|invalid|bad (unit|setting)|failed to parse" || true)"
  is "systemd-analyze finds no unknown or bad setting in the unit" "" "$v"
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
hi_migrated() { return 0; }
for bad in "sh" "-c" "gateway" "config" "chat" ""; do
  out="$( ( hi_run hermes $bad ) 2>&1 )"; rc=$?
  is "run hermes '$bad' is refused" "1" "$rc"
done
hi_migrated() { [ -e "$(hi_home "$1")/.yantrik-isolated" ]; }
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
printf '# %s\n' "$HI_CAPABILITY" > "$PREFIX/share/harnesses/lib/install/hermes.sh"
chmod -R a+rX "$work/opt" 2>/dev/null

P="$work/home/yk411p"
sudo_p() { setpriv --reuid="$(id -u yk411p)" --regid="$(id -g yk411p)" --clear-groups -- "$@"; }
sudo_m() { setpriv --reuid="$(id -u yk411m)" --regid="$(id -g yk411m)" --clear-groups -- "$@"; }
sudo_p sh -c 'umask 077; mkdir -p "$1/.hermes/sessions" "$1/.hermes/hermes-agent" "$1/.hermes/plugins/yantrik" "$1/.local/share/yantrik/memory" "$1/.local/bin" "$1/.config/systemd/user"
  printf "model: x\n" > "$1/.hermes/config.yaml"
  printf "OPENAI_API_KEY=FAKE-KEY-yk411-not-a-secret\n" > "$1/.hermes/.env"
  printf "FAKE-CANARY-yk411\n" > "$1/notes.txt"
  head -c 4096 /dev/urandom > "$1/.hermes/state.db"; printf wal > "$1/.hermes/state.db-wal"
  printf s1 > "$1/.hermes/sessions/a.json"; printf code > "$1/.hermes/hermes-agent/run.py"
  printf mem > "$1/.local/share/yantrik/memory/hermes.db"
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
out="$(hi_run_timed 2 yk411m "$work/mind" sh -c 'printf "line one[31m red
"; sleep 60' 2>&1)"; rc=$?
is "a harness that will not stop is ended at its time limit" "yes" "$([ "$rc" != 0 ] && [ $((SECONDS - t0)) -lt 20 ] && echo yes || echo no)"
has "... what it printed is relayed (to info)" "line one" "$(cat "$work/said")"
lacks "... without terminal escapes" "$(printf '')" "$(cat "$work/said")"
is "it has no terminal of the caller's (own session)" "yes" "$(hi_as_mind sh -c 'ps -o sid= -p $$' | awk -v me="$(ps -o sid= -p $$ | tr -d ' ')" '{print ($1 != me) ? "yes" : "no"}')"

echo "-- the copy"
mkdir -p "$work/log"; : > "$work/log/systemctl"
systemctl() { printf '%s\n' "$*" >> "$work/log/systemctl"; case "$*" in *--user*is-active*) [ -e "$work/log/user-active" ] ;; *) true ;; esac; }
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
is "the person's own copy is untouched" "$before" "$(python3 -I -c "$HI_PY_TREE" digest "$P" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"
no "no key was ever said or logged" grep -rq FAKE-KEY "$work/said" "$work/log"

echo "-- setting aside, and doing it again"
hi_set_aside hermes earlier
yes "a retry keeps the earlier directory, it does not delete it" sh -c "ls -d '$M'.earlier-* >/dev/null"
yes "... and starts with an empty one" test -z "$(sudo_m ls -A "$M")"
hi_copy_data hermes yk411p "$P" && yes "the copy can be made again" true
hi_set_aside hermes earlier; hi_set_aside hermes earlier
is "setting aside an empty directory is a no-op" "2" "$(ls -d "$M".earlier-* | wc -l | tr -d ' ')"

echo "-- the whole install, with a stand-in installer"
rm -rf "$M" "$M".earlier-*
hi_ready hermes
installer_calls="$work/log/installer"; : > "$installer_calls"
hi_run_installer() { # <name> <phase> <secs>: a stand-in that makes what the real one makes
  printf '%s\n' "$2" >> "$installer_calls"
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
is "... it is not marked as moved" "no" "$([ -e "$M/.yantrik-isolated" ] && echo yes || echo no)"
yes "... the system unit was turned off" grep -q 'disable --now hermes-gateway.service' "$work/log/systemctl"
yes "... the person's own gateway is running again" grep -q -e '--user -M yk411p@ start hermes-gateway.service' "$work/log/systemctl"
yes "... what was half made is kept aside, not deleted" sh -c "ls -d '$M'.failed-* >/dev/null"
has "... and the reason is named" "its installer stopped" "$(cat "$work/said")"
is "... and the person's copy is the same" "$before" "$(python3 -I -c "$HI_PY_TREE" digest "$P" .hermes "$(hi_code_dirs hermes)" "$(hi_person_extra hermes)")"

STAND_IN_FAILS=prove
( hi_install hermes ) >/dev/null 2>&1; rc=$?
is "a Hermes that starts but fails its proofs is not committed" "1" "$rc"
is "... and the person's is still in place" "yes" "$([ -f "$P/.hermes/config.yaml" ] && echo yes || echo no)"

STAND_IN_FAILS=
: > "$work/log/systemctl"
( hi_install hermes ) >/dev/null 2>&1; rc=$?
is "the install that works succeeds" "0" "$rc"
is "the phases ran in order, the unit between them" "setup prove" "$(grep -E '^(setup|prove)$' "$installer_calls" | tail -2 | tr '\n' ' ' | sed 's/ $//')"
yes "the person's gateway was stopped before the copy" grep -q -e '--user -M yk411p@ stop hermes-gateway.service' "$work/log/systemctl"
yes "the system unit was enabled and restarted" grep -q 'enable hermes-gateway.service' "$work/log/systemctl"
yes "it is marked as moved, by the service account" test "$(stat -c %U "$M/.yantrik-isolated")" = yk411m
is "the person's old copy is kept, whole, under its new name" "$hashes" "$(sudo_p sh -c 'cd "$1/.hermes.yantrik-moved" && sha256sum config.yaml .env state.db sessions/a.json' sh "$P")"
no "the old ~/.hermes is gone from the person's home" test -e "$P/.hermes"
yes "the Hermes memory file was kept beside it" sh -c "ls '$P'/.hermes.yantrik-moved/outside-home/hermes.db >/dev/null"
yes "the old launcher is kept and the person's hermes is the shim" grep -q yantrik-harness-shim "$P/.local/bin/hermes"
yes "... the old launcher is in the kept directory" grep -q 'old launcher' "$P/.hermes.yantrik-moved/outside-home/launcher"
is "the old user unit is masked" "/dev/null" "$(readlink "$P/.config/systemd/user/hermes-gateway.service")"
yes "... and the file itself is kept" test -f "$P/.hermes.yantrik-moved/hermes-gateway.service"
yes "Settings reads it as installed" test -f "$P/.local/state/yantrik/harnesses/hermes/installed"
no "the person's account cannot read what moved" sudo_p cat "$M/.hermes/.env"

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
is "... and does not stop or restart the gateway" "no" "$(grep -qE 'restart|stop' "$work/log/systemctl" && echo yes || echo no)"
( hi_install hermes ) >/dev/null 2>&1
is "pressing Install again works, and copies nothing from the person's home" "0" "$?"
is "... the kept copy was not copied over the live one" "yes" "$(sudo_m test -f "$M/.hermes/.env" && echo yes || echo no)"
no "no key was ever said or logged (whole run)" grep -rq FAKE-KEY "$work/said" "$work/log"

echo
if [ "$fails" = 0 ]; then echo "all selftest checks passed"; exit 0; fi
echo "$fails selftest check(s) failed"; exit 1
