"""What the SDK's tests share: the path to the package, a private machine, and the Rust source.

Every test runs against a machine of its own — `HOME` and `XDG_RUNTIME_DIR` in a temporary
directory — so the ceiling, the mode and the socket directory are the test's, never the
developer's. And the refusal sentences are not copied into these tests by hand: `rust()` reads
the Rust source they are ported from, and `render()` fills a Rust format string the way
`format!` would, so the expected sentence IS the Rust one. When the Rust wording changes, the
fragment is no longer found and the test says which.
"""

import json
import os
import re
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PACKAGE_ROOT = os.path.abspath(os.path.join(HERE, ".."))
REPO = os.path.abspath(os.path.join(PACKAGE_ROOT, "..", ".."))
if PACKAGE_ROOT not in sys.path:
    sys.path.insert(0, PACKAGE_ROOT)

import yantrik_surface  # noqa: E402,F401
from yantrik_surface import wire  # noqa: E402

# The dispatch this port keeps: the `yantrik-surface` crate (piece B of the SDK design), and the
# window's hop over it in `yantrik-app-runtime::control`. Read as one text, because a sentence the
# port quotes may live in either.
RUST_CONTROL = ("crates/yantrik-surface/src", "crates/yantrik-app-runtime/src/control.rs")
RUST_GATE = "crates/yantrik-ipc-transport/src/gate.rs"
RUST_SERVER = "crates/yantrik-ipc-transport/src/server.rs"
RUST_CONTRACTS = "crates/yantrik-ipc-contracts/src/control_surface.rs"
# The `yantrik-surface` crate (piece B of the SDK design): typed arguments. Quoted when present.
RUST_SURFACE_ARGS = "crates/yantrik-surface/src/args.rs"
# Owned names and the shell-peer rule (piece A of the SDK design).
RUST_OWNER = "crates/yantrik-ipc-transport/src/owner.rs"
# The pieces `gate::grant_refusal` builds its four sentences from (quoted where they are used).
GATE_HOW = ("Ask the shell for approval first (`request_approval` with this app, action and these "
            "exact arguments, poll `approval_status`, then send the granted request_id as `grant` "
            "on app.act — `yos act` does all of that for you), or have the person at the machine "
            "press Allow when the card appears.")
GATE_PLAN = ("Say what you would do and let the person decide; they switch the mode from the chip "
             "in the status bar.")
GATE_FINAL_WORD = "its own description says it cannot be undone"
YOS = os.path.join(REPO, "deploy", "yantrik-os", "yos")
# The example lives with the Rust one at the top of the repository (docs/sdk quotes it).
EXAMPLE = os.path.join(REPO, "examples", "hello_surface.py")

_sources = {}


def _files(path):
    """The `.rs` files `path` names: one file, every file under a directory, or each of a tuple."""
    if isinstance(path, tuple):
        return [f for p in path for f in _files(p)]
    full = os.path.join(REPO, path)
    if os.path.isdir(full):
        return sorted(os.path.join(root, name) for root, _, names in os.walk(full)
                      for name in names if name.endswith(".rs"))
    return [full] if os.path.isfile(full) else []


def rust(path):
    """Rust source with its string continuations joined (`\\` at a line end drops the newline
    and the next line's leading whitespace, as rustc does), or None outside the repo. `path` is a
    file, a directory of them, or a tuple of either, read as one text."""
    if path not in _sources:
        files = _files(path)
        if not files:
            _sources[path] = None
        else:
            text = []
            for full in files:
                with open(full, encoding="utf-8") as f:
                    text.append(re.sub(r"\\\n\s*", "", f.read()))
            _sources[path] = "\n".join(text)
    return _sources[path]


def quoted(case, path, fragment, skip=True):
    """Fail unless `fragment` is in the Rust source at `path`. When the source is not here (the
    package tested on its own, outside this repository; or a crate not landed yet) the test is
    skipped — or, with `skip=False`, the quote alone is let go and the rest of the test runs."""
    source = rust(path)
    if source is None:
        if not skip:
            return
        case.skipTest("%s is not in this tree, so the wording cannot be checked against it" % path)
    case.assertTrue(fragment in source,
                    "the Rust wording this port quotes is no longer in %s — it changed there, so "
                    "change the port and this test with it: %r" % (path, fragment))


def render(fragment, *positional, **named):
    """Fill a Rust format string: `{}` from `positional` in order, `{name}` from `named`."""
    values = iter(positional)

    def fill(match):
        key = match.group(1)
        return str(next(values) if key == "" else named[key])

    return re.sub(r"\{(\w*)\}", fill, fragment)


class Machine:
    """A private HOME (settings, mode) and runtime dir (sockets) for one test."""

    def __init__(self, ceiling=None, mode=None):
        self.tmp = tempfile.TemporaryDirectory(prefix="yantrik-sdk-")
        self.home = os.path.join(self.tmp.name, "home")
        self.runtime = os.path.join(self.tmp.name, "run")
        os.makedirs(os.path.join(self.home, ".config", "yantrik"))
        os.makedirs(self.runtime)
        self.settings = os.path.join(self.home, ".config", "yantrik", "settings.yaml")
        self.mode_file = os.path.join(self.home, ".config", "yantrik", "mind-mode.json")
        self.saved = {}
        if ceiling is not None:
            self.set_ceiling(ceiling)
        if mode is not None:
            self.set_mode(mode)

    def set_ceiling(self, ceiling):
        with open(self.settings, "w", encoding="utf-8") as f:
            f.write("theme: dark\ntool_permission: %s\n" % ceiling)

    def set_mode(self, mode, rules=()):
        doc = mode if isinstance(mode, dict) else {
            "mode": mode, "session_rules": [{"app": a, "action": x} for a, x in rules]}
        with open(self.mode_file, "w", encoding="utf-8") as f:
            json.dump(doc, f)

    def __enter__(self):
        for key, value in (("HOME", self.home), ("XDG_RUNTIME_DIR", self.runtime)):
            self.saved[key] = os.environ.get(key)
            os.environ[key] = value
        return self

    def __exit__(self, *exc):
        for key, value in self.saved.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
        self.tmp.cleanup()
        return False

    @property
    def socket_dir(self):
        return os.path.join(self.runtime, "yantrik")

    def env(self):
        """The environment a subprocess on this machine gets."""
        env = dict(os.environ)
        env["HOME"] = self.home
        env["XDG_RUNTIME_DIR"] = self.runtime
        env["PYTHONPATH"] = PACKAGE_ROOT + os.pathsep + env.get("PYTHONPATH", "")
        return env


class ShellStandIn:
    """`fake_shell.py` serving `app-shell.sock` on `machine`, as a process whose program is a
    `yantrik-ui` binary: a copy of this interpreter under that name. The shell-peer rule (in
    this package and in `yos`) is met as the real shell meets it, unpatched."""

    def __init__(self, machine):
        self.machine = machine
        self.dir = tempfile.mkdtemp(prefix="yantrik-shell-")
        self.exe = os.path.join(self.dir, "yantrik-ui")
        self.path = os.path.join(machine.socket_dir, "app-shell.sock")
        self.process = None

    def start(self, case):
        import shutil
        import subprocess
        import time

        try:
            shutil.copy2(os.path.realpath(sys.executable), self.exe)
        except OSError as e:
            case.skipTest("cannot copy this interpreter to stand in for yantrik-ui: %s" % e)
        env = self.machine.env()
        env["PYTHONHOME"] = sys.base_prefix
        self.process = subprocess.Popen([self.exe, os.path.join(HERE, "fake_shell.py")],
                                        env=env, stderr=subprocess.PIPE, text=True)
        case.addCleanup(self.stop)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                case.skipTest("the stand-in shell would not start under a copied interpreter: %s"
                              % self.process.stderr.read()[-300:])
            if wire.answers(self.path):
                return self
            time.sleep(0.05)
        case.fail("the stand-in shell never answered on %s" % self.path)

    def state(self):
        return wire.call_once(self.path, "app.describe", {})["result"]["state"]

    def stop(self):
        import shutil

        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.communicate(timeout=10)
            except Exception:  # noqa: BLE001 - a stand-in that will not stop is killed
                self.process.kill()
        if self.process is not None and self.process.stderr:
            self.process.stderr.close()
        shutil.rmtree(self.dir, ignore_errors=True)


class MachineCase(unittest.TestCase):
    """A test on a machine of its own: the ceiling open, the mode `bypass_all` (full bypass,
    which asks about nothing, not even what cannot be undone), unless a test pins them — the
    tests about everything except the gate do what the runtime's tests do."""

    ceiling = "dangerous"
    mode = "bypass_all"

    def setUp(self):
        self.machine = Machine(self.ceiling, self.mode)
        self.machine.__enter__()
        self.addCleanup(self.machine.__exit__, None, None, None)

    def refusal(self, fn, code=wire.RPC_INVALID_PARAMS):
        """Run fn, expect a JSON-RPC error with `code`, return its message."""
        with self.assertRaises(wire.RpcError) as caught:
            fn()
        self.assertEqual(caught.exception.code, code, caught.exception.message)
        return caught.exception.message
