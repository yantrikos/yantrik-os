#!/bin/bash
# Sourced by harness-isolation.sh, not run: how account-writable code is run, and the checks that
# gate it (#411). Split out of that file to keep both under 600 lines.
#
# Everything the account can write and root would run (the installer, the plugin refresh, `hermes
# model`) runs only in a transient unit with the gateway unit's own sandbox properties (hi_task),
# never in root's namespace. Needs HI_* from harness-isolation.sh, and warn/info/hi_run_as/hi_home/
# hi_unit from it, at call time.

# ── Code the account can write: only ever inside the unit's own sandbox ────────────────

hi_systemd_ok() { # systemd new enough for PrivatePIDs=
  local v; v="$(systemctl --version 2>/dev/null | sed -n '1s/^systemd \([0-9][0-9]*\).*/\1/p')"
  [ "${v:-0}" -ge "$HI_MIN_SYSTEMD" ] 2>/dev/null
}

# Is the unit's main process really in a PID namespace of its own? Asked of the kernel, not of the
# unit's settings: the namespace links of that process and of PID 1 must differ.
hi_pid_ns_private() { # <unit>
  local pid mine one
  pid="$(systemctl show -p MainPID --value "$1" 2>/dev/null)"
  case "$pid" in ''|0|*[!0-9]*) return 1 ;; esac
  mine="$(readlink "$HI_PROC/$pid/ns/pid" 2>/dev/null)"; one="$(readlink "$HI_PROC/1/ns/pid" 2>/dev/null)"
  [ -n "$mine" ] && [ -n "$one" ] && [ "$mine" != "$one" ]
}

# Is the Mind's unit, as loaded and as running, the one that keeps a harness out of its reach? It
# masks the harnesses' directory, has its own PID namespace, and (when it is running) its live main
# process really is in one. A build from before this release has neither, and the Mind then reads a
# harness's keys at rest and through /proc whatever the harness does.
hi_mind_isolated() {
  local u=yantrik-mind.service
  systemctl show -p InaccessiblePaths --value "$u" 2>/dev/null | grep -q -F "$HI_BASE" || return 1
  [ "$(systemctl show -p PrivatePIDs --value "$u" 2>/dev/null)" = yes ] || return 1
  systemctl is-active --quiet "$u" 2>/dev/null && { hi_pid_ns_private "$u" || return 1; }
  return 0
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
  # A drop-in would change the gateway and not the tasks (or the reverse): one list or none. Asked
  # of systemd, which knows every place one can be (and set-property's).
  if [ -n "$(systemctl show -p DropInPaths --value "$(hi_unit "$1")" 2>/dev/null)" ] \
     || [ -n "$(find "$f.d" -maxdepth 1 -name '*.conf' 2>/dev/null | head -n 1)" ]; then
    warn "$(hi_unit "$1") has a drop-in; the sandbox is one list, so nothing was run. Remove it."
    return 1
  fi
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
  # Under yantrik-init: the command is PID 1 of the task's namespace otherwise, with nothing to reap
  # its orphans and Hermes warning about it on every run.
  [ -x "$BIN_DIR/yantrik-init" ] || { warn "no $BIN_DIR/yantrik-init: nothing was run"; return 1; }
  hi_systemd_ok || { warn "systemd is older than $HI_MIN_SYSTEMD (no PrivatePIDs=): nothing was run"; return 1; }
  hi_check_home "$name" && hi_sandbox_props "$name" || return 1
  # The kernel's way-out table is what holds this account to the proxy; the gateway unit will not
  # start without it, and neither does a task (the unit's Requires=/ExecStartPre= are outside the
  # block, so they are repeated here).
  nft list table inet yantrik_mind_egress >/dev/null 2>&1 \
    || { warn "the egress table (inet yantrik_mind_egress) is not loaded: nothing was run. Run: sudo yantrik-update mind-egress apply"; return 1; }
  local -a mode=(--pipe)
  [ "$pty" = 1 ] && mode=(--pty)
  systemd-run --quiet --wait --collect "${mode[@]}" -p "User=$HI_USER" -p "Group=$HI_GROUP" \
    -p After=yantrik-mind-egress.service -p Requires=yantrik-mind-egress.service \
    -p "WorkingDirectory=$(hi_home "$name")" -p "RuntimeMaxSec=$secs" -p TimeoutStopSec=10 \
    "${HI_PROPS[@]}" -- "$BIN_DIR/yantrik-init" "$@"
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

