"""The machine under test: running things as the person, reading the desktop, guarding against a real desktop.

Everything the battery does to the desktop goes through this class, so the unit tests can swap it for a fake.
"""
import datetime
import json
import os
import subprocess
import time

MARKER = "/etc/yantrik/test-machine"
MARKER_TEXT = "yantrik-test-machine"
QUIET_START = (3, 0)
QUIET_END = (4, 30)


class NotATestMachine(RuntimeError):
    """The marker file is missing or not root's: refuse to touch this desktop."""


def in_quiet_window(now: datetime.datetime) -> bool:
    """03:00-04:30 in the machine's own clock (the VMs run on America/Chicago)."""
    t = (now.hour, now.minute)
    return QUIET_START <= t < QUIET_END


def seconds_to_quiet_end(now: datetime.datetime) -> int:
    end = now.replace(hour=QUIET_END[0], minute=QUIET_END[1], second=0, microsecond=0)
    return max(0, int((end - now).total_seconds()))


def parse_yos_json(text: str):
    """The JSON object a `yos act` / `yos describe` answer carries after its header lines, or None."""
    dec = json.JSONDecoder()
    pos = text.find("\n{")
    while pos != -1:
        try:
            obj, _ = dec.raw_decode(text[pos + 1:])
            return obj
        except ValueError:
            pos = text.find("\n{", pos + 1)
    if text.lstrip().startswith("{"):
        try:
            return json.loads(text)
        except ValueError:
            return None
    return None


class Machine:
    def __init__(self, user="tester", marker=MARKER, yos="/opt/yantrik/bin/yos"):
        self.user = user
        self.marker = marker
        self.yos_bin = yos
        import pwd
        info = pwd.getpwnam(user)
        self.uid = info.pw_uid
        self.home = info.pw_dir

    # -- guard ---------------------------------------------------------------------------------
    def assert_test_machine(self):
        try:
            st = os.stat(self.marker)
            text = open(self.marker).read()
        except OSError:
            raise NotATestMachine(f"{self.marker} is missing: this is not marked as a test machine, refusing to run")
        if st.st_uid != 0 or st.st_mode & 0o022 or MARKER_TEXT not in text:
            raise NotATestMachine(f"{self.marker} must be root's, not group/world writable, and contain '{MARKER_TEXT}'")
        if os.path.exists("/run/live"):
            raise NotATestMachine("this is a live session")

    # -- running things ------------------------------------------------------------------------
    def env(self):
        return {"XDG_RUNTIME_DIR": f"/run/user/{self.uid}", "WAYLAND_DISPLAY": "wayland-0",
                "DBUS_SESSION_BUS_ADDRESS": f"unix:path=/run/user/{self.uid}/bus"}

    def person_cmd(self, argv):
        env = [f"{k}={v}" for k, v in self.env().items()]
        if os.geteuid() == 0:
            return ["runuser", "-u", self.user, "--", "env", *env, *argv]
        return ["env", *env, *argv]

    def run(self, argv, timeout=60, person=True, stdin=None):
        cmd = self.person_cmd(argv) if person else list(argv)
        try:
            p = subprocess.run(cmd, capture_output=True, timeout=timeout, input=stdin)
            return p.returncode, p.stdout, p.stderr
        except subprocess.TimeoutExpired as e:
            return 124, e.stdout or b"", e.stderr or b""

    def text(self, argv, timeout=60, person=True) -> str:
        rc, out, _ = self.run(argv, timeout, person)
        return out.decode("utf-8", "replace")

    # -- the desktop's control surface (read-only unless an action is named) -------------------
    def describe(self, app="shell"):
        out = self.text([self.yos_bin, "describe", app, "--json"], timeout=60)
        obj = parse_yos_json(out) or {}
        return obj.get("state", obj) if isinstance(obj, dict) else {}

    def act(self, app, action, timeout=90, **args):
        argv = [self.yos_bin, "act", app, action] + [f"{k}={v}" for k, v in args.items()]
        out = self.text(argv, timeout)
        return out, parse_yos_json(out)

    def shell_state(self) -> dict:
        d = self.describe("shell")
        return {
            "screen": d.get("screen"), "in_front": d.get("in_front"),
            "windows": [w.get("title") for w in d.get("windows", [])],
            "dnd": d.get("do_not_disturb"), "approvals": (d.get("bar_minds") or {}).get("approvals", 0),
            "pending": d.get("pending_approvals") or [], "status": d.get("companion_status"),
            "answering": [m.get("id") for m in d.get("minds", []) if m.get("answering")],
            "conversation": d.get("conversation") or [],
        }

    # -- the display ---------------------------------------------------------------------------
    def grim_ppm(self):
        rc, out, _ = self.run(["grim", "-t", "ppm", "-"], timeout=30)
        if rc != 0 or not out.startswith(b"P6"):
            return None
        head = out.split(b"\n", 3)
        w, h = map(int, head[1].split())
        return w, h, head[3]

    def click(self, x, y):
        self.run(["wlrctl", "pointer", "move", "-20000", "-20000"], timeout=15)
        self.run(["wlrctl", "pointer", "move", str(int(x)), str(int(y))], timeout=15)
        time.sleep(0.4)
        self.run(["wlrctl", "pointer", "click", "left"], timeout=15)

    def now(self):
        return datetime.datetime.now()
