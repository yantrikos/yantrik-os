#!/bin/bash
# Sourced by harness-isolation.sh, not run: what a harness move needs before it starts, and what it
# leaves behind when it does not finish (#411). Split out of that file to keep both under 600 lines.
#
# 520's gate ran the first move on a disk with 4.5 GB free and it stopped at ENOSPC, after which the
# unfinished copy (1.5 GB) kept the disk full, the person's gateway was started into the full disk
# and crash-looped while the updater said it was "running again, as it was", and the next update
# started the whole move over again. Each of those is answered here:
#
#   - room is measured BEFORE anything is copied or installed (hi_room_check), against the person's
#     data plus the install headroom plus a floor the system keeps;
#   - an unfinished copy is DELETED, not kept (hi_discard_unfinished): it is a copy the mind account
#     made, never the person's data (theirs is not touched until the move has been proved), and a
#     kept copy is gigabytes that hold the disk full. Kept instead: one line (reason and time) in
#     root's own state, which is also what stops an automatic retry;
#   - the person's gateway is only said to be running when it has stayed up and its lock names a live
#     process (hi_person_gateway_back); otherwise the sentence says what is true;
#   - a failed move is not retried by an update or a reconcile (hi_auto_blocked). `yantrik-update
#     harness install hermes` (the button, or by hand) is the retry; a success clears the note.
#
# Needs from harness-isolation.sh, at call time: HI_*, hi_knob, info warn fail, hi_as_owner,
# hi_as_mind, hi_home, hi_unit, hi_person_dir, hi_code_dirs, hi_person_tops, hi_person_extra,
# hi_migrated, HI_PY_TREE.

# KB. The install of Hermes's code and its tools measured about 1.5 GB on 520; twice that is asked.
HI_HEADROOM_KB="$(hi_knob HI_HEADROOM_KB 3145728)"
# What must still be free when the move is done: the journal, the person's own work, root's reserve.
HI_RESERVE_KB="$(hi_knob HI_RESERVE_KB 524288)"
HI_DF="$(hi_knob HI_DF df)"
# How long the person's gateway must stay up to be called running (after it is first active).
HI_HOLD="$(hi_knob HI_HOLD 6)"

# ── Room ───────────────────────────────────────────────────────────────────────────────

hi_free_kb() { # free KB on the disk the harnesses' home is on (the nearest directory that exists)
  local p="$HI_BASE"
  while [ ! -d "$p" ] && [ "$p" != / ]; do p="$(dirname "$p")"; done
  "$HI_DF" -Pk "$p" 2>/dev/null | awk 'NR==2 {print $4 + 0}'
}

hi_data_kb() { # <name> <home>: KB of what the copy would write (the code is reinstalled, not copied)
  hi_as_owner sh -c 'cd "$1" && python3 -I -c "$2" list "$1" "$3" "$4" "$5" | xargs -0 -r du -sk -- 2>/dev/null | awk "{s += \$1} END {print s + 0}"' \
    sh "$2" "$HI_PY_TREE" "$(hi_person_tops "$1")" "$(hi_code_dirs "$1")" "$(hi_person_extra "$1")"
}

hi_stale_kb() { # KB of unfinished copies an older build kept: freed before the move, so they count
  local home k=0 d; home="$(hi_home "$1")"
  for d in "$home.failed" "$home.failed.new"; do
    [ -d "$d" ] && k=$((k + $(hi_as_mind du -sk "$d" 2>/dev/null | awk '{print $1 + 0}')))
  done
  printf '%s' "$k"
}

# <name> <home> <person-has-one: yes|no>: refuses, in a sentence, when the disk cannot hold the move.
# Nothing has been changed when this runs: it comes before the mind account's directory is made.
hi_room_check() {
  local data=0 free stale need
  [ "$3" != yes ] || data="$(hi_data_kb "$1" "$2")"
  data="${data:-0}"
  free="$(hi_free_kb)"; free="${free:-0}"
  stale="$(hi_stale_kb "$1")"
  need=$((data + HI_HEADROOM_KB + HI_RESERVE_KB))
  [ $((free + stale)) -lt "$need" ] || return 0
  hi_note_failure "$1" "not enough room: needs about $((need / 1024)) MB, $(((free + stale) / 1024)) MB free"
  fail "$1 was not moved: it needs about $((need / 1024)) MB free on this disk (your Hermes's data $((data / 1024)) MB, its install $((HI_HEADROOM_KB / 1024)) MB, and $((HI_RESERVE_KB / 1024)) MB left for the system) and $(((free + stale) / 1024)) MB is free. Nothing was changed. Free some room, then run: sudo yantrik-update harness install $1"
}

# ── The note a failed move leaves (root's, small, never a tree) ─────────────────────────

hi_failure_file() { printf '%s/%s.failed' "$HI_STATE" "$1"; }
hi_note_failure() { # <name> <reason>: replaced by the next failure, cleared by a success
  local f; f="$(hi_failure_file "$1")"
  [ ! -L "$f" ] || rm -f "$f"
  install -d -m 0755 -o root -g root "$HI_STATE" 2>/dev/null || [ -d "$HI_STATE" ] || return 0
  printf 'failed %s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$2" | tr -d '\000-\010\013-\037\177' > "$f.new" 2>/dev/null \
    && chmod 0644 "$f.new" && mv -f "$f.new" "$f" 2>/dev/null || true
}
hi_clear_failure() { rm -f "$(hi_failure_file "$1")" "$(hi_failure_file "$1").new"; return 0; }
hi_failure_text() { # <name>: "<time> <reason>" of the last failed move, if there is a note
  local f; f="$(hi_failure_file "$1")"
  [ -f "$f" ] && [ ! -L "$f" ] && sed -n '1s/^failed //p' "$f"
}

# An update or a reconcile does not start a move that failed: it says so and leaves it to the person.
hi_auto_blocked() { # <name>: succeeds (and says why) when the last move failed and nobody asked since
  local t; t="$(hi_failure_text "$1")" || return 1
  [ -n "$t" ] || return 1
  info "$1 stays the person's: the last move to the mind account failed ($t). An update does not retry it; when there is room and you want it, run: sudo yantrik-update harness install $1"
}

# ── Leaving things as they were ─────────────────────────────────────────────────────────

# What the mind account made while trying: removed, together with the full copies an older build kept
# as <home>.failed. A copy, not the person's data, and gigabytes; nothing of it is kept.
hi_discard_unfinished() { # <name> <why>
  local home; home="$(hi_home "$1")"
  hi_as_mind rm -rf "$home.failed.new" "$home.failed"
  [ -n "$(hi_as_mind ls -A "$home" 2>/dev/null)" ] || return 0
  hi_as_mind rm -rf "$home" || warn "could not remove the unfinished copy in $home ($2)"
  hi_as_mind sh -c 'umask 077; mkdir -p "$1"' sh "$home"
  info "removed the unfinished copy of $1 in $home ($2): it was a copy the mind account made, not your data, and a kept one would keep the disk full. Your own files were not touched"
}

# Is the person's own gateway really back? Active, still active after a hold with no restart in
# between (a crash loop is active for a moment each time), and its lock naming a live process when it
# has one. Prints the reason when not.
hi_person_gateway_back() { # <name> <owner> <home>
  local unit n=0 nr pid; unit="$(hi_unit "$1")"
  systemctl --user -M "$2@" start "$unit" >/dev/null 2>&1 || { printf 'it would not start'; return 1; }
  while [ "$n" -lt 30 ]; do
    systemctl --user -M "$2@" is-active --quiet "$unit" 2>/dev/null && break
    sleep 1; n=$((n + 1))
  done
  [ "$n" -lt 30 ] || { printf 'it was not active after 30 s'; return 1; }
  sleep "$HI_HOLD"
  systemctl --user -M "$2@" is-active --quiet "$unit" 2>/dev/null || { printf 'it started and then stopped'; return 1; }
  nr="$(systemctl --user -M "$2@" show -p NRestarts --value "$unit" 2>/dev/null)"
  [ "${nr:-0}" = 0 ] || { printf 'it keeps restarting (%s times)' "$nr"; return 1; }
  pid="$(hi_as_owner sh -c 'f="$1/$2/gateway.pid"; [ -f "$f" ] && grep -o "[0-9][0-9]*" "$f" | head -n 1' sh "$3" "$(hi_person_dir "$1")" 2>/dev/null)"
  if [ -n "$pid" ] && ! hi_as_owner kill -0 "$pid" 2>/dev/null; then printf 'its lock names process %s, which is not running' "$pid"; return 1; fi
  return 0
}

# A failed install puts the person's own Hermes back as it was: the system unit off (unless it had
# already moved: then its working gateway is left alone), the unfinished copy removed (before the
# gateway is started, which needs the room), the failure noted, the person's gateway started again if
# it was running, and the sentence about it true.
hi_abort() { # <name> <was-active: active|idle|none> <owner> <reason>
  local unit why free owner_home; unit="$(hi_unit "$1")"
  trap - TERM INT HUP
  warn "$1 was not moved: $4"
  if ! hi_migrated "$1"; then
    systemctl disable --now "$unit" >/dev/null 2>&1 || true
    hi_discard_unfinished "$1" failed
    hi_note_failure "$1" "$4"
  fi
  # Only a move that did not commit gives the person's gateway back: after the marker the system
  # one is the gateway, and two would share the platform's tokens.
  if [ "$2" = active ] && ! hi_migrated "$1"; then
    owner_home="$(getent passwd "$3" | cut -d: -f6)"
    if why="$(hi_person_gateway_back "$1" "$3" "${owner_home:-/}")"; then
      info "your own $1 gateway is running again, as it was"
    else
      free="$(hi_free_kb)"
      warn "your own $1 gateway is NOT running: $why ($(( ${free:-0} / 1024 )) MB free). Your files are as they were. Make room if the disk is full, then run: systemctl --user restart $unit"
    fi
  fi
  fail "$1 is not isolated yet: $4"
}
