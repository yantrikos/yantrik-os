#!/usr/bin/python3
"""The desktop's tools bridge, started the way Hermes starts it, inside the gateway unit's /proc (#411).

The 520 gate on #1076: Hermes could not use the desktop after the move. Its yantrik_os MCP server
"exited immediately" every 3 minutes while it was running. The unit has ProtectProc=invisible
(hidepid=invisible), and a non-dumpable process of the same user is then not listed in /proc at all:
Hermes's liveness check (psutil.pid_exists) said the bridge had exited, and the token service could not
read the bridge's parent chain, so it answered no token. `hermes mcp test` never looks again after the
handshake, which is why it passed.

Run as root (sudo python3 deploy/yantrik-os/yos-mcp-hidepid-selftest.py). It makes a PID and mount
namespace, mounts /proc with hidepid=invisible as the unit does, drops to an unprivileged user, and
there plays Hermes: the real token service (harnesses/hermes/bridge_token.py) and the real bridge
(yos-mcp) with a socket named in its environment, as hermes_config.py writes the MCP entry. Not root, or
no unshare/setpriv/hidepid here: it says so and passes, so a developer's laptop is not a failure; CI
runs it as root.
"""
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from importlib.machinery import SourceFileLoader

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
NOBODY = 65534
TOKEN = "t" * 32
fails = 0


def check(name, ok, detail=""):
    global fails
    print("  %s %s" % ("ok  " if ok else "FAIL", name))
    if not ok:
        fails += 1
        if detail != "":
            print("       %s" % (detail,))


def listed(pid):
    """What psutil.pid_exists asks of /proc after os.kill(pid, 0): is it listed, and can its status be read."""
    try:
        with open("/proc/%d/status" % pid, "rb") as f:
            f.read()
        return str(pid) in os.listdir("/proc")
    except OSError:
        return False


def start_bridge(bridge, env):
    return subprocess.Popen([sys.executable, "-I", bridge], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, env=env, text=True)


def ask(proc, msg_id, method):
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": msg_id, "method": method, "params": {}}) + "\n")
    proc.stdin.flush()
    return json.loads(proc.stdout.readline())


def inner(bridge, bridge_token_py):
    """As the unprivileged user, in the namespace: Hermes's side of the conversation."""
    spec = importlib.util.spec_from_loader("bridge_token", SourceFileLoader("bridge_token", bridge_token_py))
    bridge_token = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(bridge_token)
    check("this /proc hides what it should (the unit's ProtectProc=invisible)",
          subprocess.run(["grep", "-q", "hidepid=invisible", "/proc/self/mountinfo"]).returncode == 0)
    work = tempfile.mkdtemp(prefix="hidepid-")
    sock = os.path.join(work, "yantrik-bridge.sock")
    service = bridge_token.TokenService(sock)
    service.set_token(TOKEN)
    service.start()
    base = {"HOME": work, "PATH": "/usr/bin:/bin", "YANTRIK_MIND_RUN": work}
    try:
        plain = start_bridge(bridge, base)
        without = len(ask(plain, 1, "tools/list")["result"]["tools"])
        plain.stdin.close(); plain.wait()

        proc = start_bridge(bridge, dict(base, YANTRIK_AGENT_TOKEN_SOCK=sock))
        ask(proc, 1, "initialize")
        time.sleep(0.3)
        check("a live bridge that was given a token socket is listed in /proc (Hermes's pid_exists)",
              listed(proc.pid), "pid %d is not listed" % proc.pid)
        offered = len(ask(proc, 2, "tools/list")["result"]["tools"])
        check("and asked again before it lists tools, the token service answers it (the agent's tools are offered)",
              offered > without, "%d tools with a token service, %d without" % (offered, without))
        check("and it is still listed after that", listed(proc.pid), "pid %d is not listed" % proc.pid)
        proc.stdin.close(); proc.wait()
    finally:
        service.stop()
        shutil.rmtree(work, ignore_errors=True)


def outer():
    need = ("unshare", "setpriv", "mount")
    if os.getuid() != 0 or any(shutil.which(t) is None for t in need):
        print("skipped: needs root with unshare, setpriv and mount (CI runs it as root)")
        return 0
    unit = os.path.join(REPO, "config", "systemd", "hermes-gateway.service")
    with open(unit, encoding="utf-8") as f:
        check("the gateway unit still mounts /proc hidepid=invisible, which this test stands in for",
              any(line.strip() == "ProtectProc=invisible" for line in f))
    stage = tempfile.mkdtemp(prefix="hidepid-stage-")
    try:
        # The unprivileged user must be able to read what it runs, wherever the checkout is.
        os.chmod(stage, 0o755)
        files = {"self.py": os.path.abspath(__file__), "yos-mcp": os.path.join(HERE, "yos-mcp"),
                 "bridge_token.py": os.path.join(REPO, "harnesses", "hermes", "bridge_token.py")}
        for name, src in files.items():
            shutil.copy(src, os.path.join(stage, name))
            os.chmod(os.path.join(stage, name), 0o755)
        script = ('mount -t proc -o hidepid=invisible proc /proc || exit 77; '
                  'exec setpriv --reuid=%d --regid=%d --clear-groups python3 -I "$1/self.py" inner "$1/yos-mcp" "$1/bridge_token.py"' % (NOBODY, NOBODY))
        done = subprocess.run(["unshare", "--pid", "--fork", "--mount", "--propagation", "private", "bash", "-c", script, "sh", stage])
        if done.returncode == 77:
            print("skipped: this kernel or container does not allow a hidepid /proc mount")
            return 0
        return done.returncode or (1 if fails else 0)
    finally:
        shutil.rmtree(stage, ignore_errors=True)


if __name__ == "__main__":
    if sys.argv[1:2] == ["inner"]:
        inner(sys.argv[2], sys.argv[3])
        print("all checks passed" if not fails else "%d FAILED" % fails)
        sys.exit(1 if fails else 0)
    code = outer()
    sys.exit(code)
