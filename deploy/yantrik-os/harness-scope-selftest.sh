#!/bin/bash
# The abstract-socket scope (#411, M1) on everything the gateway unit runs as the mind account:
# bash deploy/yantrik-os/harness-scope-selftest.sh. No root needed.
#
# The person's Xwayland listens on an abstract socket, which no mount or home protection hides. So
# every line of the unit that runs as the account must start under yantrik-init --scope-abstract-unix,
# and the root-owned scripts it runs must not look anything up in what the account wrote.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
# shellcheck source=deploy/yantrik-os/selftest-lib.sh
. "$here/selftest-lib.sh"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
unit="$repo/config/systemd/hermes-gateway.service"
install_lib="$repo/harnesses/lib/install"
init="$here/yantrik-init"

echo "== every Exec line that runs as the account is scoped"
want="/opt/yantrik/bin/yantrik-init --scope-abstract-unix "
unscoped="$(grep -E '^Exec(Start|StartPre|StartPost|Stop|StopPost|Condition)=' "$unit" | grep -v -E '^Exec[A-Za-z]+=\+' | grep -v -F "=$want")"
is "no ExecStart/ExecStartPre/ExecStartPost/ExecStop(Post) without + runs outside the scope" "" "$unscoped"
has "the guard before the start is one of them, scoped" "ExecStartPre=${want}/bin/sh /opt/yantrik/share/harnesses/lib/install/hermes-guard.sh start" "$(cat "$unit")"
has "... and the gateway itself" "ExecStart=${want}" "$(cat "$unit")"
# ExecReload runs inside the unit's PID namespace unless it is a `+` line, and $MAINPID is the host's
# number for the main process: without the `+` the kill finds "No such process" (the 520 gate on #1076).
is "the only other Exec line is the reload, a kill by root's /bin, outside the PID namespace (+)"   "ExecReload=+/usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin /bin/kill -USR1 \$MAINPID" "$(grep -E '^Exec(Reload)=' "$unit")"

# hi_task runs the installer, the plugin refresh, `hermes model` and the guard's `after`: Hermes's code.
has "hi_task puts every task under the scope (L5)" 'yantrik-init" --scope-abstract-unix "$@"' "$(cat "$here/harness-sandbox.sh")"

echo "== every + line (root, unsandboxed) runs only root-owned code (H1)"
# `+` drops User= and the namespaces but keeps Environment=, so the unit's PATH (the account's
# ~/.local/bin first) reaches a root shell. Each + line must reset it to root's directories, run
# absolute programs, and source/read nothing the account can write; then it is run with a planted
# command for each name in an account-writable PATH, and no plant may run.
rootpath='(/usr/sbin|/usr/bin|/sbin|/bin)'
plusdirs="$work/plus"; mkdir -p "$plusdirs/home/.local/bin"
for name in systemctl head cut stat grep readlink nft tr sed awk cat id dirname basename ls find xargs env sh bash python3 test; do
  printf '#!/bin/sh\n/usr/bin/touch "%s/ran-%s"\n' "$plusdirs" "$name" > "$plusdirs/home/.local/bin/$name"; chmod 755 "$plusdirs/home/.local/bin/$name"
done
for f in "$repo"/config/systemd/*.service; do
  u="$(basename "$f")"
  # `!`/`!!` also run with full privileges and keep Environment=; none is allowed (L6).
  is "$u: no ! or !! Exec line (root with the unit's environment)" "" "$(grep -E '^Exec[A-Za-z]+=[-:@+]*!' "$f")"
  grep -q -E '^Exec[A-Za-z]+=[-:@]*\+' "$f" || continue
  lacks "$u: no EnvironmentFile (systemd would read it as root through the account's link)" "EnvironmentFile" "$(grep -v '^#' "$f")"
  n=0
  while IFS= read -r line; do
    # Strip the whole leading prefix run (-, :, @, + in any order), not just one + (L6).
    n=$((n + 1)); cmd="${line#*=}"
    while :; do case "$cmd" in [-:@+]*) cmd="${cmd#?}" ;; *) break ;; esac; done
    case "$cmd" in
      *.local/bin*|*"source "*) bad=yes ;;
      *) bad=no ;;
    esac
    is "$u + line $n names nothing of the account's and sources nothing" "no" "$bad"
    printf '%s' "$cmd" | grep -q -E "^/usr/bin/env -i PATH=$rootpath(:$rootpath)* /" && ok=yes || ok=no
    is "$u + line $n starts /usr/bin/env -i PATH=<root dirs only> /absolute-program" "yes" "$ok"
    printf '%s' "$cmd" | grep -q -E '(^|[;&|{(][[:space:]]*)\.[[:space:]]' && src=yes || src=no
    is "$u + line $n has no '. file' source" "no" "$src"
    rm -f "$plusdirs"/ran-*
    run="$(printf '%s' "$cmd" | sed -e 's/\$\$/$/g' -e 's/%%/%/g')"
    # The unit's own PATH, as the + line inherits it, with the account's directory first.
    env -i HOME="$plusdirs/home" PATH="$plusdirs/home/.local/bin:/usr/local/bin:/usr/bin:/bin" /bin/sh -c "$run" </dev/null >/dev/null 2>&1
    ran=""; for r in "$plusdirs"/ran-*; do [ -e "$r" ] && ran="$ran${r##*/} "; done
    is "$u + line $n: no command planted in the account's ~/.local/bin ran as root" "" "$ran"
  done < <(grep -E '^Exec[A-Za-z]+=[-:@]*\+' "$f")
done
# The test sees what it should: a bare-name + line is caught.
# shellcheck disable=SC2016  # the text of a command, not an expansion
bare='v=$(systemctl --version | head -n 1)'
rm -f "$plusdirs"/ran-*
env -i PATH="$plusdirs/home/.local/bin:/usr/bin:/bin" /bin/sh -c "$bare" >/dev/null 2>&1
is "control: the same planted dir does catch a + line that looks commands up by name" "yes" "$([ -e "$plusdirs/ran-systemctl" ] && echo yes || echo no)"

echo "== the scripts run in it look nothing up in what the account wrote"
for f in hermes-guard.sh hermes-model.sh; do
  body="$(grep -v '^[[:space:]]*#' "$install_lib/$f")"
  has "$f sets a fixed PATH after common.sh put ~/.local/bin first" "PATH=/usr/bin:/bin" "$body"
  # shellcheck disable=SC2016  # the text of the script, not an expansion
  has "... and runs the runtime lookup on /usr/bin/python3 -I (no user site, no .pth)" '/usr/bin/python3 -I "$HARNESS_LIB/hermes_runtime.py"' "$body"
done
H="$work/home"; mkdir -p "$H/.local/bin" "$H/.hermes"
printf '#!/bin/sh\necho ran > "%s/plant-ran"\necho /bin/true\n' "$work" > "$H/.local/bin/python3"; chmod 755 "$H/.local/bin/python3"
for f in hermes-guard.sh hermes-model.sh; do
  rm -f "$work/plant-ran"
  env -i HOME="$H" HERMES_HOME="$H/.hermes" PATH="$H/.local/bin:/usr/bin:/bin" sh "$install_lib/$f" start </dev/null >/dev/null 2>&1
  is "a python3 planted in the account's ~/.local/bin is not run by $f" "no" "$([ -e "$work/plant-ran" ] && echo yes || echo no)"
done

echo "== the scope itself"
cat > "$work/abs.py" <<'PY'
import os, socket, subprocess, sys
name = "\0yantrik-selftest-%d" % os.getpid()
srv = socket.socket(socket.AF_UNIX)
srv.bind(name)
srv.listen(1)
child = ("import socket, sys\n"
         "s = socket.socket(socket.AF_UNIX)\n"
         "try:\n s.connect(chr(0) + sys.argv[1]); print('CONNECTED')\n"
         "except OSError:\n print('DENIED')\n")
r = subprocess.run([sys.executable, sys.argv[1], "--scope-abstract-unix", sys.executable, "-c", child, name[1:]],
                   capture_output=True, text=True)
print(r.returncode, r.stdout.strip())
PY
r="$(python3 "$work/abs.py" "$init" 2>/dev/null)"
if scope_supported; then
  is "yantrik-init --scope-abstract-unix: the connect is refused" "0 DENIED" "$r"
else
  echo "  note: this kernel has no Landlock ABI 6: the scope cannot be applied, so the init must refuse"
  is "... and nothing is run (126)" "126" "${r%% *}"
fi

# The guard's own Python is Hermes's venv Python, which the account can replace. Under the unit's
# ExecStartPre that plant must meet the scope too (M1a).
V="$work/home/venv/bin"; mkdir -p "$V"
cat > "$V/python" <<'PY'
#!/usr/bin/python3
import os, socket, sys
s = socket.socket(socket.AF_UNIX)
try:
    s.connect("\0" + os.environ["PROBE"]); out = "CONNECTED"
except OSError:
    out = "DENIED"
open(os.environ["OUT"], "w").write(out)
PY
chmod 755 "$V/python"; : > "$V/hermes"; chmod 755 "$V/hermes"; ln -s "$V/hermes" "$H/.local/bin/hermes"
cat > "$work/guard.py" <<'PY'
import os, socket, subprocess, sys
init, guard, home, out = sys.argv[1:5]
name = "yantrik-selftest-guard-%d" % os.getpid()
srv = socket.socket(socket.AF_UNIX); srv.bind("\0" + name); srv.listen(1)
env = {"HOME": home, "HERMES_HOME": home + "/.hermes", "PATH": "/usr/bin:/bin", "PROBE": name, "OUT": out}
r = subprocess.run([sys.executable, init, "--scope-abstract-unix", "/bin/sh", guard, "start"], env=env, capture_output=True)
print(r.returncode)
PY
rc="$(python3 "$work/guard.py" "$init" "$install_lib/hermes-guard.sh" "$H" "$work/probe" 2>/dev/null)"
if scope_supported; then
  is "a Python planted where the guard runs Hermes's is refused the abstract socket (M1a)" "DENIED" "$(cat "$work/probe" 2>/dev/null)"
else
  is "... without the scope the guard is not run at all (126)" "126" "$rc"
fi

echo "== the reload signal reaches Hermes through the PID namespace (the 520 gate on #1076)"
# The unit's main process is yantrik-init, PID 1 of a namespace of its own; ExecReload (a + line) kills
# its HOST pid. PID 1 of a namespace drops a signal it has no handler for, so what matters is that
# yantrik-init has one and passes it on. Needs root for the namespace.
cat > "$work/hermes.py" <<'PY'
import signal, sys, time
signal.signal(signal.SIGUSR1, lambda *_: (print("DRAINING", flush=True), sys.exit(75)))
print("up", flush=True)
time.sleep(8)
PY
if unshare --pid --fork --mount-proc true >/dev/null 2>&1; then
  reload_reaches() { # <command run as PID 1 of a new namespace>...: its output after a host-side USR1
    local out="$work/reload.out" ns pid i
    : > "$out"
    unshare --pid --fork --mount-proc "$@" >> "$out" 2>&1 & ns=$!
    for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do grep -q '^up' "$out" && break; sleep 0.25; done
    pid="$(pgrep -P "$ns" | head -n 1)"
    [ -n "$pid" ] && kill -USR1 "$pid"
    sleep 1
    if kill -0 "$pid" 2>/dev/null; then echo ALIVE >> "$out"; kill -KILL "$pid" 2>/dev/null; else echo GONE >> "$out"; fi
    wait "$ns" 2>/dev/null
    tr '
' ' ' < "$out"
  }
  has "a USR1 sent to yantrik-init's host pid reaches Hermes, which drains" "DRAINING" "$(reload_reaches python3 "$init" python3 "$work/hermes.py")"
  has "... and the init ends with it (GONE)" "GONE" "$(reload_reaches python3 "$init" python3 "$work/hermes.py")"
  has "control: a PID 1 with no handler drops the same signal (so the handler is what matters)" "ALIVE" "$(reload_reaches python3 -c 'import time; print("up", flush=True); time.sleep(8)')"
else
  echo "  note: no PID namespace here (needs root); the reload signal path was not run"
fi

echo
if [ "$fails" = 0 ]; then echo "all scope checks passed"; exit 0; fi
echo "$fails scope check(s) failed"; exit 1
