"""The toolkit a conformance probe is written against.

`verify_calendar.py` and `verify_images.py` were the same script written twice: the same
`act` wrapper, the same pgrep, the same "put the machine back" tail, and the same bug in
each copy — a refusal was recorded as the string "1", because `yos` reports a refusal by
printing to stderr and exiting, and `str(SystemExit(1))` is "1". The refusal text is the
thing under test in contract point 4, so losing it loses the check.

This module is that script written once, with the refusal captured properly.

Three rules it enforces on every probe that uses it:

* **An assertion records, it does not raise.** One failed check must not hide the six
  after it. `Probe.check` appends a result and carries on; the probe exits nonzero at the
  end if anything required failed.
* **Restoration always runs.** `preserved` and `moved_aside` are context managers that
  restore on the way out of the block, on an exception, on SIGTERM, and via `atexit` if
  something more violent happens. The VM is a shared machine with the user's own windows
  on it.
* **Evidence is what something else says.** The helpers read processes, the compositor,
  and files on disk. An action's own answer is evidence of what the action claimed, and
  is recorded as such — never as proof that it happened.
"""

import atexit
import contextlib
import hashlib
import io
import json
import os
import pathlib
import runpy
import shutil
import signal
import socket
import subprocess
import sys
import time

YOS = "/opt/yantrik/bin/yos"
RUNTIME_DIR = pathlib.Path(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()))
SOCKET_DIR = RUNTIME_DIR / "yantrik"
BIN_DIR = pathlib.Path("/opt/yantrik/bin")

# A probe that is killed must still unwind. SIGTERM's default is to end the process
# without running `finally` blocks or `atexit` hooks, which would leave calendar-service
# renamed away. Turning it into SystemExit makes a timeout as safe as a clean exit.
def _terminate(signum, _frame):
    raise SystemExit("probe received signal %d" % signum)


for _sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
    try:
        signal.signal(_sig, _terminate)
    except (ValueError, OSError):  # not on the main thread, or not supported
        pass


# ── Talking to the OS ────────────────────────────────────────────────────────

class YosUnavailable(RuntimeError):
    """`yos` is not on this machine, so nothing below can be checked."""


_refusals = []


def _load_call():
    """`yos`'s own `call`, with its `die` replaced so a refusal keeps its words.

    `yos.die` prints "yos: <message>" to stderr and raises `SystemExit(1)`. Loading the
    script with `runpy` and rebinding `die` inside the function's own globals means the
    message arrives as the exception's argument instead of being printed into a stderr
    nobody captured. `SystemExit` is still what is raised, so any `except SystemExit`
    written against the old behaviour keeps working.
    """
    if not os.path.exists(YOS):
        raise YosUnavailable("no %s on this machine" % YOS)
    stderr = io.StringIO()
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(stderr):
        namespace = runpy.run_path(YOS, run_name="conformance-probe")
    call = namespace["call"]

    def die(message):
        _refusals.append(message)
        raise SystemExit("yos: " + message)

    call.__globals__["die"] = die
    return call


_call = None


def call(app, method, params=None, timeout=40):
    """One JSON-RPC round trip to an app or service. Raises SystemExit when refused."""
    global _call
    if _call is None:
        _call = _load_call()
    return _call(app, method, params or {}, timeout)


def act(app, action, _timeout=40, **args):
    """Run an action and return a dict, whether it was carried out or refused.

    The shape is always the same, so a probe never has to know which way it went:

        accepted  bool or None   what the app said, if it answered at all
        settled   bool or None   false means accepted and not finished
        result    dict           the action's own report of what it did
        summary   str            the app's one-line account
        revision  str            the surface revision after the action
        refused   str or None    the refusal, in the words the caller was given

    `refused` is the point of the wrapper. The audit's finding in four apps was an action
    that answered success without looking; the repair is an action that says why it could
    not, and a probe cannot check the saying if the words are thrown away.
    """
    before = len(_refusals)
    try:
        reply = call(app, "app.act", {"action": action, "args": args}, timeout=_timeout)
    except SystemExit as exc:
        message = _refusals[-1] if len(_refusals) > before else str(exc)
        return {"accepted": False, "settled": None, "result": {}, "summary": "",
                "revision": None, "refused": message}
    except Exception as exc:  # noqa: BLE001 - a crash here is a result, not a stack trace
        return {"accepted": False, "settled": None, "result": {}, "summary": "",
                "revision": None, "refused": "%s: %s" % (type(exc).__name__, exc)}
    if not isinstance(reply, dict):
        return {"accepted": None, "settled": None, "result": reply, "summary": "",
                "revision": None, "refused": None}
    refused = None
    if reply.get("accepted") is False:
        refused = reply.get("error") or reply.get("summary") or "refused without a reason"
    return {"accepted": reply.get("accepted"), "settled": reply.get("settled"),
            "result": reply.get("result") or {}, "summary": reply.get("summary") or "",
            "revision": reply.get("revision"), "refused": refused,
            "state": reply.get("state") or {}}


# The marker the control surface puts on a refusal that never reached the app.
# `Registry::act` in `frameworks/yantrik-app/src/control.rs` formats every ceiling
# refusal as `CEILING: <app>.<action> is graded ...`, and that file's own tests assert
# `err.starts_with("CEILING:")` for precisely this reason — so a caller can branch on it
# rather than on the sentence, which is written for a person and will be reworded.
CEILING_MARKER = "CEILING:"


def refusal_kind(answer):
    """Who turned an action away: the machine, the app, or nobody.

        "policy"  the control surface refused on the action's grade, before dispatch. The
                  app never ran. Nothing in this answer is the app's account of anything,
                  and no assertion about what the app does has been exercised.
        "app"     the app itself declined, in its own words.
        None      it was not refused.

    In one place, because from the outside the first two are identical — both arrive as
    `accepted: false` with a sentence in `refused` — and reading a policy refusal as the
    app's is how a probe comes to assert that the machine's ceiling should have mentioned
    a missing process. It is the app's answer that the check is about; when the app was
    never asked, the honest record is that the check was not exercised.

    The text arrives with `yos`'s own prefix in front of the runtime's —
    `system-monitor.app.act refused: CEILING: ...` — so the marker is looked for anywhere
    in the sentence rather than at its start.
    """
    if not isinstance(answer, dict):
        return None
    if answer.get("accepted") is True:
        return None
    text = answer.get("refused") or answer.get("error")
    if not text:
        return None
    return "policy" if CEILING_MARKER in str(text) else "app"


def describe(app):
    """The whole view an app publishes, or `{"unreachable": "..."}`."""
    before = len(_refusals)
    try:
        view = call(app, "app.describe", {})
    except SystemExit as exc:
        message = _refusals[-1] if len(_refusals) > before else str(exc)
        return {"unreachable": message}
    except Exception as exc:  # noqa: BLE001
        return {"unreachable": "%s: %s" % (type(exc).__name__, exc)}
    return view if isinstance(view, dict) else {"unreachable": "describe returned %r" % type(view)}


def state(app):
    """Just the state block, so `state("calendar").get("notice")` reads as one thought."""
    return describe(app).get("state") or {}


def actions(app):
    """The action names an app publishes."""
    return [a.get("name") for a in describe(app).get("actions") or [] if isinstance(a, dict)]


# ── Talking to the mind ──────────────────────────────────────────────────────
#
# `yos` reaches apps and services. The companion is neither: it runs on a worker thread inside
# the shell and serves its own socket, and `companion.tool {name, args}` runs one of the mind's
# tools by name with no language model in the loop. That distinction is the point — a probe that
# went through a conversation would be measuring a model's willingness to pick the tool rather
# than the tool.
#
# This lived in `probes/one-calendar.py` while it had one caller. It is here now because the
# calendar has a second, and because `C:\Users\sync\tour-frames\wake.py` has been framing the
# same socket by hand for the same reason. Two copies of verify_calendar.py are why lib.py exists.

COMPANION_SOCK = SOCKET_DIR / "companion.sock"


class CompanionUnreachable(RuntimeError):
    """The shell is not serving the companion, so none of the mind's tools can be run."""


def companion_call(method, params, timeout=180):
    """One newline-delimited JSON-RPC round trip to `companion.sock`.

    The framing is the one every service on this machine uses: one JSON object, one newline, one
    JSON object back. The timeout is generous because the companion worker is a single lane — a
    tool call arriving while an answer is being generated waits for the whole answer.
    """
    if not COMPANION_SOCK.exists():
        raise CompanionUnreachable("no %s on this machine" % COMPANION_SOCK)
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    try:
        sock.connect(str(COMPANION_SOCK))
        request = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
        sock.sendall((request + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = sock.recv(65536)
            if not chunk:
                break
            buf += chunk
    except OSError as exc:
        raise CompanionUnreachable("%s: %s" % (COMPANION_SOCK, exc))
    finally:
        sock.close()
    if not buf.strip():
        raise CompanionUnreachable("the companion closed the connection without answering")
    return json.loads(buf.decode("utf-8", "replace"))


def companion_tool(name, args=None, timeout=180, tool_timeout_ms=90_000):
    """Run one of the mind's tools by name. Always a dict, whichever way it went.

        text     str          what the tool said
        error    str or None  why it could not be run at all

    A tool that refuses says so in `text`: these tools answer prose, which is what they were
    written to do, and the refusal is the thing under test. `error` is the transport or the
    registry — no such tool, the worker gone, no socket on this machine — and a check written
    against `text` must look at `error` first or it will read "the shell is not running" as
    "the tool declined".
    """
    try:
        reply = companion_call(
            "companion.tool",
            {"name": name, "args": args or {}, "timeout_ms": tool_timeout_ms},
            timeout=timeout,
        )
    except CompanionUnreachable as exc:
        return {"text": "", "error": str(exc)}
    except Exception as exc:  # noqa: BLE001 - a crash here is a result, not a stack trace
        return {"text": "", "error": "%s: %s" % (type(exc).__name__, exc)}
    if isinstance(reply, dict) and reply.get("error"):
        message = reply["error"]
        return {"text": "",
                "error": message.get("message") if isinstance(message, dict) else str(message)}
    result = (reply or {}).get("result") or {}
    return {"text": str(result.get("result", "")), "error": None}


# ── Processes and windows ────────────────────────────────────────────────────

def _run(argv, timeout=30):
    try:
        done = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except (FileNotFoundError, subprocess.TimeoutExpired) as exc:
        return "", str(exc), 127
    return done.stdout, done.stderr, done.returncode


def running(pattern):
    """The full command lines of the processes matching `pattern`, minus this one.

    Give it a path — `/opt/yantrik/bin/yantrik-calendar` — rather than a word. `pgrep -f
    calendar` also matches the probe file that is doing the asking.
    """
    out, _, _ = _run(["pgrep", "-af", pattern])
    mine = str(os.getpid())
    lines = []
    for line in out.strip().splitlines():
        pid = line.split(" ", 1)[0]
        if pid == mine:
            continue
        lines.append(line.strip())
    return lines


def pids(pattern):
    return [line.split(" ", 1)[0] for line in running(pattern)]


def kill_app(pattern, wait=2.0):
    """Stop everything matching `pattern` and wait for it to be gone.

    Returns the command lines that were killed, so a probe can show what it ended.
    """
    victims = running(pattern)
    if not victims:
        return []
    _run(["pkill", "-f", pattern])
    deadline = time.time() + wait + 6
    while time.time() < deadline:
        if not running(pattern):
            break
        time.sleep(0.3)
    else:
        _run(["pkill", "-9", "-f", pattern])
        time.sleep(1.0)
    return victims


def toplevels():
    """What the compositor says is mapped, one line per window.

    This is the strongest window witness available here and it is still not proof that a
    window is on screen. `wlrctl` reads the foreign-toplevel list, which says a surface
    exists and has a title; it says nothing about stacking, size, or whether anything was
    painted. The README says so and the gate repeats it.
    """
    out, _, code = _run(["wlrctl", "toplevel", "list"])
    if code != 0:
        return []
    return [line.strip() for line in out.strip().splitlines() if line.strip()]


def has_window(*words):
    """Is there a mapped toplevel whose line mentions any of these words?"""
    lines = [line.lower() for line in toplevels()]
    return any(any(word.lower() in line for line in lines) for word in words)


def wait_for(predicate, timeout=30.0, interval=0.4):
    """Poll until `predicate()` is truthy. Returns what it returned, or None."""
    deadline = time.time() + timeout
    while True:
        try:
            value = predicate()
        except Exception:  # noqa: BLE001 - a poll that throws is a poll that is not ready
            value = None
        if value:
            return value
        if time.time() >= deadline:
            return None
        time.sleep(interval)


class Waited:
    """What one bounded wait saw: whether it settled, what to, and how long it took.

    `wait_for` returns the value or `None`, which throws the waiting itself away. A probe
    that polls a deferred outcome — an action that `defers`, a thread the app started after
    it answered — has to be able to *say* that it polled, for how long, and what it was
    still seeing when it gave up. Otherwise a timeout disappears into whatever assertion
    comes next and is reported as the app being wrong about something else entirely.
    """

    __slots__ = ("what", "timeout", "value", "last", "seconds", "polls", "error")

    def __init__(self, what, timeout):
        self.what = what
        self.timeout = float(timeout)
        self.value = None   # the truthy thing the predicate finally returned
        self.last = None    # the last thing it returned, truthy or not
        self.seconds = 0.0
        self.polls = 0
        self.error = None   # the last exception a poll raised, if any

    @property
    def settled(self):
        return self.value is not None

    def __bool__(self):
        return self.settled

    def evidence(self, **extra):
        """This wait as a check's evidence. Extra keys are merged in beside it."""
        out = {
            "waited_for": self.what,
            "settled": self.settled,
            "waited_s": round(self.seconds, 1),
            "deadline_s": self.timeout,
            "polls": self.polls,
        }
        if self.settled:
            out["settled_as"] = self.value
        else:
            out["timed_out"] = True
            out["still_seeing"] = self.last
        if self.error is not None:
            out["last_poll_error"] = self.error
        out.update(extra)
        return out


def wait_until(predicate, timeout=30.0, what=None, interval=0.4):
    """Poll until `predicate()` is truthy, keeping a record of the waiting.

    The same loop as `wait_for`, returning a `Waited` rather than a bare value, so a probe
    can report the timeout as its own failure with the wait as its evidence instead of
    letting it fall through into the next assertion:

        settled = wait_until(lambda: checksum() in ("pass", "fail"), timeout=60,
                             what="the checksum to settle out of `verifying`")
        probe.check("the checksum settles inside the deadline", bool(settled),
                    evidence=settled.evidence(row=row(id)))

    `what` finishes the sentence "waited for ...". It is written into the evidence, so the
    report says what was being waited on and not merely that something was.
    """
    record = Waited(what or "a condition the probe did not name", timeout)
    started = time.time()
    deadline = started + float(timeout)
    while True:
        record.polls += 1
        try:
            value = predicate()
            record.error = None
        except Exception as exc:  # noqa: BLE001 - a poll that throws is a poll that is not ready
            value = None
            record.error = "%s: %s" % (type(exc).__name__, exc)
        record.last = value
        if value:
            record.value = value
            record.seconds = time.time() - started
            return record
        if time.time() >= deadline:
            record.seconds = time.time() - started
            return record
        time.sleep(interval)


def surface_up(app):
    """True when the app's control surface answers and names itself.

    A socket file outlives the process that made it, so the file existing proves nothing:
    `app-calendar.sock` is on this machine right now with no calendar behind it. Asking
    the surface who it is, is the cheapest honest test.
    """
    view = describe(app)
    if "unreachable" in view:
        return False
    return view.get("app") == app or bool(view.get("summary"))


def open_app(name, wait_for_socket=True, expect_process=None, window_words=(), timeout=45.0):
    """Ask the shell to open an app and report what actually happened.

    Returns the evidence rather than a boolean, because the audit's whole finding was that
    the boolean lied: `open_app name=image-viewer` answered `accepted: true` with no
    process and no window.

        {"accepted": ..., "surface_up": ..., "processes": [...],
         "windows": [...], "failed_launches": [...], "seconds": 4.1}

    `failed_launches` is the shell's own running list and it is not reset between probes:
    killing an app — which the restart check has to do — leaves a `signal: 15 (SIGTERM)`
    entry behind, and the next probe would read it as its own launch failing. So the
    entries present before the launch are recorded, and `new_failed_launches` holds only
    what this launch added. A probe asserts on that, never on the whole list.
    """
    started = time.time()
    failed_before = state("shell").get("failed_launches") or []
    asked = act("shell", "open_app", name=name)
    if wait_for_socket:
        wait_for(lambda: surface_up(name), timeout=timeout)
    if expect_process:
        wait_for(lambda: running(expect_process), timeout=max(5.0, timeout / 3))
    # A window is mapped a beat after the surface answers; give it that beat.
    if window_words:
        wait_for(lambda: has_window(*window_words), timeout=10.0)
    shell_state = state("shell")
    failed_after = shell_state.get("failed_launches") or []
    remaining = [json.dumps(f, sort_keys=True, default=str) for f in failed_before]
    new_failed = []
    for entry in failed_after:
        key = json.dumps(entry, sort_keys=True, default=str)
        if key in remaining:
            remaining.remove(key)
        else:
            new_failed.append(entry)
    return {
        "accepted": asked.get("accepted"),
        "refused": asked.get("refused"),
        "surface_up": surface_up(name),
        "processes": running(expect_process) if expect_process else [],
        "windows": [w for w in toplevels() if not window_words or any(
            word.lower() in w.lower() for word in window_words)],
        "all_windows": toplevels(),
        "failed_launches": failed_after,
        "new_failed_launches": new_failed,
        "new_failed_launches_for_this_app": [f for f in new_failed
                                             if str(f.get("app") if isinstance(f, dict) else f)
                                             == name],
        "seconds": round(time.time() - started, 1),
    }


def spawn(argv, env_extra=None):
    """Start a binary in the session's Wayland environment, detached from this probe."""
    env = dict(os.environ)
    env.setdefault("XDG_RUNTIME_DIR", str(RUNTIME_DIR))
    env.setdefault("WAYLAND_DISPLAY", "wayland-0")
    env.update(env_extra or {})
    return subprocess.Popen(argv, env=env, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, start_new_session=True)


def run_and_wait(argv, timeout=30, env_extra=None):
    """Run a binary to completion in the session environment. For handover launches."""
    env = dict(os.environ)
    env.setdefault("XDG_RUNTIME_DIR", str(RUNTIME_DIR))
    env.setdefault("WAYLAND_DISPLAY", "wayland-0")
    env.update(env_extra or {})
    try:
        done = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, env=env)
    except subprocess.TimeoutExpired:
        return {"exit": None, "note": "still running after %ds" % timeout}
    return {"exit": done.returncode, "stderr": done.stderr.strip()[-400:]}


def sha256(path):
    path = pathlib.Path(path)
    if not path.exists():
        return None
    return hashlib.sha256(path.read_bytes()).hexdigest()


# ── Putting the machine back ─────────────────────────────────────────────────

MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024


class Preserved:
    """A directory or file remembered on the way in and restored on the way out.

    Files larger than `MAX_SNAPSHOT_BYTES` are remembered by name and size only; if one of
    those is modified the restore cannot put the old bytes back, and `differences()` says
    so rather than pretending. Probes here only ever touch small files.
    """

    def __init__(self, path, max_bytes=MAX_SNAPSHOT_BYTES):
        self.path = pathlib.Path(path)
        self.max_bytes = max_bytes
        self.before = {}
        self.after = {}
        self.existed = False
        self.was_dir = False
        self.unrestorable = []
        self._hook = None

    # -- snapshot --------------------------------------------------------
    def _read(self):
        snap = {}
        if self.path.is_dir():
            for entry in sorted(self.path.iterdir()):
                if entry.is_dir():
                    snap[entry.name] = {"kind": "dir"}
                    continue
                size = entry.stat().st_size
                record = {"kind": "file", "size": size}
                if size <= self.max_bytes:
                    record["bytes"] = entry.read_bytes()
                snap[entry.name] = record
        elif self.path.exists():
            size = self.path.stat().st_size
            record = {"kind": "file", "size": size}
            if size <= self.max_bytes:
                record["bytes"] = self.path.read_bytes()
            snap[self.path.name] = record
        return snap

    def listing(self, snap=None):
        """A snapshot as a person would read it: `name (size bytes)`, sorted."""
        snap = self.before if snap is None else snap
        return ["%s (%s)" % (name, "dir" if rec["kind"] == "dir" else "%d bytes" % rec["size"])
                for name, rec in sorted(snap.items())]

    def differences(self):
        added = sorted(set(self.after) - set(self.before))
        removed = sorted(set(self.before) - set(self.after))
        changed = sorted(n for n in set(self.before) & set(self.after)
                         if self.before[n].get("size") != self.after[n].get("size"))
        out = {}
        if added:
            out["added"] = added
        if removed:
            out["removed"] = removed
        if changed:
            out["changed"] = changed
        if self.unrestorable:
            out["could_not_restore"] = self.unrestorable
        return out

    # -- lifecycle -------------------------------------------------------
    def __enter__(self):
        self.existed = self.path.exists()
        self.was_dir = self.path.is_dir()
        self.before = self._read()
        self._hook = self.restore
        atexit.register(self._hook)
        return self

    def __exit__(self, *_exc):
        self.restore()
        if self._hook is not None:
            atexit.unregister(self._hook)
            self._hook = None
        return False

    def restore(self):
        """Delete what appeared, put back what vanished or changed. Idempotent.

        A path that was not there when the block was entered is removed entirely, so a
        probe that makes its own scratch directory leaves none behind.
        """
        try:
            if not self.existed:
                if self.path.is_dir():
                    shutil.rmtree(self.path, ignore_errors=True)
                elif self.path.exists():
                    self.path.unlink(missing_ok=True)
                return
            if self.was_dir:
                self.path.mkdir(parents=True, exist_ok=True)
                for entry in list(self.path.iterdir()):
                    if entry.name in self.before:
                        continue
                    if entry.is_dir():
                        shutil.rmtree(entry, ignore_errors=True)
                    else:
                        entry.unlink(missing_ok=True)
                for name, record in self.before.items():
                    target = self.path / name
                    if record["kind"] == "dir":
                        target.mkdir(parents=True, exist_ok=True)
                        continue
                    if "bytes" not in record:
                        if not target.exists():
                            self.unrestorable.append(name)
                        continue
                    if not target.exists() or target.read_bytes() != record["bytes"]:
                        target.write_bytes(record["bytes"])
            else:
                record = next(iter(self.before.values()), None)
                if record and "bytes" in record:
                    self.path.parent.mkdir(parents=True, exist_ok=True)
                    self.path.write_bytes(record["bytes"])
                elif not self.path.exists():
                    self.unrestorable.append(self.path.name)
        except Exception as exc:  # noqa: BLE001 - restoration must not throw on the way out
            self.unrestorable.append("%s: %s" % (type(exc).__name__, exc))
        finally:
            self.after = self._read()


def preserved(path, max_bytes=MAX_SNAPSHOT_BYTES):
    """`with preserved("~/Pictures") as pics:` — snapshot in, restore out."""
    return Preserved(pathlib.Path(path).expanduser(), max_bytes)


class MovedAside:
    """Rename a file away to force a failure, and put it back no matter what.

    Forcing a failure is the only way to check contract point 4, and the only way this
    suite does it on a shared machine. The rename is `sudo -n mv`, one file, in
    `/opt/yantrik/bin`, and the restore is registered with `atexit` before the move
    happens — so a crash, a `SystemExit`, or the runner's SIGTERM on a timeout all put the
    binary back. Nothing else in this suite writes to `/opt`.
    """

    def __init__(self, path, suffix=".conformance-hidden"):
        self.path = pathlib.Path(path)
        self.hidden = pathlib.Path(str(self.path) + suffix)
        self.moved = False
        self._hook = None

    def __enter__(self):
        if not self.path.exists():
            raise FileNotFoundError("nothing to move aside at %s" % self.path)
        self._hook = self.restore
        atexit.register(self._hook)
        out, err, code = _run(["sudo", "-n", "mv", str(self.path), str(self.hidden)])
        if code != 0:
            atexit.unregister(self._hook)
            self._hook = None
            raise RuntimeError("could not move %s aside: %s" % (self.path, (err or out).strip()))
        self.moved = True
        return self

    def __exit__(self, *_exc):
        self.restore()
        if self._hook is not None:
            atexit.unregister(self._hook)
            self._hook = None
        return False

    def restore(self):
        if not self.moved:
            return
        if self.path.exists() and not self.hidden.exists():
            self.moved = False
            return
        out, err, code = _run(["sudo", "-n", "mv", str(self.hidden), str(self.path)])
        if code == 0:
            self.moved = False
        else:  # last resort: say it loudly on stderr, which the runner captures
            print("CONFORMANCE: FAILED TO RESTORE %s (%s)" % (self.path, (err or out).strip()),
                  file=sys.stderr)


def moved_aside(path):
    return MovedAside(path)


# ── Recording what was found ─────────────────────────────────────────────────

REQUIRED = "required"
ADVISORY = "advisory"


class Probe:
    """The report a probe writes, and the exit code the runner reads.

    A check records and returns; it does not raise. One app failing point 2 must still be
    measured against points 3, 4 and 6 — otherwise a fix chases one assertion at a time.
    An `advisory` check is reported and does not fail the probe; use it for something the
    suite cannot prove on this machine, never to soften a contract point.

    Used as a context manager, so the report is written even when the body throws:

        with Probe("calendar", ONE_JOB) as probe:
            probe.check("it opens", ..., contract=1, evidence={...})
    """

    def __init__(self, app, one_job, argv=None):
        self.app = app
        self.one_job = one_job
        self.checks = []
        self.notes = {}
        self.started = time.time()
        self.argv = list(argv or sys.argv[1:])

    def check(self, name, passed, evidence=None, severity=REQUIRED, contract=None):
        record = {"name": name, "passed": bool(passed), "severity": severity}
        if contract is not None:
            record["contract"] = contract
        record["evidence"] = _jsonable(evidence if evidence is not None else {})
        self.checks.append(record)
        return bool(passed)

    def note(self, key, value):
        """Evidence that is not itself a check: a before listing, a screenshot path."""
        self.notes[key] = _jsonable(value)

    @property
    def passed(self):
        return not any(c["severity"] == REQUIRED and not c["passed"] for c in self.checks)

    def report(self):
        return {
            "app": self.app,
            "one_job": self.one_job,
            "passed": self.passed,
            "checks": self.checks,
            "failed": [c["name"] for c in self.checks if not c["passed"]],
            "counts": {
                "passed": sum(1 for c in self.checks if c["passed"]),
                "total": len(self.checks),
                "advisory_failed": sum(1 for c in self.checks
                                       if not c["passed"] and c["severity"] == ADVISORY),
            },
            "notes": self.notes,
            "duration_s": round(time.time() - self.started, 1),
        }

    def finish(self):
        print(json.dumps(self.report(), indent=2, default=str))
        sys.exit(0 if self.passed else 1)

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, _tb):
        if exc_type is not None and not issubclass(exc_type, SystemExit):
            import traceback
            self.check("the probe ran to the end", False, severity=REQUIRED,
                       evidence={"exception": "%s: %s" % (exc_type.__name__, exc),
                                 "traceback": traceback.format_exc().splitlines()[-8:]})
        elif exc_type is not None:
            self.check("the probe ran to the end", False, severity=REQUIRED,
                       evidence={"exit": str(exc)})
        self.finish()
        return True


def _jsonable(value):
    """Anything a check wants to record, made safe for `json.dumps`."""
    if isinstance(value, Waited):
        # A wait handed straight to `check(evidence=...)` records as its own account of
        # itself rather than as `<lib.Waited object at 0x...>`.
        return _jsonable(value.evidence())
    if isinstance(value, dict):
        return {str(k): _jsonable(v) for k, v in value.items()}
    if isinstance(value, (list, tuple, set)):
        return [_jsonable(v) for v in value]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    if isinstance(value, bytes):
        return value.decode("utf-8", "replace")
    return str(value)
