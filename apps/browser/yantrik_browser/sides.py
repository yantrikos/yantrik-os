"""The two browsers the service can drive, and which one a call gets (#1085).

One connection per side, held for as long as that side's browser runs. A call is sent to its
side's browser by who is calling (guard.py), and only after what holds it has been checked: the
port for Mind View's browser, the pipe for the person's (#1137). The connection is bound to what
was checked, and dropped if that ever changes.
"""

import threading

from . import cdp, guard, pipes, ports


class Sides:
    def __init__(self, fixed=None, check=None, holder=None):
        """`fixed`: one browser the caller runs and owns (the tests'), used for everyone and
        never checked. Otherwise Mind View's browser on its port and the person's over the pipe
        `holder` keeps (pipes.py), checked by `check` (a Guard). The person's side has no address,
        so there is nothing for it to fall back to."""
        self.pipes = holder or getattr(check, "pipes", None) or pipes.PipeHolder()   # a guard's own
        self.fixed = fixed
        self.guard = None if fixed is not None else (check or guard.Guard(pipes=self.pipes))
        self.browsers = ({ports.PERSON: fixed, ports.MIND: fixed} if fixed is not None else
                         {ports.PERSON: cdp.Browser(opener=self.pipes.open),
                          ports.MIND: cdp.Browser(ports.address(ports.MIND_VIEW_PORT))})
        self.bound = {}                 # side -> the listener its connection was made to
        self.last = {ports.PERSON: {"open": False}, ports.MIND: {"open": False}}
        self._here = threading.local()

    @property
    def current(self):
        """The browser the call this thread is handling drives."""
        return self._here.browser

    def side(self):
        return self.guard.side() if self.guard is not None else ports.MIND

    def decide(self):
        """Who this call is for, before it waits for anything: (side, gone)."""
        return self.guard.decide() if self.guard is not None else (ports.MIND, lambda: False)

    def bind(self, start=False, decision=None):
        """Choose this call's browser and check what holds its port. Raises guard.Denied when it
        may not be used. With `start`, a mind whose browser is not open gets Mind View's started
        (the first web call on a cold start carries on rather than failing). Returns the side."""
        if self.guard is None:
            self._here.browser = self.fixed
            return ports.MIND
        side, gone = decision or self.guard.decide()
        if gone():
            raise guard.Denied("Nothing was done: whoever asked has gone away while this waited, "
                               "and a call is never carried out for a caller that is not there.")
        browser = self._here.browser = self.browsers[side]
        try:
            held = self.guard.vet(side)
        except guard.Denied:
            browser.close()
            raise
        if held is None and start and side == ports.MIND:
            try:
                held = self.guard.start_mind_browser()
            except guard.Denied:
                browser.close()
                raise
        if held is None or (browser.alive and self.bound.get(side) != held.key):
            # Nothing there, or not what the connection was made to: that connection is over.
            browser.close()
        self.bound[side] = held and held.key
        browser.check = lambda: self._still(side, held)
        return side

    def _still(self, side, held):
        """The connection's own check, before and after it opens: the port holds what it held."""
        now = self.guard.vet(side)
        if (now and now.key) != (held and held.key):
            raise cdp.BrowserClosed()
