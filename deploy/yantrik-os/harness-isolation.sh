#!/bin/bash
# Sourced, not run: harnesses that run as the mind account instead of as the person (#411).
#
# yantrik-update sources this beside itself (like mind-capabilities.sh) and calls it from
# `harness`, `migrate-harnesses` and `reconcile`. A harness (Hermes now; Pi, OpenClaw and DeepSeek
# next) used to be a `systemctl --user` unit of the person's, so its shell and file tools could read
# everything the person can. It is now a system unit of the mind account, like yantrik-mind.service:
#
#   - its home is /var/lib/yantrik-mind/harness/<name>, which the person cannot open. The Mind
#     shares its uid, so the two are kept apart by mount AND PID namespaces together: the unit hides
#     the Mind's files (TemporaryFileSystem) and PrivatePIDs=yes closes /proc/<pid>/{root,fd,environ}.
#     Both units have the line, so it holds both ways. What a stop does under it is measured in the
#     units' comments (a handler is waited for on 257.13; the Mind, with none, takes its 30 s
#     TimeoutStopSec), and Hermes drains itself (agent.restart_drain_timeout). A systemd older than 257
#     would ignore the line, so no harness is installed or started on one (harness-sandbox.sh); the
#     Mind's unit has no gate of its own, because refusing to start would be no Mind. A rollback to a
#     build whose Mind unit has neither is caught too (hi_mind_isolated, hi_guard_mixed);
#   - it reaches the desktop only at the mind door (/run/yantrik-minds, YANTRIK_MIND_RUN), where
#     every act is held to the shell's grades and approvals;
#   - its installer is the same script the Install button always ran, run as the mind account with
#     that home (harnesses/lib/install/<name>.sh, YANTRIK_HARNESS_ISOLATED=setup then =prove),
#     because everything in it is relative to $HOME; only the user unit is replaced.
#
# Root never works on a path someone else controls. The person's files are read as the person; the
# mind account's are written, by fixed system tools (hi_as_mind), as the mind account; anything the
# account can write and root would run (the installer, the refresh, `hermes model`) runs only in a
# transient unit with the gateway unit's own sandbox properties (hi_task), never outside it.
#
# A new harness is: one line in hi_known/hi_unit, an installer that understands
# YANTRIK_HARNESS_ISOLATED (hermes.sh is the example), and its unit in config/systemd.
#
# Needs from yantrik-update: PREFIX, say info warn fail, tree_is_roots, desktop_owner, and the
# mind account's own reconcile_* (the account, the door, the egress table, the units).

# These paths are root's. They are read from the environment only under the selftest (YANTRIK_SELFTEST=1),
# so nothing a caller exports can steer a root-run verb; sudo's env_reset was the only thing that did.
hi_knob() { if [ "${YANTRIK_SELFTEST:-}" = 1 ]; then printf '%s' "${!1:-$2}"; else printf '%s' "$2"; fi; }
HI_USER="$(hi_knob HI_USER yantrik-mind)"
HI_BASE="$(hi_knob HI_BASE /var/lib/yantrik-mind/harness)"
HI_DOOR="$(hi_knob HI_DOOR /run/yantrik-minds)"
HI_PROXY="$(hi_knob HI_PROXY http://127.0.0.1:7450)"
HI_UNIT_DIR="$(hi_knob HI_UNIT_DIR /etc/systemd/system)"
HI_GROUP="$(hi_knob HI_GROUP yantrik-minds)"
# Root's own record of what has moved. Not in the account's home, which the harness can write.
HI_STATE="$(hi_knob HI_STATE /var/lib/yantrik-harness)"
# PrivatePIDs= (the unit's PID namespace) is systemd 257's; an older systemd would ignore the line
# in a unit, so nothing is installed or started without it.
HI_MIN_SYSTEMD="$(hi_knob HI_MIN_SYSTEMD 257)"
HI_COPY_TIMEOUT="$(hi_knob HI_COPY_TIMEOUT 1200)"
HI_LOCK="$(hi_knob HI_LOCK /run/yantrik-harness.lock)"
HI_PROC="$(hi_knob HI_PROC /proc)"
# What a person who migrated keeps: their old Hermes, whole, renamed. Never deleted by this.
HI_KEEP=".yantrik-moved"
# The installer carries this word when it understands the two phases. An older installer (a rolled
# back build's share/) is never run as the mind account: it would try to make a user unit.
HI_CAPABILITY="YANTRIK_HARNESS_ISOLATED"

# shellcheck source=deploy/yantrik-os/harness-sandbox.sh
. "$(dirname "${BASH_SOURCE[0]}")/harness-sandbox.sh" || return 1
# shellcheck source=deploy/yantrik-os/harness-isolation-room.sh
. "$(dirname "${BASH_SOURCE[0]}")/harness-isolation-room.sh" || return 1
# A missing helper must refuse the move, never skip its checks ("command not found" is not a failure).
hi_parts_ok() { type hi_abort hi_room_check hi_task hi_discard_unfinished hi_auto_blocked >/dev/null 2>&1; }

hi_known() { case "$1" in hermes) return 0 ;; *) return 1 ;; esac; }
hi_unit() { case "$1" in hermes) printf 'hermes-gateway.service' ;; esac; }
hi_home() { printf '%s/%s' "$HI_BASE" "$1"; }
hi_installer() { printf '%s/share/harnesses/lib/install/%s.sh' "$PREFIX" "$1"; }
hi_plugin_dir() { printf '%s/share/harnesses/%s' "$PREFIX" "$1"; }
# What the person's home holds of a harness: the directory its code and data are in.
hi_person_dir() { case "$1" in hermes) printf '.hermes' ;; esac; }
# Every directory that moves (colon-separated): the harness's own, and the embedder model its memory
# engine downloaded once and cached (a public model, copied so the account does not need the network
# to remember: the proof "Can it remember?" failed without it on a real machine).
hi_person_tops() { case "$1" in hermes) printf '.hermes:.cache/yantrikdb/models' ;; esac; }
# The names that are Hermes's code, rebuilt by the installer from the pinned commit and the image's
# wheels, with absolute paths inside that would not survive a move. Everything else in .hermes
# (config, keys, state.db and its WAL, sessions, memories, skills, cron, third-party plugins) moves.
hi_code_dirs() {
  case "$1" in hermes) printf '%s' '.hermes/hermes-agent:.hermes/tools:.hermes/installs:.hermes/bin:.hermes/venv:.hermes/node_modules:.hermes/.cache:.hermes/.yantrik-staging:.hermes/plugins/yantrik:.hermes/plugins/yantrikdb:.hermes/gateway_state.json:.hermes/gateway.pid:.hermes/gateway.lock' ;; esac
}
# Files outside that directory the harness's own data lives in (globs, relative to the home).
hi_person_extra() { case "$1" in hermes) printf '%s' '.local/share/yantrik/memory/hermes.db*' ;; esac; }

# ── Who runs what ───────────────────────────────────────────────────────────────────────

hi_ids() { # <user>: "uid gid", refused for root and for a name that is not an account
  local uid gid
  uid="$(id -u "$1" 2>/dev/null)" && gid="$(id -g "$1" 2>/dev/null)" || return 1
  case "$uid:$gid" in 0:*|*:0|*[!0-9:]*) return 1 ;; esac
  printf '%s %s' "$uid" "$gid"
}

# <user> <home> <command...>: a FIXED SYSTEM TOOL as that account, clean environment, PATH without
# the account's own bin (it could shadow tar or python). Never for code the account can write: that
# is hi_task. No terminal of
# the caller's (setsid; a controlling terminal could be written to with TIOCSTI after we are gone),
# no supplementary groups, no new privileges. The mind's proxy variables are set for the mind.
hi_run_as() {
  local user="$1" home="$2" ids uid gid; shift 2
  ids="$(hi_ids "$user")" || { warn "no account $user"; return 1; }
  uid="${ids% *}"; gid="${ids#* }"
  local -a env_args=(HOME="$home" USER="$user" LOGNAME="$user" LANG=C.UTF-8
    PATH="/usr/local/bin:/usr/bin:/bin" TERM="${TERM:-dumb}")
  if [ "$user" = "$HI_USER" ]; then
    env_args+=(YANTRIK_MIND_RUN="$HI_DOOR" HTTPS_PROXY="$HI_PROXY" https_proxy="$HI_PROXY"
      HTTP_PROXY="$HI_PROXY" http_proxy="$HI_PROXY" ALL_PROXY="$HI_PROXY" all_proxy="$HI_PROXY"
      NO_PROXY=127.0.0.1,localhost,::1 no_proxy=127.0.0.1,localhost,::1)
  fi
  local -a limit=()
  [ -z "${HI_TIMEOUT:-}" ] || limit=("$(command -v timeout)" -k 10 "$HI_TIMEOUT")
  ( cd "$home" 2>/dev/null || cd / ; exec setsid -w "${limit[@]}" "$(command -v setpriv)" --reuid="$uid" --regid="$gid" \
      --clear-groups --no-new-privs -- env -i "${env_args[@]}" "$@" ) 8>&- 9>&-
}
hi_as_mind() { hi_run_as "$HI_USER" "$HI_BASE" "$@"; }
hi_as_owner() { # <command...>
  local owner home
  owner="$(desktop_owner)"; home="$(getent passwd "$owner" | cut -d: -f6)"
  hi_run_as "$owner" "$home" "$@"
}

# What a script says, one line each, for a person's row or a terminal: control characters dropped
# (the text is a program's, written to a terminal root owns).
hi_relay() { tr -d '\000-\010\013-\037\177' | LC_ALL=C sed ':a;s/\xc2[\x80-\x9f]//;ta' | while IFS= read -r line; do info "$line"; done; }

# <seconds> <user> <home> <command...>: a fixed tool, output relayed, exit status returned, ended
# by KILL at the limit.
hi_run_timed() {
  local secs="$1" rc; shift
  HI_TIMEOUT="$secs" hi_run_as "$@" 2>&1 </dev/null | hi_relay
  rc=${PIPESTATUS[0]}
  return "$rc"
}

# ── The person's side ───────────────────────────────────────────────────────────────────

hi_owner_home() { # prints "<owner> <home>" or fails
  local owner home
  owner="$(desktop_owner)"
  case "$owner" in ''|root|*[!a-z0-9_-]*) return 1 ;; esac
  home="$(getent passwd "$owner" | cut -d: -f6)"
  [ -n "$home" ] && [ -d "$home" ] || return 1
  printf '%s %s' "$owner" "$home"
}

# Has this person a harness of their own to move? (Their directory, not a link.)
hi_person_has() { # <name> <home>
  local d="$2/$(hi_person_dir "$1")"
  [ -d "$d" ] && [ ! -L "$d" ] && [ -f "$d/config.yaml" ]
}

hi_migrated() { [ -f "$HI_STATE/$1.isolated" ] && [ ! -L "$HI_STATE/$1.isolated" ]; }
hi_mark_migrated() { # <name>: root's file, written whole
  install -d -m 0755 -o root -g root "$HI_STATE" 2>/dev/null || [ -d "$HI_STATE" ] || return 1
  printf 'isolated %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$HI_STATE/$1.isolated.new" \
    && chmod 0644 "$HI_STATE/$1.isolated.new" && mv -f "$HI_STATE/$1.isolated.new" "$HI_STATE/$1.isolated" \
    && hi_clear_failure "$1"
}

# The person's old gateway, off, for the person and for everyone the global enable reached, and
# held off: a masked user unit cannot be started by `hermes gateway install` or by hand. The unit
# file is moved into the kept directory, not deleted.
hi_stop_person_unit() { # <name> <owner> <home>: succeeds with "active" or "idle" on stdout
  local unit state=idle; unit="$(hi_unit "$1")"
  if systemctl --user -M "$2@" is-active --quiet "$unit" 2>/dev/null; then state=active; fi
  systemctl --user -M "$2@" stop "$unit" >/dev/null 2>&1 || true
  printf '%s' "$state"
}
hi_disable_person_unit() { # <name> <owner> <home>
  local unit file; unit="$(hi_unit "$1")"; file="$3/.config/systemd/user/$unit"
  systemctl --user -M "$2@" stop "$unit" >/dev/null 2>&1 || true
  systemctl --user -M "$2@" disable "$unit" >/dev/null 2>&1 || true
  systemctl --global disable "$unit" >/dev/null 2>&1 || true
  if [ -e "$file" ] && [ ! -L "$file" ]; then
    hi_as_owner sh -c 'mkdir -p "$2" && mv "$1" "$2/$3"' sh "$file" "$3/$(hi_person_dir "$1")$HI_KEEP" "$unit" \
      || warn "could not move $file aside"
  fi
  # A mask is a link to /dev/null, which is what `systemctl --user mask` writes.
  hi_as_owner sh -c 'mkdir -p "$(dirname "$1")" && ln -sfn /dev/null "$1"' sh "$file" \
    || warn "could not mask $unit for $2"
  systemctl --user -M "$2@" daemon-reload >/dev/null 2>&1 || true
}

# The person's old copy, kept. Renamed once to .hermes.yantrik-moved (with the memory file and the
# old launcher inside), so a second run leaves it alone; nothing is deleted.
hi_keep_person_copy() { # <name> <home>
  local d keep; d="$(hi_person_dir "$1")"; keep="$2/$d$HI_KEEP"
  [ -d "$2/$d" ] && [ ! -L "$2/$d" ] || return 0
  [ ! -e "$keep" ] || { warn "$keep is already there; leaving $2/$d where it is"; return 0; }
  hi_as_owner sh -c '
    set -e; home="$1"; dir="$2"; keep="$3"
    mv "$home/$dir" "$keep.new"
    mkdir -p "$keep.new/outside-home"
    for f in "$home"/.local/share/yantrik/memory/hermes.db*; do
      [ -e "$f" ] && mv "$f" "$keep.new/outside-home/"
    done
    mv "$keep.new" "$keep"
  ' sh "$2" "$d" "$keep" && info "your Hermes, as it was, is kept in $keep (nothing deleted). Its API keys are still in it, readable by anything running as you; delete the folder when you no longer want that copy" \
    || warn "could not move $2/$d aside; it stays where it is"
}

# The person's `hermes` command: what is left of it asks the mind account's Hermes, for the few
# things a person does by hand. The old launcher is kept beside the old copy. Not Hermes's own
# `gateway` commands: a second gateway is exactly what this prevents.
hi_shim_text() {
  printf '%s\n' '#!/bin/sh' \
    '# yantrik-harness-shim: written by yantrik-update (#411). Hermes now runs as the mind account, out' \
    '# of reach of your files; this runs the few commands you need against it.' \
    'case "${1:-}" in' \
    "    model|status|--version) exec sudo -n $BIN_DIR/yantrik-update harness run hermes \"\$@\" ;;" \
    'esac' \
    'echo "hermes: Hermes runs as the Yantrik mind account now, so this command does only: model, status, --version." >&2' \
    'echo "Its gateway is the system service hermes-gateway (sudo yantrik-update harness restart hermes)." >&2' \
    'exit 2'
}
hi_write_shim() { # <name> <home>
  local bin="$2/.local/bin/$1" keep="$2/$(hi_person_dir "$1")$HI_KEEP"
  # Read as the person, whose path it is: root does not open a file the person can link.
  if hi_as_owner sh -c '[ -f "$1" ] && grep -q yantrik-harness-shim "$1"' sh "$bin" 2>/dev/null; then return 0; fi
  hi_as_owner sh -c '
    set -e; bin="$1"; keep="$2"; shim="$3"
    mkdir -p "$(dirname "$bin")"
    if [ -e "$bin" ] || [ -L "$bin" ]; then mkdir -p "$keep/outside-home"; mv "$bin" "$keep/outside-home/launcher"; fi
    printf "%s\n" "$shim" > "$bin.new"; chmod 755 "$bin.new"; mv "$bin.new" "$bin"
  ' sh "$bin" "$keep" "$(hi_shim_text)" || warn "could not leave a hermes command in $2/.local/bin"
}

# What Settings reads to show a harness as installed: the person's own state file, written only
# after the proofs passed under the mind account.
hi_mark_person_installed() { # <name> <home>
  hi_as_owner sh -c 'umask 077; d="$1/.local/state/yantrik/harnesses/$2"; mkdir -p "$d" && date -u +%Y-%m-%dT%H:%M:%SZ > "$d/installed"' \
    sh "$2" "$1" || warn "could not record in $2 that $1 is installed"
}

# ── The mind account's side ─────────────────────────────────────────────────────────────

# The account, the door, the egress table and the units: the mind's own reconcile steps, not a copy.
hi_ready() { # <name>
  local unit; unit="$(hi_unit "$1")"
  hi_systemd_ok || fail "this machine's systemd is older than $HI_MIN_SYSTEMD, which a harness needs to be kept apart from the Mind (PrivatePIDs=); $1 stays as it was"
  reconcile_minds
  reconcile_mind_egress_units
  reconcile_mind_units
  reconcile_egress
  reconcile_mind_egress
  hi_mind_isolated || fail "the Mind's unit, as loaded or as running, does not hide $HI_BASE or have its own PID namespace (a build from before this release does not): $1 would be readable from it. Run: sudo yantrik-update reconcile"
  getent passwd "$HI_USER" >/dev/null 2>&1 || fail "there is no mind account ($HI_USER); nothing was changed"
  [ -f "$HI_UNIT_DIR/$unit" ] || fail "this build ships no $unit; nothing was changed"
  [ -d "$HI_DOOR" ] || fail "the mind door ($HI_DOOR) is not there; run: sudo yantrik-update reconcile"
  hi_mask_dir || fail "could not make $HI_BASE as $HI_USER; nothing was changed"
  hi_as_mind sh -c 'umask 077; mkdir -p "$1"' sh "$(hi_home "$1")" \
    || fail "could not make $(hi_home "$1") as $HI_USER"
  hi_check_home "$1" || fail "$(hi_home "$1") is not the account's own directory; nothing was changed"
}

# Copy the person's Hermes data into the mind account's home, as the person (reading) and as the
# mind (writing, umask 077), then compare count and hash on both sides. The person's copy is not
# touched; if the numbers differ, nothing is used.
hi_copy_data() { # <name> <owner> <home>
  local name="$1" skip extra top dest before after
  skip="$(hi_code_dirs "$name")"; extra="$(hi_person_extra "$name")"; top="$(hi_person_tops "$name")"; dest="$(hi_home "$name")"
  before="$(HI_TIMEOUT=$HI_COPY_TIMEOUT hi_as_owner python3 -I -c "$HI_PY_TREE" digest "$3" "$top" "$skip" "$extra")" || return 1
  # The producer runs as the person and the consumer as the mind account; tar on the consumer side
  # refuses absolute names and `..`, and everything it makes is the mind account's.
  { HI_TIMEOUT=$HI_COPY_TIMEOUT hi_as_owner sh -c 'cd "$1" && python3 -I -c "$2" list "$1" "$3" "$4" "$5" | tar --null --no-recursion -T - -cf -' \
      sh "$3" "$HI_PY_TREE" "$top" "$skip" "$extra" ; } \
    | HI_TIMEOUT=$HI_COPY_TIMEOUT hi_as_mind sh -c 'umask 077; cd "$1" && tar -xf -' sh "$dest" || return 1
  after="$(HI_TIMEOUT=$HI_COPY_TIMEOUT hi_as_mind python3 -I -c "$HI_PY_TREE" digest "$dest" "$top" "$skip" "$extra")" || return 1
  [ "$before" = "$after" ] || { warn "the copy of $name differs from its source (${before%% *} files, then ${after%% *})"; return 1; }
  info "${before%% *} files of your $name moved to its own account and counted twice"
}

hi_has_installer() { # <name>: the shipped installer understands the two phases
  local f; f="$(hi_installer "$1")"
  [ -f "$f" ] && grep -q "$HI_CAPABILITY" "$f"
}

hi_run_installer() { # <name> <setup|prove> <seconds>
  hi_task_relay "$3" "$1" /usr/bin/env "$HI_CAPABILITY=$2" /bin/sh "$(hi_installer "$1")" "$(hi_plugin_dir "$1")"
}

hi_unit_wait() { # <unit> <seconds>: active for two checks in a row
  local n=0
  while [ "$n" -lt "$2" ]; do
    systemctl is-active --quiet "$1" 2>/dev/null && { sleep 2; systemctl is-active --quiet "$1" 2>/dev/null && return 0; }
    sleep 1; n=$((n + 1))
  done
  return 1
}

# What a kill or a stop of the move does: the same as a failure, so no interrupted move leaves the
# system gateway running beside the person's.
HI_NOW=""
hi_interrupted() { local n o w; read -r n o w <<< "$HI_NOW"; hi_abort "$n" "$w" "$o" "the move was interrupted"; }

# The point of no return, all of it repeatable: root's commit marker (outside anything the harness
# can write), then the system unit enabled for boot, the person's old gateway off and masked, their
# copy kept, a `hermes` command that works, and the row's installed marker. Any run after the first
# only repeats what is missing.
hi_commit() { # <name> <owner> <home>
  trap - TERM INT HUP
  # Not committed until the marker is written: a failure here gives the person's gateway back.
  hi_mark_migrated "$1" || hi_abort "$1" "${HI_NOW##* }" "$2" "could not record that it moved"
  systemctl enable "$(hi_unit "$1")" >/dev/null 2>&1 || warn "$(hi_unit "$1") could not be enabled for boot; sudo yantrik-update reconcile tries again"
  hi_finish "$1" "$2" "$3"
  hi_mark_person_installed "$1" "$3"
}
hi_finish() { # <name> <owner> <home>
  # The copy first: the unit file and the launcher are kept inside it.
  hi_keep_person_copy "$1" "$3"
  hi_disable_person_unit "$1" "$2" "$3"
  hi_write_shim "$1" "$3"
}

# ── Commands ────────────────────────────────────────────────────────────────────────────

# The Install button, and a machine whose Hermes is still the person's: do everything, in the order
# that lets any failure or interruption leave the person's own Hermes as it was. The unit is started
# but not enabled until the proofs pass, so a move cut short never leaves two gateways for the
# next boot.
hi_install() { # <name>
  local name="$1" unit who owner home was=none shown
  hi_known "$name" || fail "unknown harness '$name' (known here: hermes)"
  [ "$(id -u)" = 0 ] || fail "run as root"
  hi_parts_ok || fail "this install is missing a harness-isolation helper (an update that did not finish?); nothing was changed. Run: sudo yantrik-update"
  tree_is_roots || fail "this machine's tree is not root's yet; run: sudo yantrik-update migrate-ownership"
  # One move at a time: the button and the detached update-time move would interleave otherwise.
  [ -n "${HI_LOCKED:-}" ] || { exec 8>"$HI_LOCK" && flock -n 8 && HI_LOCKED=1; }     || fail "another harness move is running (journalctl -u yantrik-harness-migrate); try again when it ends"
  hi_has_installer "$name" || fail "this build's $name installer does not know the mind account yet; nothing was changed"
  unit="$(hi_unit "$name")"
  who="$(hi_owner_home)" || fail "cannot tell whose desktop this is; nothing was changed"
  owner="${who% *}"; home="${who#* }"
  say "Setting $name up under the mind account"
  hi_room_check "$name" "$home" "$(hi_person_has "$name" "$home" && ! hi_migrated "$name" && echo yes || echo no)"
  hi_ready "$name"
  if hi_person_has "$name" "$home" && ! hi_migrated "$name"; then
    was="$(hi_stop_person_unit "$name" "$owner" "$home")"
    HI_NOW="$name $owner $was"
    trap hi_interrupted TERM INT HUP
    hi_discard_unfinished "$name" "left by an earlier try" || hi_abort "$name" "$was" "$owner" "an earlier try's files could not be removed"
    hi_copy_data "$name" "$owner" "$home" || hi_abort "$name" "$was" "$owner" "its files could not be copied and counted"
  fi
  HI_NOW="$name $owner $was"
  trap hi_interrupted TERM INT HUP
  hi_run_installer "$name" setup 1500 || hi_abort "$name" "$was" "$owner" "its installer stopped (the lines above say why)"
  systemctl daemon-reload >/dev/null 2>&1 || true
  systemctl restart "$unit" || hi_abort "$name" "$was" "$owner" "$unit would not start (journalctl -u $unit)"
  hi_unit_wait "$unit" 60 || hi_abort "$name" "$was" "$owner" "$unit did not stay up (journalctl -u $unit)"
  # The PID namespace is what keeps the Mind's and the harness's processes apart; a systemd that
  # ignored the line would run the harness without it. Asked of systemd, not assumed.
  shown="$(systemctl show -p PrivatePIDs --value "$unit" 2>/dev/null)"
  [ "$shown" = yes ] || hi_abort "$name" "$was" "$owner" "$unit is running without a PID namespace of its own (PrivatePIDs is '${shown:-unknown}')"
  hi_pid_ns_private "$unit" || hi_abort "$name" "$was" "$owner" "$unit's main process shares PID 1's PID namespace"
  hi_run_installer "$name" prove 200 || hi_abort "$name" "$was" "$owner" "it started but did not pass its proofs"
  hi_commit "$name" "$owner" "$home"
  say "$name runs as the mind account now"
}

# The plugins in the mind account's home brought up to this build's, by the same script that does it
# for a person's home (harnesses/lib/install/refresh-plugins.sh), run as the mind account. Not
# restarting the gateway: a turn may be in flight (the script says why); it loads them when it next
# starts. An installer that does not know the mind account (a rolled-back build) is left alone.
hi_refresh() { # <name>
  local script="$PREFIX/share/harnesses/lib/install/refresh-plugins.sh"
  hi_has_installer "$1" && [ -f "$script" ] || return 0
  hi_task_relay 600 "$1" /bin/sh "$script" || warn "could not refresh $1's plugins; the ones in place are kept"
}

# After a rollback to a build from before this release, the Mind's unit no longer hides a harness or
# has a PID namespace. Nothing the harness does protects its keys at rest from that Mind, so the
# gateway is stopped and disabled, the row stops saying installed, and the updater says what is left
# to do. (The older unit lands at a reconcile; this runs right after it, in the same one.)
hi_guard_mixed() { # <owner> <home>
  local name
  for name in hermes; do
    hi_migrated "$name" && ! hi_mind_isolated || continue
    systemctl disable --now "$(hi_unit "$name")" >/dev/null 2>&1 || true
    hi_as_owner rm -f "$2/.local/state/yantrik/harnesses/$name/installed"
    warn "$name's gateway was stopped and disabled: the Mind's unit no longer hides its files or has its own PID namespace (an older build's). Its API keys at rest are readable by that Mind: rotate them. Update to this release or later, then press Install."
  done
}

# Every known harness that is the person's, or already moved: moved, or finished and refreshed.
# From `yantrik-update migrate-harnesses`, and from reconcile (detached, because a first move
# downloads the harness).
hi_migrate_all() {
  local name who owner home
  [ "$(id -u)" = 0 ] || return 0
  tree_is_roots || return 0
  who="$(hi_owner_home)" || return 0
  owner="${who% *}"; home="${who#* }"
  hi_guard_mixed "$owner" "$home"
  for name in hermes; do
    if hi_migrated "$name" && hi_mind_isolated; then
      hi_finish "$name" "$owner" "$home"
      hi_refresh "$name"
      systemctl is-enabled --quiet "$(hi_unit "$name")" 2>/dev/null || systemctl enable --now "$(hi_unit "$name")" >/dev/null 2>&1 || true
      continue
    fi
    hi_person_has "$name" "$home" || continue
    hi_has_installer "$name" || { info "$name stays the person's for now: this build's installer is older than the mind account"; continue; }
    hi_install "$name"
  done
}

hi_restart() { # <name>
  hi_known "$1" || fail "unknown harness '$1'"
  hi_migrated "$1" || fail "$1 is not under the mind account here; nothing to restart"
  [ "$(id -u)" = 0 ] || fail "run as root"
  info "Hermes is stopped by letting its running turns finish, up to 60 s (scheduled jobs 50 s); a turn still running then is interrupted, and Hermes tells the person and marks the session to resume"
  systemctl restart "$(hi_unit "$1")" || fail "$(hi_unit "$1") would not restart (journalctl -u $(hi_unit "$1"))"
  info "$(hi_unit "$1") restarted"
}

# The updater's own call after it has put a new bridge (yos-mcp) in place: Hermes drains and
# restarts itself (ExecReload sends it SIGUSR1), so a turn in flight is not cut. The person's
# `check` reaches this through the updater's sudo rule (yantrik-harness-bridge.sh).
hi_reload() { # <name>
  hi_known "$1" || fail "unknown harness '$1'"
  hi_migrated "$1" || fail "$1 is not under the mind account here; nothing to reload"
  [ "$(id -u)" = 0 ] || fail "run as root"
  type harness_bridge_signal >/dev/null 2>&1 || fail "yantrik-harness-bridge.sh is not beside the updater"
  harness_bridge_signal reload || fail "$(hi_unit "$1") would not reload (journalctl -u $(hi_unit "$1"))"
  info "$(hi_unit "$1") asked to finish its turns and restart"
}

# "Use Yantrik models" for a harness that has moved (Hermes, #411): its config is in the account's
# home, which the person's desktop cannot write, so the desktop sends the model settings here, on
# stdin (never an argument: they hold the gateway token), and the account's own tool writes them
# inside the gateway's sandbox (hermes_model.py: only the `model` keys, only a local address).
# Then Hermes drains and restarts onto them. `revert` puts its earlier model section back.
# Hermes can rewrite its own config.yaml (its `model` and `update` commands write the whole file, and a
# terminal toolset on another platform can edit it), so the allowlist is re-asserted by the account's
# own tool (hermes-guard.sh: hermes_config.py reassert) after anything that could have changed it, and
# before every gateway start (the unit's ExecStartPre=). 0 it held, 3 it had been widened and is put
# right, else it could not be put right: then the gateway is stopped, not reloaded onto it.
hi_guard_toolsets() { # <name>: sets nothing; returns the tool's status
  local name="$1" dir rc
  dir="$(dirname "$(hi_installer "$name")")"
  [ -f "$dir/$name-guard.sh" ] || { warn "this build has no $name-guard.sh: $name's toolset allowlist was not re-checked"; return 1; }
  hi_task 300 0 "$name" /bin/sh "$dir/$name-guard.sh" after </dev/null 2>&1 | hi_relay
  rc="${PIPESTATUS[0]}"
  [ "$rc" = 0 ] || [ "$rc" = 3 ] || systemctl stop "$(hi_unit "$name")" >/dev/null 2>&1 || true
  return "$rc"
}

hi_model() { # <name> <apply|revert>; the settings on stdin for apply
  local name="${1:-}" op="${2:-}" body="" dir
  hi_known "$name" || fail "usage: yantrik-update harness model hermes apply|revert"
  case "$op" in apply|revert) ;; *) fail "usage: yantrik-update harness model $name apply|revert" ;; esac
  hi_migrated "$name" || fail "$name is not under the mind account here; nothing to set"
  [ "$(id -u)" = 0 ] || fail "run as root"
  dir="$(dirname "$(hi_installer "$name")")"
  [ -f "$dir/$name-model.sh" ] || fail "this build has no $name-model.sh"
  if [ "$op" = apply ]; then body="$(head -c 8192)"; [ -n "$body" ] || fail "no model settings on stdin"; fi
  printf '%s' "$body" | hi_task 120 0 "$name" /usr/bin/env "HERMES_HOME=$(hi_home "$name")/.hermes" /bin/sh "$dir/$name-model.sh" "$op" 2>&1 | hi_relay
  [ "${PIPESTATUS[1]}" = 0 ] || fail "$name's model settings were not changed (the line above says why)"
  local g=0; hi_guard_toolsets "$name" || g=$?; case "$g" in 0|3) ;; *) fail "$name's toolset allowlist could not be held after the model change; $(hi_unit "$name") is stopped, not reloaded" ;; esac
  # A drain-aware restart where the unit can reload, a plain restart where it cannot: Hermes reads
  # its config as it starts.
  systemctl reload-or-restart "$(hi_unit "$name")" >/dev/null 2>&1 \
    || fail "$name's model settings were saved, but $(hi_unit "$name") would not reload, so it keeps the old model until it is restarted (journalctl -u $(hi_unit "$name"); sudo systemctl restart $(hi_unit "$name"))"
  info "$name's model settings changed; $(hi_unit "$name") finishes its turns and restarts"
}

# The few things a person does by hand, as the mind account, in their terminal. A list, not a
# pass-through: the person's sudo rule must not become a shell as the account that holds the
# Mind's memory.
hi_run() { # <name> <args...>
  local name="${1:-}"; shift || true
  hi_known "$name" || fail "usage: yantrik-update harness run hermes model|status|--version"
  hi_migrated "$name" || fail "$name is not under the mind account here"
  # Exactly one word, and one of three: nothing after it reaches Hermes's argv.
  [ "$#" = 1 ] && case "$1" in model|status|--version) true ;; *) false ;; esac \
    || fail "$name: only model, status and --version, alone, are run for you"
  [ "$(id -u)" = 0 ] || fail "run as root"
  local rc=0
  hi_task 1800 1 "$name" "$(hi_home "$name")/.local/bin/$name" "$@" || rc=$?
  # `hermes model` rewrites the whole file: hold the allowlist before the gateway reads it again.
  if [ "$1" = model ]; then
    local g=0; hi_guard_toolsets "$name" || g=$?
    case "$g" in
      0) ;;
      3) systemctl try-reload-or-restart "$(hi_unit "$name")" >/dev/null 2>&1 || warn "$(hi_unit "$name") would not reload onto the corrected file; restart it: sudo systemctl restart $(hi_unit "$name")" ;;
      *) warn "$name's toolset allowlist could not be held after \`model\`; $(hi_unit "$name") is stopped"; [ "$rc" != 0 ] || rc=1 ;;
    esac
  fi
  return "$rc"
}

cmd_harness() {
  local sub="${1:-}"; shift || true
  case "$sub" in
    install|restart|reload|model|run|status) ;;
    *) fail "usage: yantrik-update harness install|restart|reload|model|run|status hermes" ;;
  esac
  migrate_if_needed
  elevate "$sub" "$@"
  case "$sub" in
    install) hi_install "${1:-}" ;;
    restart) hi_restart "${1:-}" ;;
    reload) hi_reload "${1:-}" ;;
    model) hi_model "$@" ;;
    run) hi_run "$@" ;;
    status)
      hi_known "${1:-}" || fail "usage: yantrik-update harness status hermes"
      info "$1: $(hi_migrated "$1" && echo "under the mind account" || echo "not moved")"
      systemctl is-active "$(hi_unit "$1")" 2>/dev/null | sed 's/^/  gateway: /'
      ;;
  esac
}

cmd_migrate_harnesses() {
  migrate_if_needed
  elevate "$@"
  hi_migrate_all
}

# From reconcile: a machine with a person's harness, or one already moved, is brought along without
# holding the update up. The first move downloads the harness, so it runs as its own transient
# unit (one at a time, by name) and the update goes on.
reconcile_harness_isolation() {
  [ "$(id -u)" = 0 ] || return 0
  hi_parts_ok || { warn "harness moves are off: a harness-isolation helper is missing from this install"; return 0; }
  tree_is_roots || return 0
  local who owner home name todo=0
  who="$(hi_owner_home)" || return 0
  owner="${who% *}"; home="${who#* }"
  hi_guard_mixed "$owner" "$home"
  for name in hermes; do
    hi_migrated "$name" && { todo=1; continue; }
    # A person's own: moved only when this build can, and not again after a failure nobody has asked to retry.
    hi_person_has "$name" "$home" && hi_has_installer "$name" && ! hi_auto_blocked "$name" && todo=1
  done
  [ "$todo" = 1 ] || return 0
  command -v systemd-run >/dev/null 2>&1 || { hi_migrate_all; return 0; }
  if systemd-run --quiet --unit=yantrik-harness-migrate --no-block --collect \
       "$BIN_DIR/yantrik-update" migrate-harnesses >/dev/null 2>&1; then
    info "harnesses are being moved to the mind account in the background: journalctl -u yantrik-harness-migrate"
  else
    info "a harness move is already running (journalctl -u yantrik-harness-migrate)"
  fi
  return 0
}
