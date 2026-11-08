#!/bin/bash
# Sourced by `yantrik-update selftest`, never run, and not installed: the checks for the DevTools
# and VNC guard (#1137), kept out of yantrik-update so that the updater does not grow by them.
# They read the text the updater writes, because that text is the rule: the uid layer stays, the
# cgroup layer refuses the person's user manager, and the hook loads it before the manager runs.
# Runs inside cmd_selftest and uses its `is`.

selftest_devtools() {
  local base session dropin unit
  base="$(devtools_guard_text alice "0, 1000" "0, 1000")"
  session="$(devtools_session_text 1000)"
  dropin="$(devtools_session_dropin_text /usr/sbin/nft)"
  unit="$(devtools_unit_text /usr/sbin/nft)"
  has() { printf '%s' "$2" | grep -qF -- "$1" && echo yes || echo no; }

  is "devtools: the uid rule stays" "yes" "$(has 'meta skuid != { 0, 1000 } reject with tcp reset' "$base")"
  is "devtools: the answering-side drop stays" "yes" "$(has 'tcp sport { 5900-5909, 9222-9322 } meta skuid != { 0, 1000 } drop' "$base")"
  is "devtools: the uid table does not carry the cgroup rule (it must load without the person's manager)" "no" "$(has cgroupv2 "$base")"
  is "devtools: the cgroup rule refuses the person's user manager, level 3" "yes" \
    "$(has 'socket cgroupv2 level 3 "user.slice/user-1000.slice/user@1000.service" reject with tcp reset' "$session")"
  is "devtools: ... on loopback, for 9222-9322 and VNC 5900-5909" "yes" \
    "$(has 'oifname "lo" tcp dport { 5900-5909, 9222-9322 } socket cgroupv2' "$session")"
  is "devtools: ... in a table of its own, replaced whole on each load" "yes" \
    "$(has 'delete table inet yantrik_devtools_session' "$session")"
  is "devtools: the cgroup rule names the uid it is given" "yes" "$(has 'user@4242.service' "$(devtools_session_text 4242)")"
  is "devtools: the manager loads the rule itself, before its first process, with full rights" "yes" \
    "$(has 'ExecStartPre=+/usr/sbin/nft -f /etc/yantrik/devtools-guard-session.nft' "$dropin")"
  is "devtools: the boot and reload unit puts it back, tolerating a manager not yet up" "yes" \
    "$(has 'ExecStart=-/usr/sbin/nft -f /etc/yantrik/devtools-guard-session.nft' "$unit")"
  is "devtools: the uid layer loads first and is not tolerant" "yes" "$(has 'ExecStart=/usr/sbin/nft -f /etc/yantrik/devtools-guard.nft' "$unit")"
  is "devtools: a failed manual load leaves no drop-in (a kernel without the match cannot lock the person out)" "yes" \
    "$(declare -f reconcile_devtools_session | tr -s ' 
' ' ' | grep -qE 'nft" -f "\$DEVTOOLS_SESSION_GUARD" 2> ?/dev/null; then rm -f [^;]*yantrik-devtools.conf"; warn .*; return 1' && echo yes || echo no)"
  is "devtools: the guard reconcile runs the cgroup layer" "yes" \
    "$(declare -f reconcile_devtools_guard | grep -q 'reconcile_devtools_session' && echo yes || echo no)"
}
