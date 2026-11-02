"""Is the caller the person? Proven, not assumed (#1085).

The browser surface tells a Mind from the person. The uid does not: Hermes, OpenClaw, Pi and
DeepSeek run as the person, and Hermes's bridge carries no agent token. So a caller on the
person's uid is the person only if its ancestry PROVES it, all of:

  1. The whole ancestry is read: the caller's `/proc` entry, then each parent up to pid 1, every
     one readable, each started no later than its child (a reused pid breaks that), no loop. A
     parent that has exited leaves the child under init and the chain stops short of an anchor:
     that is not proof.
  2. Somewhere in it is an anchor that only a person's session has: the desktop's own shell
     (`yantrik-ui`), the SSH daemon, or `login`, by the executable the kernel reports. A bridge's
     gateway under systemd has none of these above it.
  3. Nothing in it declares itself a bridge: no process running `yos-mcp`, none with
     `YOS_VIA=yos-mcp` in its environment (which `yos-mcp` sets on every `yos` it runs). This
     only ever refuses; its absence proves nothing, so it is defence in depth.
  4. The shell, asked about the caller's pid (`caller_kind`), says it is known and not a mind.
     That is the desktop's one rule for naming a mind (the attached harnesses by pid or program
     name, `yos-mcp`); nothing here lists one. A shell that does not answer is not an answer.

Anything else is treated as a Mind. A terminal the person opens from the desktop (a child of the
shell) proves: what they type there is theirs, as `mind_view::classify` has it. A shell inside
tmux or screen, whose server is the person's but reparented to init, does not, and neither does
anything a Mind starts that outlives its parent. The person who is refused can close the browser
a Mind would be refused, or start the command from a terminal of the desktop.
"""

import os
import time

from yantrik_surface import gate, reach, wire

PROC = "/proc"
DEPTH = 64
ANCHORS = {"sshd", "login"}               # beside the shell's own binary (`gate.is_shell_binary`)
VIA = "YOS_VIA=yos-mcp"
SHELL_TIMEOUT = 3.0
REMEMBER = 5.0                            # seconds a proof is kept, for one process that has not changed


class Unproven(Exception):
    """The caller could not be shown to be the person; `str()` says what was missing."""


def _stat(proc, pid):
    """(ppid, start time) of `pid`."""
    try:
        with open(os.path.join(proc, str(pid), "stat")) as f:
            rest = f.read().rpartition(")")[2].split()
        return int(rest[1]), int(rest[19])
    except (OSError, ValueError, IndexError):
        raise Unproven("process %s could not be read" % pid) from None


def _member(proc, pid):
    ppid, start = _stat(proc, pid)
    try:
        exe = os.readlink(os.path.join(proc, str(pid), "exe"))
    except OSError:
        exe = ""
    try:
        with open(os.path.join(proc, str(pid), "cmdline"), "rb") as f:
            cmd = [a.decode("utf-8", "replace") for a in f.read().split(b"\0") if a]
    except OSError:
        cmd = []
    try:
        with open(os.path.join(proc, str(pid), "environ"), "rb") as f:
            declared = VIA.encode() in f.read().split(b"\0")
    except OSError:
        declared = False
    return {"pid": pid, "ppid": ppid, "start": start, "exe": exe, "cmd": cmd, "declared": declared}


def chain_of(pid, proc=PROC):
    """The caller and every ancestor up to pid 1, or `Unproven`."""
    chain, seen, at = [], set(), pid
    while at > 0:
        if at in seen or len(chain) >= DEPTH:
            raise Unproven("the ancestry loops or is too deep")
        seen.add(at)
        member = _member(proc, at)
        if chain and chain[-1]["start"] < member["start"]:
            raise Unproven("process %d started after its child; a pid was reused" % at)
        chain.append(member)
        if at == 1:
            return chain
        at = member["ppid"]
    raise Unproven("the ancestry does not reach init")


def _runs_yos_mcp(member):
    names = [os.path.basename(member["exe"])] + [os.path.basename(a) for a in member["cmd"][:2]]
    return "yos-mcp" in names


def _anchored(member):
    return (os.path.basename(member["exe"]) in ANCHORS
            or (member["exe"] and gate.is_shell_binary(member["exe"])))


def ask_shell(pid):
    """The shell's `caller_kind` answer for `pid`; raises `Unproven` when it does not give one."""
    try:
        reply = wire.call_once(wire.default_socket_path(gate.SHELL), "app.act",
                               {"action": "caller_kind", "args": {"pid": str(pid)}},
                               timeout=SHELL_TIMEOUT, peer_rule=gate.must_be_the_shell)
        answer = reach._act_result(reply)  # noqa: SLF001 - the one reader of an act's answer
    except (wire.PeerRefused, reach.Unanswered, OSError, ConnectionError, ValueError) as e:
        raise Unproven("the shell did not say whether process %d is a mind's (%s)" % (pid, e)) from None
    if not isinstance(answer, dict):
        raise Unproven("the shell's answer about process %d is not one it gives" % pid)
    return answer


_proven = {}


def classify(pid, proc=PROC, ask=ask_shell, now=time.monotonic):
    """"person" when the ancestry of `pid` proves it (see the module); "mind" when it shows a
    bridge or the shell names a mind; "unproven" when it could not be shown either way (a
    Terminal left open across a shell restart, a broken chain, a shell that does not answer).
    Never raises."""
    try:
        chain = chain_of(pid, proc)
        key = (pid, chain[0]["start"], proc)
        if any(m["declared"] or _runs_yos_mcp(m) for m in chain):
            return "mind"
        if gate.is_shell_binary(chain[0]["exe"]):
            return "mind"       # the desktop's own call is the companion's, and it waits on this very answer
        if not any(_anchored(m) for m in chain):
            return "unproven"
        if _proven.get(key, 0) <= now():
            answer = ask(pid)
            if not isinstance(answer, dict) or answer.get("known") is not True:
                return "unproven"
            if answer.get("mind") is not False:
                return "mind"
    except Unproven:
        return "unproven"
    _proven[key] = now() + REMEMBER
    return "person"


def proves_person(pid, proc=PROC, ask=ask_shell, now=time.monotonic):
    return classify(pid, proc, ask, now) == "person"
