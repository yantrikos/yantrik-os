"""One DevTools connection to the desktop's browser, held for as long as the browser runs.

`yos web` opened a websocket per command and evaluated a script in the page's own world: a new
connection per read, and a registry of elements any page script could see and rewrite. This is
the other way round. One browser-level connection, sessions attached per tab in flat mode, and
the reader (page.js) evaluated in an isolated world — the page's DOM, none of its JavaScript.

Thread-safe: calls come from the surface's connection threads; one reader thread answers them
and keeps the event state (tabs, dialogs, requests in flight) the waits read.
"""

import collections
import json
import os
import random
import threading
import time
import urllib.error
import urllib.request

from . import adopt, fence, ports, transport
from .pagelog import console_entry, exception_entry, log_entry

WORLD = "yantrik-browser"
CALL_TIMEOUT = 20.0
CONSOLE_KEEP = 80          # console lines kept per tab: errors, warnings, logs and uncaught exceptions

# The reader, in the order its parts build on each other; evaluated as one expression.
SCRIPTS = ("page.js", "watch.js")
_PAGE_JS = None


def page_js():
    global _PAGE_JS
    if _PAGE_JS is None:
        parts = []
        for name in SCRIPTS:
            with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), name), encoding="utf-8") as f:
                parts.append(f.read())
        _PAGE_JS = "\n;\n".join(parts)
    return _PAGE_JS


class BrowserClosed(Exception):
    """No browser this desktop can drive is answering."""


class CdpError(Exception):
    """The browser refused a command."""


def _holds_file_frame(tree):
    """Whether a frame tree has a `file:` frame anywhere in it."""
    return (str((tree.get("frame") or {}).get("url", "")).lower().startswith("file:")
            or any(_holds_file_frame(child) for child in tree.get("childFrames") or []))


def _may_have_drawn_file(tab, tree):
    """Whether a page the browser did not hold at its start may have drawn something from a `file:`
    URL: a `file:` frame, or a document that is no site's own (a window with an opener, an about:,
    blob: or data: page), whose image, style or object is no frame."""
    url = str((tree.get("frame") or {}).get("url", "")).lower()
    return bool(tab.opened_by) or url.startswith(("about:", "blob:", "data:")) or _holds_file_frame(tree)


def short(target_id):
    """A target id as the surface names a tab: its first eight characters."""
    return target_id[:8]


class Tab:
    """What the connection knows about one page target."""

    def __init__(self, target_id, url="", title=""):
        self.id = target_id
        self.url = url
        self.title = title
        self.session = None
        self.ready = False         # attached, fenced and enabled: what attach() makes
        self.attachments = 0       # how many times attach() has made it ready
        self.attaching = threading.RLock()   # one attach at a time per tab
        self.preattached = False   # the browser said another session was on it before this service saw it
        self.gap = False           # another session was on it and no navigation has committed behind
                                   # our fence yet: it may hold a request nobody decides (adopt.py)
        self.watch = False         # its gap closed with no reload: a `file:` commit we did not start re-opens it
        self.fence_nav = 0         # its navigation count when our fence went up
        self.fenced = threading.Event()   # set once Fetch.enable is on for the session (fence.py)
        self.born = False          # attached before its first request (the browser held it for us)
        self.agent = None          # whose rule its `file:` requests are held to: True an agent's, False the
                                   # person's, None not known (held to the agent's). Sticky towards True
                                   # and inherited by a window it opens (#1170 re-review, B)
        self.world = None          # executionContextId of the isolated world in the top frame
        self.main_frame = None
        self.dialog = None         # {type, message, default} while a JS dialog is open
        self.inflight = {}         # requestId -> started (monotonic)
        self.last_network = time.monotonic()
        self.loaded = True
        self.navigations = 0
        self.opened_by = None      # the tab this one was opened from, if any
        # What the page wrote to its console and the exceptions nobody caught (#1152): the
        # page's own words, kept so a reader can see "Uncaught SyntaxError at line 4091"
        # without DevTools. Each entry is numbered (`console_seq`) so an act can say which
        # lines appeared while it ran, and carries the navigation it belongs to.
        self.console = collections.deque(maxlen=CONSOLE_KEEP)
        self.console_seq = 0
        self.held_keys = set()     # keys key_down put down and key_up has not released (hands.py)
        self.hold_timers = {}      # key -> the timer that releases it if nobody does (holds.py)
        self.cache_hits = 0        # responses the browser took from a cache rather than the network
        self.cached_requests = set()   # their request ids, so one is not counted twice
        self.document_cached = False   # the last main document came from a cache


class Browser:
    def __init__(self, address=None, opener=None, held_at_start=True):
        """A TCP DevTools port (Mind View's unless told otherwise; where a browser is, is decided
        in ports.py and nowhere a process's environment reaches), or, with `opener`, a pipe: a
        function that gives a transport.PipeTransport or raises BrowserClosed. A browser given an
        opener has no address and never touches TCP: the person's Browser (#1137)."""
        self.opener = opener
        self.address = None if opener else (address or ports.address(ports.MIND_VIEW_PORT)).rstrip("/")
        # Called before a connection is opened and again once it is, to hold it to the browser it
        # was made for (driver.py); it raises to stop. None where the caller owns the browser.
        self.check = None
        # Mind View's browser holds every page target at its start and fences it (fence.py). The
        # person's does not: it fences only the tabs the surface drives, with the person's rule, and
        # never closes or holds a window of theirs.
        self.held_at_start = held_at_start
        self.ws = None
        self.lock = threading.Lock()          # one writer
        self.cond = threading.Condition()     # replies and events
        self.replies = {}
        self.abandoned = set()                 # ids whose answer nobody waits for any more
        self.generation = 0                    # which connection is the current one
        # High and random: a reply the browser wrote to a process before this one (a service
        # restart on the held pipe) must not carry an id this process will send (aff60d15, L-a).
        self.ids = random.randrange(1 << 20, 1 << 30)
        self.tabs = {}                         # target id -> Tab, in the order they appeared
        self.by_session = {}
        self.active = None                     # the tab actions go to unless told otherwise
        self.stale = set()                     # sessions of a dead service seen on events (adopt.py)
        self.adopted = False                   # the person's tabs have been adopted on this connection
        self.fence = fence.Fence(self.call, self._agent_of, self._idle)   # decides every `file:` request a tab makes (fence.py)
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
        if self.check:
            self.check()
        self.close()
        ws = self._open()
        if self.check:
            try:
                self.check()
            except BaseException:
                ws.close()
                raise
        with self.cond:
            self.generation += 1
            generation = self.generation
            self.ws = ws
            self.alive = True
            self.replies.clear()
            self.abandoned.clear()
        self.tabs.clear()
        self.by_session.clear()
        self.stale.clear()
        self.adopted = False
        self.active = None
        self.reader = threading.Thread(target=self._read, args=(ws, generation), name="cdp-reader",
                                       daemon=True)
        self.reader.start()
        self.call("Target.setDiscoverTargets", {"discover": True})
        # Every page target, ours or one a page opens, is held at its start until its fence is up
        # (fence.py): no request of it, and no frame it writes, is made before the fence decides it.
        if self.held_at_start:
            self.call("Target.setAutoAttach", {"autoAttach": True, "waitForDebuggerOnStart": True,
                                               "flatten": True, "filter": [{"type": "page"}, {"exclude": True}]})
        for info in self.call("Target.getTargets").get("targetInfos", []):
            self._saw_target(info)
        if not self.held_at_start:
            adopt.adopt_pages(self)

    def _open(self):
        """The transport to the browser: its pipe, or a WebSocket to its port."""
        if self.opener is not None:
            return self.opener()
        try:
            from websocket import create_connection
        except ImportError:
            raise CdpError("python3-websocket is not installed") from None
        url = self._version().get("webSocketDebuggerUrl")
        if not url:
            raise BrowserClosed()
        # The origin the desktop's launcher allows (wire/dock.rs, --remote-allow-origins): its own
        # debugging address and nothing wider.
        return transport.WebSocketTransport(
            create_connection(url, origin=self.address, timeout=None, suppress_origin=False))

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
            return None
        tab = self.tabs.get(info["targetId"])
        if tab is None:
            tab = Tab(info["targetId"])
            tab.opened_by = info.get("openerId")
            tab.preattached = tab.gap = bool(info.get("attached"))
            opener = self.tabs.get(tab.opened_by)
            tab.agent = opener.agent if opener else None      # a window is held to its opener's rule
            self.tabs[tab.id] = tab
        tab.url = str(info.get("url", tab.url) or "")
        tab.title = str(info.get("title", tab.title) or "")
        return tab

    def _attached(self, p):
        """The browser attached us to a target it holds at its start (`waitForDebuggerOnStart`): a
        page is fenced and then let run, anything else is just let run. Both off the reader's thread."""
        info, session, waiting = p.get("targetInfo") or {}, p.get("sessionId"), bool(p.get("waitingForDebugger"))
        tab = self._saw_target(info)
        if tab is None:
            if waiting:
                self.fence.arm(session, None, True, None)
            return
        if tab.session == session:
            return
        tab.session, tab.born, tab.ready = session, waiting, False
        tab.fenced.clear()
        self.by_session[session] = tab
        self.fence.arm(session, tab.id, waiting, tab.fenced.set)

    def _idle(self, session):
        """The fence decided the last request paused on `session`: a stale one can go (adopt.py)."""
        if session in self.stale:
            self.fence.arming.submit(adopt.sweep, self, True)

    def _agent_of(self, session):
        """Whether the tab on this session is held to an agent's rule. On Mind View's browser, yes
        unless known not to be. On the person's, never: no agent reaches it (guard.py)."""
        tab = self.by_session.get(session)
        return self.held_at_start and (tab is None or tab.agent is not False)

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
        if method == "Target.attachedToTarget":
            self._attached(p)
            return
        if method == "Target.detachedFromTarget":
            tab = self.by_session.pop(p.get("sessionId"), None)
            if tab:
                tab.session = tab.world = None
                tab.ready = False
            return
        if method == "Fetch.requestPaused":
            adopt.learn(self, msg.get("sessionId"))
            self.fence.paused(msg.get("sessionId"), p)      # decided off the reader's thread
            return
        tab = self.by_session.get(msg.get("sessionId"))
        if tab is None:
            adopt.learn(self, msg.get("sessionId"))
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
                if adopt.regap(self, tab):
                    pass                   # a `file:` commit nobody here started: reloaded behind the fence
                elif tab.gap and tab.fenced.is_set() and tab.navigations > tab.fence_nav:
                    tab.gap = False        # a document committed behind our fence: the gap's is gone
                    adopt.after_commit(self)
                tab.held_keys.clear()      # a new document starts with nothing held
                tab.cached_requests.clear()
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
        elif method == "Network.requestServedFromCache":
            # The browser's memory or disk cache answered instead of the network (the one event
            # Chromium sends for it; responseReceived may follow with fromDiskCache, or not).
            tab.cache_hits += 1
            tab.cached_requests.add(p.get("requestId"))
        elif method == "Network.responseReceived":
            r = p.get("response") or {}
            cached = bool(r.get("fromDiskCache") or r.get("fromPrefetchCache") or r.get("fromServiceWorker")
                          or p.get("requestId") in tab.cached_requests)
            if cached and p.get("requestId") not in tab.cached_requests:
                tab.cache_hits += 1
                tab.cached_requests.add(p.get("requestId"))
            if p.get("type") == "Document" and p.get("frameId") == tab.main_frame:
                tab.document_cached = cached
        elif method == "Runtime.exceptionThrown":
            self._console(tab, exception_entry(p.get("exceptionDetails") or {}))
        elif method == "Runtime.consoleAPICalled":
            entry = console_entry(p)
            if entry is not None:
                self._console(tab, entry)
        elif method == "Log.entryAdded":
            self._console(tab, log_entry(p.get("entry") or {}))

    def _console(self, tab, entry):
        tab.console_seq += 1
        entry["seq"] = tab.console_seq
        entry["nav"] = tab.navigations
        entry["when"] = time.monotonic()
        tab.console.append(entry)

    # ── tabs ──

    def pages(self):
        """The open tabs, oldest first, without the browser's own internal pages."""
        return [t for t in self.tabs.values() if not t.url.startswith("devtools://")]

    def tab(self, tab_id=None, agent=False):
        """The tab an action goes to: the one named, else the active one, else the first. `agent`:
        the caller is an agent, which holds the tab to the agent's rule for good (see Tab.agent)."""
        self.connect()
        pages = self.pages()
        if tab_id:
            for t in pages:
                if t.id == tab_id or t.id.startswith(tab_id):
                    return self.attach(self._owned(t, agent))
            raise CdpError("no open tab `%s`; tabs lists them" % tab_id)
        if self.active is None or self.active.id not in self.tabs:
            if not pages:
                raise CdpError("the browser has no tab open; go opens one")
            self.active = pages[0]
        return self.attach(self._owned(self.active, agent))

    @staticmethod
    def _owned(tab, agent):
        """The tab, held to the agent's rule from now on if an agent is the caller, and to the
        person's if nobody knew whose it was and the caller is not an agent."""
        if agent:
            tab.agent = True
        elif tab.agent is None:
            tab.agent = False
        return tab

    def attach(self, tab, wait=CALL_TIMEOUT):
        """The tab with a session, fenced and enabled. A page the browser held at its start was
        fenced before its first request. One it did not hold (open before this connection, or its
        session was lost) is fenced now, and if a `file:` frame is in it, that loaded unfenced: it
        is loaded again behind the fence, and waited for up to `wait`. One thread at a time per tab."""
        if tab.ready:
            return tab
        with tab.attaching:
            return self._attach(tab, wait)

    def _attach(self, tab, wait):
        if tab.ready:
            return tab
        if self.held_at_start:
            with self.cond:    # the session auto-attach is making for a tab only just seen
                self.cond.wait_for(lambda: tab.session or tab.attachments or not self.alive, 2.0)
        if not tab.session:
            got = self.call("Target.attachToTarget", {"targetId": tab.id, "flatten": True})
            tab.session, tab.born = got["sessionId"], False
            self.by_session[tab.session] = tab
            self.call("Fetch.enable", {"patterns": fence.PATTERNS}, session=tab.session)
            tab.fence_nav = tab.navigations
            tab.fenced.set()
        elif not tab.fenced.wait(CALL_TIMEOUT):
            raise CdpError("the browser did not let the fence onto this tab")
        adopt.stop_preattached(self, tab)       # every pass, until a stop succeeds (adopt.py)
        tab.attachments += 1
        for method in ("Page.enable", "Runtime.enable", "Network.enable", "Log.enable"):
            self.call(method, session=tab.session)
        tree = self.call("Page.getFrameTree", session=tab.session)["frameTree"]
        tab.main_frame = tree["frame"]["id"]
        if not tab.born and (_may_have_drawn_file(tab, tree) if self.held_at_start
                             else _holds_file_frame(tree)):
            self._reload(tab, wait)             # a gap clears on its commit (frameNavigated)
        elif not tab.born:
            adopt.close_gap_unreloaded(self, tab)
        tab.ready = True
        return tab

    def _reload(self, tab, wait=CALL_TIMEOUT):
        """Load the tab again behind the fence and wait for it to commit and finish, or for the
        tab to be gone."""
        before = tab.navigations
        self.call("Page.reload", session=tab.session)
        deadline = time.monotonic() + wait
        with self.cond:
            while (not (tab.navigations > before and tab.loaded) and self.alive
                   and tab.id in self.tabs and time.monotonic() < deadline):
                self.cond.wait(0.2)

    def new_tab(self, url, agent=False):
        # A local page is opened blank and navigated once the fence is up: made with its address,
        # it would load, and draw what it embeds, before anything could vet it. (createTarget with a
        # url is not held at its start: test_local_files pins that no `file:` url is ever sent to it.)
        local = str(url or "").lower().startswith("file:")
        got = self.call("Target.createTarget", {"url": "about:blank" if local else url or "about:blank"})
        deadline = time.monotonic() + 5
        with self.cond:
            while got["targetId"] not in self.tabs and time.monotonic() < deadline:
                self.cond.wait(0.2)
        tab = self.tabs.get(got["targetId"]) or Tab(got["targetId"], url)
        self.tabs.setdefault(tab.id, tab)
        self.active = self.attach(self._owned(tab, agent))
        if local:
            self.call("Page.navigate", {"url": url}, session=tab.session)
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
