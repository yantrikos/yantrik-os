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
  is "devtools: the manager loads the rule itself, before its first process, with full rights" "yes"     "$(has "ExecStartPre=+/bin/sh -c '/usr/sbin/nft -f /etc/yantrik/devtools-guard-session.nft || " "$dropin")"
  is "devtools: ... and falls back to the lockdown, so the manager still starts" "yes"     "$(has '|| /usr/sbin/nft -f /etc/yantrik/devtools-guard-lockdown.nft' "$dropin")"
  is "devtools: the lockdown shuts the same ports to everyone but root, on loopback" "yes"     "$(has 'oifname "lo" tcp dport { 5900-5909, 9222-9322 } meta skuid != 0 reject with tcp reset' "$(devtools_lockdown_text)")"
  is "devtools: the lockdown replaces the session table whole" "yes"     "$(has 'delete table inet yantrik_devtools_session' "$(devtools_lockdown_text)")"
  is "devtools: the boot and reload unit puts it back, tolerating a manager not yet up" "yes" \
    "$(has 'ExecStart=-/usr/sbin/nft -f /etc/yantrik/devtools-guard-session.nft' "$unit")"
  is "devtools: the uid layer loads first and is not tolerant" "yes" "$(has 'ExecStart=/usr/sbin/nft -f /etc/yantrik/devtools-guard.nft' "$unit")"
  is "devtools: the guard reconcile runs the cgroup layer" "yes" \
    "$(declare -f reconcile_devtools_guard | grep -q 'reconcile_devtools_session' && echo yes || echo no)"
  selftest_devtools_reconcile
}

# reconcile_devtools_session against a stand-in nft, in a scratch tree: which branch installs the
# drop-in, and what is said when a table has gone. FAKE_FAIL picks what the stand-in refuses.
selftest_devtools_reconcile() {
  local d out rc dropin
  d="$(mktemp -d)" || return 0
  mkdir -p "$d/bin" "$d/etc" "$d/cg/user.slice/user-1000.slice/user@1000.service"
  cat > "$d/bin/nft" <<'FAKE'
#!/bin/bash
all="$*"; [ "$1" = "-f" ] && [ "$2" = "-" ] && all="$all $(cat)"
echo "$all" >> "$FAKE_LOG"
case "$FAKE_FAIL:$all" in
  probe:*yantrik_devtools_probe*|session:*/session.nft*|all:*) exit 1 ;;
esac
[ "$1" = "list" ] && [ -n "${FAKE_MISSING:-}" ] && exit 1
[ "$1" = "list" ] && [ -n "${FAKE_LOCKDOWN:-}" ] && echo 'meta skuid != 0 reject'
exit 0
FAKE
  chmod +x "$d/bin/nft"
  dropin="$d/etc/user@1000.service.d/yantrik-devtools.conf"
  dt_run() { # <FAKE_FAIL> <cgroup present: yes|no> [extra env]
    rm -rf "$d/etc"; mkdir -p "$d/etc"; : > "$d/log"
    local cgroot="$d/cg"; [ "$2" = no ] && cgroot="$d/nocg"
    ( DEVTOOLS_SESSION_GUARD="$d/etc/session.nft" DEVTOOLS_LOCKDOWN="$d/etc/lockdown.nft"       DEVTOOLS_DROPIN_ROOT="$d/etc" DEVTOOLS_CGROOT="$cgroot" DEVTOOLS_NO_RELOAD=1       FAKE_LOG="$d/log" FAKE_FAIL="$1" FAKE_MISSING="${FAKE_MISSING:-}" FAKE_LOCKDOWN="${FAKE_LOCKDOWN:-}"       reconcile_devtools_session "$d/bin/nft" 1000 ) 2>&1
  }
  out="$(dt_run none yes)"; rc=$?
  is "devtools: with the cgroup up and every load good, the drop-in goes in" "yes" "$([ -f "$dropin" ] && echo yes || echo no)"
  out="$(dt_run none no)"
  is "devtools: with nobody logged in the kernel is probed first, then the drop-in goes in" "yes"     "$([ -f "$dropin" ] && grep -q yantrik_devtools_probe "$d/log" && echo yes || echo no)"
  is "devtools: ... the probe names a cgroup every systemd machine has" "yes" "$(grep -q 'level 1 "system.slice"' "$d/log" && echo yes || echo no)"
  out="$(dt_run probe no)"
  is "devtools: with nobody logged in and a kernel that cannot match a cgroup, no drop-in (the login cannot break)" "no" "$([ -f "$dropin" ] && echo yes || echo no)"
  is "devtools: ... and the updater says so" "yes" "$(printf '%s' "$out" | grep -q 'cannot match on a cgroup' && echo yes || echo no)"
  out="$(dt_run session yes)"
  is "devtools: a rule that will not load leaves no drop-in" "no" "$([ -f "$dropin" ] && echo yes || echo no)"
  out="$(FAKE_MISSING=1 dt_run none yes)"
  is "devtools: a session table that has gone is warned about" "yes" "$(printf '%s' "$out" | grep -q 'is not loaded although' && echo yes || echo no)"
  out="$(FAKE_LOCKDOWN=1 dt_run none yes)"
  is "devtools: a lockdown table standing at login is warned about" "yes" "$(printf '%s' "$out" | grep -q 'in lockdown' && echo yes || echo no)"
  rm -rf "$d"
}
