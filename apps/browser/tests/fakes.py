"""Stand-ins for the shell's files_stat and for a browser, for the upload tests."""

import os
import stat as st
import threading

import conftest  # noqa: F401 - the package and the SDK on the path, for `unittest discover` too


def files_stat(home):
    """A stand-in for the shell's `files_stat` (yantrik_ipc_contracts::home_paths::stat) over
    `home`, answering in its shape. The protected names here are only the ones the tests use:
    the real list is the shell's, and these tests are about what upload does with its answer."""
    protected = {".ssh", ".gnupg", ".aws"}

    def stat(path):
        if not path.startswith(home + "/"):
            return {"path": path, "exists": "unknown", "reason": "outside"}
        parts = path[len(home) + 1:].split("/")
        if any(p.lower() in protected or p.lower().endswith((".key", ".pem")) for p in parts):
            return {"path": path, "exists": "unknown", "reason": "protected"}
        via_link, probe = False, home
        for p in parts:
            probe = os.path.join(probe, p)
            if os.path.islink(probe):
                via_link = True
        real = os.path.realpath(path)
        if not os.path.exists(real):
            return {"path": path, "exists": False, "reason": "not_found"}
        info = os.stat(real)
        if st.S_ISREG(info.st_mode) and info.st_nlink > 1:
            return {"path": path, "exists": "unknown", "reason": "hard_link"}
        kind = "file" if st.S_ISREG(info.st_mode) else "directory" if st.S_ISDIR(info.st_mode) else "other"
        return {"path": path, "exists": True, "kind": kind, "size": info.st_size, "real": real,
                "via_link": via_link}
    return stat


class Tab:
    def __init__(self, url):
        self.id = "TAB0123456789"
        self.url = url
        self.title = "Settings"
        self.session = "S1"
        self.world = 7
        self.dialog = None
        self.inflight = {}
        self.last_network = 0.0
        self.loaded = True
        self.navigations = 0
        self.opened_by = None
        self.chooser = None
        self.main_frame = "F0"


class Browser:
    """Just enough of cdp.Browser for upload: one tab, one control, and a record of every
    command, so a test can say exactly whether a file was put into the page."""

    def __init__(self, url="https://github.com/settings/profile", control=None):
        self.cond = threading.Condition()
        self.alive = True
        self.t = Tab(url)
        self.active = self.t
        self.calls = []
        self.control = control or {"ref": "e5", "role": "button", "name": "Upload a photo…",
                                   "direct": True, "accepts": "image/*"}
        self.files = []

    @property
    def set_files(self):
        return [c[1] for c in self.calls if c[0] == "DOM.setFileInputFiles"]

    def connect(self):
        pass

    def tab(self, tab_id=None):
        return self.t

    def pages(self):
        return [self.t]

    def run(self, tab, fn, *args, timeout=None):
        if fn == "fileTarget":
            return {"gone": True} if args[0] != self.control["ref"] else dict(self.control)
        if fn == "target":
            return {"ref": args[0], "role": self.control["role"], "name": self.control["name"],
                    "x": 10, "y": 10}
        if fn == "snapshot":
            return {"url": self.t.url, "title": self.t.title, "elements": []}
        if fn == "where":
            return {"url": self.t.url, "title": self.t.title}
        if fn == "changes":
            return {"appeared": []}
        return True

    def element(self, tab, fn, *args):
        return "obj-1"

    def call(self, method, params=None, session=None, timeout=None, until=None):
        self.calls.append((method, dict(params or {})))
        if method == "DOM.setFileInputFiles":
            self.files = [os.path.basename(f) for f in params["files"]]
        if method == "Input.dispatchMouseEvent" and params.get("type") == "mouseReleased":
            self.t.chooser = {"mode": "selectSingle", "node": 42}
        if method == "Runtime.callFunctionOn":
            return {"result": {"value": list(self.files)}}
        if method == "DOM.resolveNode":
            return {"object": {"objectId": "obj-2"}}
        return {}
