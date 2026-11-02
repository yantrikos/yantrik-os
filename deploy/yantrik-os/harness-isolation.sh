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
#     The Mind's own unit gets the same line when its bundle handles SIGTERM as PID 1 (see its unit);
#     until then the Mind can still read Hermes's keys;
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

HI_USER="${HI_USER:-yantrik-mind}"
HI_BASE="${HI_BASE:-/var/lib/yantrik-mind/harness}"
HI_DOOR="${HI_DOOR:-/run/yantrik-minds}"
HI_PROXY="${HI_PROXY:-http://127.0.0.1:7450}"
HI_UNIT_DIR="${HI_UNIT_DIR:-/etc/systemd/system}"
HI_GROUP="${HI_GROUP:-yantrik-minds}"
# Root's own record of what has moved. Not in the account's home, which the harness can write.
HI_STATE="${HI_STATE:-/var/lib/yantrik-harness}"
# PrivatePIDs= (the unit's PID namespace) is systemd 257's; an older systemd would ignore the line
# in a unit, so nothing is installed or started without it.
HI_MIN_SYSTEMD="${HI_MIN_SYSTEMD:-257}"
HI_COPY_TIMEOUT="${HI_COPY_TIMEOUT:-1200}"
# What a person who migrated keeps: their old Hermes, whole, renamed. Never deleted by this.
HI_KEEP=".yantrik-moved"
# The installer carries this word when it understands the two phases. An older installer (a rolled
# back build's share/) is never run as the mind account: it would try to make a user unit.
HI_CAPABILITY="YANTRIK_HARNESS_ISOLATED"

hi_known() { case "$1" in hermes) return 0 ;; *) return 1 ;; esac; }
hi_unit() { case "$1" in hermes) printf 'hermes-gateway.service' ;; esac; }
hi_home() { printf '%s/%s' "$HI_BASE" "$1"; }
hi_installer() { printf '%s/share/harnesses/lib/install/%s.sh' "$PREFIX" "$1"; }
hi_plugin_dir() { printf '%s/share/harnesses/%s' "$PREFIX" "$1"; }
# What the person's home holds of a harness: the directory its code and data are in.
hi_person_dir() { case "$1" in hermes) printf '.hermes' ;; esac; }
# The names that are Hermes's code, rebuilt by the installer from the pinned commit and the image's
# wheels, with absolute paths inside that would not survive a move. Everything else in .hermes
# (config, keys, state.db and its WAL, sessions, memories, skills, cron, third-party plugins) moves.
hi_code_dirs() {
  case "$1" in hermes) printf '%s' '.hermes/hermes-agent:.hermes/tools:.hermes/installs:.hermes/bin:.hermes/venv:.hermes/node_modules:.hermes/.cache:.hermes/.yantrik-staging:.hermes/plugins/yantrik:.hermes/plugins/yantrikdb' ;; esac
}
# Files outside that directory the harness's own data lives in (globs, relative to the home).
hi_person_extra() { case "$1" in hermes) printf '%s' '.local/share/yantrik/memory/hermes.db*' ;; esac; }

# The tree a harness moves, by one program so that what is listed, copied and compared cannot
# disagree. `list`: NUL-separated relative paths of the files and links to move. `digest`: the
# count and one hash over their names and contents. Links are listed and hashed by their target
# text, never followed.
HI_PY_TREE='
import glob, hashlib, os, stat, sys
mode, root, top, skip, extra = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4].split(":"), sys.argv[5]
skip = [s for s in skip if s]
def skipped(rel):
    return any(rel == s or rel.startswith(s + "/") for s in skip)
found = []
if top and os.path.isdir(os.path.join(root, top)) and not os.path.islink(os.path.join(root, top)):
    for here, dirs, files in os.walk(os.path.join(root, top), followlinks=False):
        rel = os.path.relpath(here, root)
        dirs[:] = sorted(d for d in dirs if not skipped(os.path.join(rel, d)))
        for d in list(dirs):
            if os.path.islink(os.path.join(here, d)):
                found.append(os.path.join(rel, d))
        for f in sorted(files):
            full = os.path.join(here, f)
            if not skipped(os.path.join(rel, f)) and (os.path.islink(full) or os.path.isfile(full)):
                found.append(os.path.join(rel, f))
for pattern in filter(None, extra.split(":")):
    for path in sorted(glob.glob(os.path.join(root, pattern))):
        if os.path.islink(path) or os.path.isfile(path):
            found.append(os.path.relpath(path, root))
found = sorted(set(found))
if mode == "list":
    sys.stdout.buffer.write(b"\0".join(p.encode("utf-8", "surrogateescape") for p in found))
    sys.exit(0)
digest = hashlib.sha256()
for rel in found:
    full = os.path.join(root, rel)
    digest.update(rel.encode("utf-8", "surrogateescape") + b"\0")
    if os.path.islink(full):
        digest.update(b"L" + os.readlink(full).encode("utf-8", "surrogateescape"))
    elif os.path.isfile(full):
        # Never blocks on a file swapped for a FIFO after the walk, and reads only regular files.
        try:
            fd = os.open(full, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
        except OSError:
            digest.update(b"?")
        else:
            with os.fdopen(fd, "rb") as f:
                if stat.S_ISREG(os.fstat(f.fileno()).st_mode):
                    for block in iter(lambda: f.read(1 << 20), b""):
                        digest.update(block)
                else:
                    digest.update(b"?")
    digest.update(b"\0")
print(len(found), digest.hexdigest())
'

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
      --clear-groups --no-new-privs -- env -i "${env_args[@]}" "$@" ) 9>&-
}
hi_as_mind() { hi_run_as "$HI_USER" "$HI_BASE" "$@"; }
hi_as_owner() { # <command...>
  local owner home
  owner="$(desktop_owner)"; home="$(getent passwd "$owner" | cut -d: -f6)"
  hi_run_as "$owner" "$home" "$@"
}

# What a script says, one line each, for a person's row or a terminal: control characters dropped
# (the text is a program's, written to a terminal root owns).
hi_relay() { tr -d '\000-\010\013-\037\177' | while IFS= read -r line; do info "$line"; done; }

# <seconds> <user> <home> <command...>: a fixed tool, output relayed, exit status returned, ended
# by KILL at the limit.
hi_run_timed() {
  local secs="$1" rc; shift
  HI_TIMEOUT="$secs" hi_run_as "$@" 2>&1 </dev/null | hi_relay
  rc=${PIPESTATUS[0]}
  return "$rc"
}

# ── Code the account can write: only ever inside the unit's own sandbox ────────────────

hi_systemd_ok() { # systemd new enough for PrivatePIDs=
  local v; v="$(systemctl --version 2>/dev/null | sed -n '1s/^systemd \([0-9][0-9]*\).*/\1/p')"
  [ "${v:-0}" -ge "$HI_MIN_SYSTEMD" ] 2>/dev/null
}

# The home must be the account's own plain directory, not a link someone put there: BindPaths= would
# follow it.
hi_check_home() { # <name>
  local h; h="$(hi_home "$1")"
  [ -d "$h" ] && [ ! -L "$h" ] && [ ! -L "$HI_BASE" ] && [ "$(stat -c %u "$h" 2>/dev/null)" = "$(id -u "$HI_USER" 2>/dev/null)" ] \
    || { warn "$h is not the account's own directory (a link, or not its): refusing to run anything in it"; return 1; }
}

# The sandbox lines of the harness's unit (between its `# sandbox:begin` and `# sandbox:end`), as
# -p arguments in HI_PROPS: one list, the unit's, shared with every run below, never a second copy.
hi_sandbox_props() { # <name>
  local f="$HI_UNIT_DIR/$(hi_unit "$1")" line in=0
  HI_PROPS=()
  [ -f "$f" ] || { warn "no $f"; return 1; }
  while IFS= read -r line; do
    case "$line" in
      "# sandbox:begin"*) in=1 ;;
      "# sandbox:end"*) in=0 ;;
      ""|"#"*) ;;
      *) [ "$in" = 1 ] && HI_PROPS+=(-p "$line") ;;
    esac
  done < "$f"
  [ "${#HI_PROPS[@]}" -gt 0 ] || { warn "$f has no sandbox block"; return 1; }
}

# <seconds> <pty: 0|1> <name> <command...>: a transient unit of the account with the gateway's
# sandbox (same mount view, PID namespace, no home, no Mind files) and its own cgroup, torn down
# when the command ends or the limit does. A pty is the unit's own, so the account's code never holds
# the caller's terminal. A systemd without PrivatePIDs refuses the property, and nothing runs.
hi_task() {
  local secs="$1" pty="$2" name="$3"; shift 3
  hi_systemd_ok || { warn "systemd is older than $HI_MIN_SYSTEMD (no PrivatePIDs=): nothing was run"; return 1; }
  hi_check_home "$name" && hi_sandbox_props "$name" || return 1
  local -a mode=(--pipe)
  [ "$pty" = 1 ] && mode=(--pty)
  systemd-run --quiet --wait --collect "${mode[@]}" -p "User=$HI_USER" -p "Group=$HI_GROUP" \
    -p "WorkingDirectory=$(hi_home "$name")" -p "RuntimeMaxSec=$secs" -p TimeoutStopSec=10 \
    "${HI_PROPS[@]}" -- "$@"
}
hi_task_relay() { # <seconds> <name> <command...>: output relayed, no terminal
  local secs="$1" name="$2" rc; shift 2
  hi_task "$secs" 0 "$name" "$@" 2>&1 </dev/null | hi_relay
  rc=${PIPESTATUS[0]}
  return "$rc"
}

# The empty, private directory the Mind's unit masks (InaccessiblePaths=), made before that unit is
# (re)started: a mask on a path that does not exist yet is skipped, and a Mind started then would
# see the keys that land there. Made by the account, never through a link.
hi_mask_dir() {
  if [ -L "$HI_BASE" ]; then warn "$HI_BASE is a link; the harnesses' directory is not masked from the Mind"; return 1; fi
  HI_TIMEOUT=60 hi_run_as "$HI_USER" / sh -c 'umask 077; mkdir -p "$1" && chmod 0700 "$1"' sh "$HI_BASE" \
    || { warn "could not make $HI_BASE as $HI_USER"; return 1; }
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
    && chmod 0644 "$HI_STATE/$1.isolated.new" && mv -f "$HI_STATE/$1.isolated.new" "$HI_STATE/$1.isolated"
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
  if [ -f "$bin" ] && grep -q yantrik-harness-shim "$bin" 2>/dev/null; then return 0; fi
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
  getent passwd "$HI_USER" >/dev/null 2>&1 || fail "there is no mind account ($HI_USER); nothing was changed"
  [ -f "$HI_UNIT_DIR/$unit" ] || fail "this build ships no $unit; nothing was changed"
  [ -d "$HI_DOOR" ] || fail "the mind door ($HI_DOOR) is not there; run: sudo yantrik-update reconcile"
  hi_mask_dir || fail "could not make $HI_BASE as $HI_USER; nothing was changed"
  hi_as_mind sh -c 'umask 077; mkdir -p "$1"' sh "$(hi_home "$1")" \
    || fail "could not make $(hi_home "$1") as $HI_USER"
  hi_check_home "$1" || fail "$(hi_home "$1") is not the account's own directory; nothing was changed"
}

# A directory the mind account made, put aside whole (kept, never deleted) so a retry starts clean.
hi_set_aside() { # <name> <why>
  local home stamp; home="$(hi_home "$1")"; stamp="$(date +%Y%m%d-%H%M%S)-$RANDOM"
  [ -n "$(hi_as_mind ls -A "$home" 2>/dev/null)" ] || return 0
  hi_as_mind mv "$home" "$home.$2-$stamp" && info "kept the earlier $1 directory as $home.$2-$stamp"
  hi_as_mind sh -c 'umask 077; mkdir -p "$1"' sh "$home"
}

# Copy the person's Hermes data into the mind account's home, as the person (reading) and as the
# mind (writing, umask 077), then compare count and hash on both sides. The person's copy is not
# touched; if the numbers differ, nothing is used.
hi_copy_data() { # <name> <owner> <home>
  local name="$1" skip extra top dest before after
  skip="$(hi_code_dirs "$name")"; extra="$(hi_person_extra "$name")"; top="$(hi_person_dir "$name")"; dest="$(hi_home "$name")"
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

# A failed install puts the person's own Hermes back as it was: their gateway started again if it
# was running, the mind account's half-made directory kept aside, the system unit off (unless it
# had already moved: then its working gateway is left alone).
hi_abort() { # <name> <was-active: active|idle|none> <owner> <reason>
  local unit; unit="$(hi_unit "$1")"
  trap - TERM INT HUP
  warn "$1 was not moved: $4"
  hi_migrated "$1" || { systemctl disable --now "$unit" >/dev/null 2>&1 || true; hi_set_aside "$1" failed; }
  if [ "$2" = active ]; then
    systemctl --user -M "$3@" start "$(hi_unit "$1")" >/dev/null 2>&1 \
      && info "your own $1 gateway is running again, as it was"
  fi
  fail "$1 is not isolated yet: $4"
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
  hi_mark_migrated "$1" || fail "could not record that $1 is isolated"
  trap - TERM INT HUP
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
  tree_is_roots || fail "this machine's tree is not root's yet; run: sudo yantrik-update migrate-ownership"
  hi_has_installer "$name" || fail "this build's $name installer does not know the mind account yet; nothing was changed"
  unit="$(hi_unit "$name")"
  who="$(hi_owner_home)" || fail "cannot tell whose desktop this is; nothing was changed"
  owner="${who% *}"; home="${who#* }"
  say "Setting $name up under the mind account"
  hi_ready "$name"
  if hi_person_has "$name" "$home" && ! hi_migrated "$name"; then
    was="$(hi_stop_person_unit "$name" "$owner" "$home")"
    HI_NOW="$name $owner $was"
    trap hi_interrupted TERM INT HUP
    hi_set_aside "$name" earlier
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

# Every known harness that is the person's, or already moved: moved, or finished and refreshed.
# From `yantrik-update migrate-harnesses`, and from reconcile (detached, because a first move
# downloads the harness).
hi_migrate_all() {
  local name who owner home
  [ "$(id -u)" = 0 ] || return 0
  tree_is_roots || return 0
  who="$(hi_owner_home)" || return 0
  owner="${who% *}"; home="${who#* }"
  for name in hermes; do
    if hi_migrated "$name"; then
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
  systemctl restart "$(hi_unit "$1")" || fail "$(hi_unit "$1") would not restart (journalctl -u $(hi_unit "$1"))"
  info "$(hi_unit "$1") restarted"
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
  hi_task 1800 1 "$name" "$(hi_home "$name")/.local/bin/$name" "$@"
}

cmd_harness() {
  local sub="${1:-}"; shift || true
  case "$sub" in
    install|restart|run|status) ;;
    *) fail "usage: yantrik-update harness install|restart|run|status hermes" ;;
  esac
  migrate_if_needed
  elevate "$sub" "$@"
  case "$sub" in
    install) hi_install "${1:-}" ;;
    restart) hi_restart "${1:-}" ;;
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
  tree_is_roots || return 0
  local who owner home name todo=0
  who="$(hi_owner_home)" || return 0
  owner="${who% *}"; home="${who#* }"
  for name in hermes; do
    if hi_migrated "$name" || hi_person_has "$name" "$home"; then todo=1; fi
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
