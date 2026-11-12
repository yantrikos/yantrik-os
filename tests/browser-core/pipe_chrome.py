"""A real Chromium started the way the shell starts a browser: `--remote-debugging-pipe`, its
stream a connected socket pair whose far end is the browser's fds 3 and 4 (browser_pipe.rs `attach`).
No DevTools port, and nothing of the test listens on TCP for it. Shared by the live tests, so that
they drive a browser over the same road the service does (#1200).
"""

import fcntl
import os
import socket
import subprocess
import time

# Quiet, offline and fast; and, as root (CI containers, this repo's cloud sessions), without the
# sandbox, which Chromium refuses to start with as root.
QUIET = ["--no-first-run", "--no-default-browser-check", "--disable-background-networking",
         "--disable-component-update", "--disable-gpu"]


def launch(chrome, args, log, env=None, wait=60):
    """Start `chrome` with `args` and a DevTools pipe. Returns (process, our end of the stream),
    once the browser has answered on it. Raises RuntimeError saying why when it did not."""
    args = [chrome, *QUIET, *(["--no-sandbox"] if os.geteuid() == 0 else []),
            "--remote-debugging-pipe", *args]
    mine, theirs = socket.socketpair()
    # As browser_pipe::attach does it: the end duplicated above 4 (so dup2 onto 3 and 4 cannot land
    # on itself) and inheritable, then dup2 in the child. Python's own close pass runs after
    # preexec_fn and would close the 3 and 4 made there, so it is off; every other descriptor of
    # this process is non-inheritable anyway (PEP 446).
    keep = fcntl.fcntl(theirs.fileno(), fcntl.F_DUPFD, 10)
    os.set_inheritable(keep, True)

    def fds_3_and_4():
        os.dup2(keep, 3)
        os.dup2(keep, 4)
    proc = subprocess.Popen(args, stdout=log, stderr=log, env=env, stdin=subprocess.DEVNULL,
                            preexec_fn=fds_3_and_4, close_fds=False)
    os.close(keep)
    theirs.close()
    mine.settimeout(wait)
    mine.sendall(b'{"id":1,"method":"Browser.getVersion"}\0')
    try:
        got, until = b"", time.monotonic() + wait
        while b"\0" not in got:              # a reply may come in pieces: read to the NUL
            if time.monotonic() > until:
                raise OSError("no complete reply in %d s" % wait)
            chunk = mine.recv(65536)
            if not chunk:
                raise OSError("closed before a complete reply")
            got += chunk
    except OSError as e:
        proc.kill()
        raise RuntimeError("the browser did not answer on its pipe (%s; exit %s)" % (e, proc.poll())) from None
    mine.settimeout(None)
    return proc, mine


def listening_sockets_of(pids, proc="/proc"):
    """The TCP sockets in LISTEN state held by any of `pids`, as (address, port), from the
    kernel's tables and the processes' own descriptors; what the "no DevTools listener" checks read."""
    held = set()
    for pid in pids:
        fd_dir = os.path.join(proc, str(pid), "fd")
        try:
            for name in os.listdir(fd_dir):
                try:
                    link = os.readlink(os.path.join(fd_dir, name))
                except OSError:
                    continue
                if link.startswith("socket:["):
                    held.add(int(link[8:-1]))
        except OSError:
            continue
    out = []
    for table in ("tcp", "tcp6"):
        try:
            with open(os.path.join(proc, "net", table), encoding="ascii", errors="replace") as f:
                lines = f.read().splitlines()[1:]
        except OSError:
            continue
        for line in lines:
            f = line.split()
            if len(f) > 9 and f[3] == "0A" and int(f[9]) in held:
                address, _, port = f[1].rpartition(":")
                out.append((address, int(port, 16)))
    return out


def process_tree(root, proc="/proc"):
    """`root` and every process below it (Chromium's zygote, renderers and utility processes)."""
    children = {}
    for name in os.listdir(proc):
        if not name.isdigit():
            continue
        try:
            with open(os.path.join(proc, name, "stat"), encoding="ascii", errors="replace") as f:
                parent = int(f.read().rpartition(")")[2].split()[1])
        except (OSError, ValueError, IndexError):
            continue
        children.setdefault(parent, []).append(int(name))
    out, todo = [], [root]
    while todo:
        pid = todo.pop()
        out.append(pid)
        todo.extend(children.get(pid, []))
    return out
