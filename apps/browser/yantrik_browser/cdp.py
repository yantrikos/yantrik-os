"""One DevTools connection to the desktop's browser, held for as long as the browser runs.

`yos web` opened a websocket per command and evaluated a script in the page's own world: a new
connection per read, and a registry of elements any page script could see and rewrite. This is
the other way round. One browser-level connection, sessions attached per tab in flat mode, and
the reader (page.js) evaluated in an isolated world — the page's DOM, none of its JavaScript.

Thread-safe: calls come from the surface's connection threads; one reader thread answers them
and keeps the event state (tabs, dialogs, requests in flight) the waits read.
"""

import json
import os
import threading
import time
import urllib.error
import urllib.request

DEFAULT_ADDRESS = "http://127.0.0.1:9222"
WORLD = "yantrik-browser"
CALL_TIMEOUT = 20.0

_PAGE_JS = None


def page_js():
    global _PAGE_JS
    if _PAGE_JS is None:
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "page.js"),
                  encoding="utf-8") as f:
            _PAGE_JS = f.read()
    return _PAGE_JS


class BrowserClosed(Exception):
    """No browser this desktop can drive is answering."""


class CdpError(Exception):
    """The browser refused a command."""


class Tab:
    """What the connection knows about one page target."""

    def __init__(self, target_id, url="", title=""):
        self.id = target_id
        self.url = url
        self.title = title
        self.session = None
        self.world = None          # executionContextId of the isolated world in the top frame
        self.main_frame = None
        self.dialog = None         # {type, message, default} while a JS dialog is open
        self.inflight = {}         # requestId -> started (monotonic)
        self.last_network = time.monotonic()
        self.loaded = True
        self.navigations = 0
        self.opened_by = None      # the tab this one was opened from, if any


class Browser:
    def __init__(self, address=None):
        self.address = (address or os.environ.get("YANTRIK_BROWSER_CDP") or DEFAULT_ADDRESS).rstrip("/")
        self.ws = None
        self.lock = threading.Lock()          # one writer
        self.cond = threading.Condition()     # replies and events
        self.replies = {}
        self.abandoned = set()                 # ids whose answer nobody waits for any more
        self.generation = 0                    # which connection is the current one
        self.ids = 0
        self.tabs = {}                         # target id -> Tab, in the order they appeared
        self.by_session = {}
        self.active = None                     # the tab actions go to unless told otherwise
        self.reader = None
        self.alive = False

    # ── the connection ──

    def _version(self):
        try:
            with urllib.request.urlopen(self.address + "/json/version", timeout=3) as r:
                return json.load(r)
        except (urllib.error.URLError, OSError, ValueError):
            raise BrowserClosed() from None

    def connect(self):
        """Connect if not connected. Raises BrowserClosed when no browser answers."""
        if self.alive:
            return
        try:
            from websocket import create_connection
        except ImportError:
            raise CdpError("python3-websocket is not installed") from None
        url = self._version().get("webSocketDebuggerUrl")
        if not url:
            raise BrowserClosed()
        # The origin the desktop's launcher allows (wire/dock.rs, --remote-allow-origins): its own
        # debugging address and nothing wider.
        self.close()
        ws = create_connection(url, origin=self.address, timeout=None, suppress_origin=False)
        with self.cond:
            self.generation += 1
            generation = self.generation
            self.ws = ws
            self.alive = True
            self.replies.clear()
            self.abandoned.clear()
        self.tabs.clear()
        self.by_session.clear()
        self.active = None
        self.reader = threading.Thread(target=self._read, args=(ws, generation), name="cdp-reader",
                                       daemon=True)
        self.reader.start()
        self.call("Target.setDiscoverTargets", {"discover": True})
        for info in self.call("Target.getTargets").get("targetInfos", []):
            self._saw_target(info)

    def close(self):
        with self.cond:
            self.alive = False
            ws, self.ws = self.ws, None
            self.cond.notify_all()
        try:
            if ws:
                ws.close()
        except Exception:  # noqa: BLE001
            pass

    def _read(self, ws, generation):
        """The reader of one connection. It reads its own socket, never `self.ws`, and on its way
        out marks the browser gone only if its connection is still the current one — a reader
        left behind by a reconnect cannot end the connection that replaced it."""
        while True:
            try:
                raw = ws.recv()
            except Exception:  # noqa: BLE001 - the browser went away
                break
            if not raw:
                continue
            try:
                msg = json.loads(raw)
            except ValueError:
                continue
            with self.cond:
                if generation != self.generation:
                    return
                if "id" in msg:
                    if msg["id"] in self.abandoned:
                        self.abandoned.discard(msg["id"])
                    else:
                        self.replies[msg["id"]] = msg
                else:
                    self._event(msg)
                self.cond.notify_all()
        with self.cond:
            if generation == self.generation:
                self.alive = False
            self.cond.notify_all()

    def call(self, method, params=None, session=None, timeout=CALL_TIMEOUT, until=None):
        """Send one command and wait for its answer. `until`, when given, is checked while
        waiting: once it is true the wait ends and None is the answer — for input that opens a
        JavaScript dialog, which holds the page (and so the answer) until someone answers it."""
        if not self.alive:
            raise BrowserClosed()
        with self.lock:
            self.ids += 1
            mid = self.ids
            msg = {"id": mid, "method": method, "params": params or {}}
            if session:
                msg["sessionId"] = session
            try:
                self.ws.send(json.dumps(msg))
            except Exception:  # noqa: BLE001
                self.alive = False
                raise BrowserClosed() from None
        deadline = time.monotonic() + timeout
        with self.cond:
            while mid not in self.replies:
                if not self.alive:
                    raise BrowserClosed()
                if until is not None and until():
                    self.abandoned.add(mid)
                    return None
                left = deadline - time.monotonic()
                if left <= 0:
                    self.abandoned.add(mid)
                    raise CdpError("the browser did not answer %s in %d s" % (method, timeout))
                self.cond.wait(min(left, 0.2) if until is not None else left)
            reply = self.replies.pop(mid)
        if "error" in reply:
            raise CdpError("%s: %s" % (method, reply["error"].get("message", "refused")))
        return reply.get("result", {})

    # ── events ──

    def _saw_target(self, info):
        if info.get("type") != "page":
            return
        tab = self.tabs.get(info["targetId"])
        if tab is None:
            tab = Tab(info["targetId"])
            tab.opened_by = info.get("openerId")
            self.tabs[tab.id] = tab
        tab.url = str(info.get("url", tab.url) or "")
        tab.title = str(info.get("title", tab.title) or "")

    def _event(self, msg):
        method = msg.get("method", "")
        p = msg.get("params", {})
        if method in ("Target.targetCreated", "Target.targetInfoChanged"):
            self._saw_target(p.get("targetInfo", {}))
            return
        if method == "Target.targetDestroyed":
            tab = self.tabs.pop(p.get("targetId"), None)
            if tab and tab.session:
                self.by_session.pop(tab.session, None)
            if self.active is tab:
                self.active = None
            return
        if method == "Target.detachedFromTarget":
            tab = self.by_session.pop(p.get("sessionId"), None)
            if tab:
                tab.session = tab.world = None
            return
        tab = self.by_session.get(msg.get("sessionId"))
        if tab is None:
            return
        if method == "Runtime.executionContextCreated":
            ctx = p.get("context", {})
            aux = ctx.get("auxData", {})
            if ctx.get("name") == WORLD and aux.get("frameId") == tab.main_frame:
                tab.world = ctx.get("id")
        elif method in ("Runtime.executionContextsCleared",):
            tab.world = None
        elif method == "Runtime.executionContextDestroyed":
            if p.get("executionContextId") == tab.world:
                tab.world = None
        elif method == "Page.frameNavigated":
            frame = p.get("frame", {})
            if not frame.get("parentId"):
                tab.main_frame = frame.get("id")
                tab.url = frame.get("url", tab.url)
                tab.world = None
                tab.navigations += 1
        elif method == "Page.frameStartedLoading":
            if p.get("frameId") == tab.main_frame:
                tab.loaded = False
        elif method in ("Page.loadEventFired", "Page.frameStoppedLoading"):
            if method == "Page.loadEventFired" or p.get("frameId") == tab.main_frame:
                tab.loaded = True
        elif method == "Page.javascriptDialogOpening":
            tab.dialog = {"type": p.get("type"), "message": p.get("message", "")[:500],
                          "default": p.get("defaultPrompt", "")}
        elif method == "Page.javascriptDialogClosed":
            tab.dialog = None
        elif method == "Network.requestWillBeSent":
            if p.get("type") not in ("WebSocket", "EventSource"):
                tab.inflight[p.get("requestId")] = time.monotonic()
            tab.last_network = time.monotonic()
        elif method in ("Network.loadingFinished", "Network.loadingFailed"):
            tab.inflight.pop(p.get("requestId"), None)
            tab.last_network = time.monotonic()

    # ── tabs ──

    def pages(self):
        """The open tabs, oldest first, without the browser's own internal pages."""
        return [t for t in self.tabs.values() if not t.url.startswith("devtools://")]

    def tab(self, tab_id=None):
        """The tab an action goes to: the one named, else the active one, else the first."""
        self.connect()
        pages = self.pages()
        if tab_id:
            for t in pages:
                if t.id == tab_id or t.id.startswith(tab_id):
                    return self.attach(t)
            raise CdpError("no open tab `%s`; tabs lists them" % tab_id)
        if self.active is None or self.active.id not in self.tabs:
            if not pages:
                raise CdpError("the browser has no tab open; go opens one")
            self.active = pages[0]
        return self.attach(self.active)

    def attach(self, tab):
        if tab.session:
            return tab
        got = self.call("Target.attachToTarget", {"targetId": tab.id, "flatten": True})
        tab.session = got["sessionId"]
        self.by_session[tab.session] = tab
        for method in ("Page.enable", "Runtime.enable", "Network.enable"):
            self.call(method, session=tab.session)
        tree = self.call("Page.getFrameTree", session=tab.session)
        tab.main_frame = tree["frameTree"]["frame"]["id"]
        return tab

    def new_tab(self, url):
        got = self.call("Target.createTarget", {"url": url or "about:blank"})
        deadline = time.monotonic() + 5
        with self.cond:
            while got["targetId"] not in self.tabs and time.monotonic() < deadline:
                self.cond.wait(0.2)
        tab = self.tabs.get(got["targetId"]) or Tab(got["targetId"], url)
        self.tabs.setdefault(tab.id, tab)
        self.active = self.attach(tab)
        return tab

    # ── the reader ──

    def world(self, tab):
        """The isolated world's context in the tab's top frame, made (and the reader installed)
        when the page has none yet."""
        if tab.world is None:
            got = self.call("Page.createIsolatedWorld",
                            {"frameId": tab.main_frame, "worldName": WORLD, "grantUniveralAccess": True},
                            session=tab.session)
            tab.world = got["executionContextId"]
            self._eval(tab, page_js())
        return tab.world

    def _eval(self, tab, expression, timeout=CALL_TIMEOUT):
        got = self.call("Runtime.evaluate", {
            "expression": expression, "contextId": tab.world, "returnByValue": True,
            "awaitPromise": True}, session=tab.session, timeout=timeout)
        if got.get("exceptionDetails"):
            d = got["exceptionDetails"]
            text = (d.get("exception") or {}).get("description") or d.get("text") or "an error"
            raise CdpError("the page reader failed: %s" % text.splitlines()[0][:200])
        return (got.get("result") or {}).get("value")

    def run(self, tab, fn, *args, timeout=CALL_TIMEOUT):
        """Call `__yb.<fn>(*args)` in the tab's isolated world, making it on a page that has none."""
        expression = "globalThis.__yb.%s(%s)" % (fn, ", ".join(json.dumps(a) for a in args))
        guarded = ("(() => { if (!globalThis.__yb || globalThis.__yb.__yantrik !== true) "
                   "return {__missing: true}; return %s; })()" % expression)
        for attempt in (1, 2, 3):
            self.world(tab)
            try:
                value = self._eval(tab, guarded, timeout=timeout)
            except CdpError as e:
                # A navigation between making the world and using it destroys the context.
                if attempt == 3 or "context" not in str(e).lower():
                    raise
                tab.world = None
                continue
            if isinstance(value, dict) and value.get("__missing"):
                self._eval(tab, page_js())
                continue
            return value
        raise CdpError("the page reader could not be installed in this page")
