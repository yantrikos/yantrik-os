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
is "the only other Exec line is the reload, a kill by root's /bin" "ExecReload=/bin/kill -USR1 \$MAINPID" "$(grep -E '^Exec(Reload)=' "$unit")"

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

echo
if [ "$fails" = 0 ]; then echo "all scope checks passed"; exit 0; fi
echo "$fails scope check(s) failed"; exit 1
