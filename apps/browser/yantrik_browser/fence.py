"""What a `file:` page may draw: every `file:` request of a tab is held until it is vetted.

Chromium lets one local file embed another as an image, a frame, an object, a stylesheet or a
font, and the renderer draws it; a screenshot of the tab then carries what the page was never
allowed to name (a key under `.ssh`, a file outside the home). Script cannot read such a file,
but the picture can. A flag cannot close this, since the browser is already running when the
surface meets it, and a loopback server would be one more thing listening. So the tab's own
DevTools session pauses each `file:` request (`Fetch.requestPaused`) and this decides it: the
gate (driver.py, from local_files.py) answers with the URL of the real path it vetted, which is
what loads — a link swapped after the vetting is not followed — or with nothing, and the request
fails as access denied. The page itself is held to the same gate as everything it embeds. Nothing
is decided on the reader's thread, which must stay free to read the answer to the decision.
"""

import queue
import threading

PATTERNS = [{"urlPattern": "file://*"}]


class Fence:
    """The decider of one browser's paused `file:` requests. `call` is the browser's `call`;
    `agent_of(session)` says whether the tab on a session is held to an agent's rule; `gate(url,
    agent)` is set by the driver and is `None` until it is, which refuses everything: a fence
    with no rule is shut. Who is asking is the requesting tab's, never the last caller's."""

    def __init__(self, call, agent_of):
        self.call = call
        self.agent_of = agent_of
        self.gate = None
        self.queue = queue.Queue()
        self.worker = None
        self.lock = threading.Lock()

    def _put(self, job):
        with self.lock:
            if self.worker is None:
                self.worker = threading.Thread(target=self._run, name="file-fence", daemon=True)
                self.worker.start()
        self.queue.put(job)

    def paused(self, session, params):
        """A request the browser holds, from the reader's thread: queued, never decided here."""
        self._put(lambda: self._decide(session, params))

    def arm(self, session, target, waiting, done):
        """A target the browser attached us to, from the reader's thread: a page (`target`) gets
        the fence, and only then, if the browser holds it at its start (`waiting`), is let run, so
        its first request is already decided here. `done` is called once the fence is up. A page
        that cannot be fenced is closed rather than let run."""
        self._put(lambda: self._arm(session, target, waiting, done))

    def _run(self):
        while True:
            try:
                self.queue.get()()
            except Exception:  # noqa: BLE001 - the browser went away: nobody is left to answer
                pass

    def _arm(self, session, target, waiting, done):
        if target:
            try:
                self.call("Fetch.enable", {"patterns": PATTERNS}, session=session)
            except Exception:  # noqa: BLE001 - never run a page the fence is not on
                self.call("Target.closeTarget", {"targetId": target})
                return
            finally:
                if done:
                    done()
        if waiting:
            self.call("Runtime.runIfWaitingForDebugger", session=session)

    def _decide(self, session, params):
        try:
            load = (self.gate((params.get("request") or {}).get("url", ""), self.agent_of(session))
                    if self.gate else None)
        except Exception:  # noqa: BLE001 - a gate that fails is a gate that refuses
            load = None
        if load:
            self.call("Fetch.continueRequest", {"requestId": params["requestId"], "url": load}, session=session)
        else:
            self.call("Fetch.failRequest", {"requestId": params["requestId"], "errorReason": "AccessDenied"},
                      session=session)
