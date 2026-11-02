"""The two browsers the service can drive, and which one a call gets (#1085).

One connection per side, held for as long as that side's browser runs. A call is sent to its
side's browser by who is calling (guard.py), and only after what holds that port has been
checked; the connection is bound to the listener that was checked, and dropped if the port ever
holds anything else.
"""

import threading

from . import cdp, guard, ports


class Sides:
    def __init__(self, fixed=None, check=None):
        """`fixed`: one browser the caller runs and owns (the tests'), used for everyone and
        never checked. Otherwise a browser per side on its port, checked by `check` (a Guard)."""
        self.fixed, self.guard = fixed, (None if fixed is not None else (check or guard.Guard()))
        self.browsers = ({ports.PERSON: fixed, ports.MIND: fixed} if fixed is not None else
                         {side: cdp.Browser(ports.address(ports.port_of(side)))
                          for side in (ports.PERSON, ports.MIND)})
        self.bound = {}                 # side -> the listener its connection was made to
        self.last = {ports.PERSON: {"open": False}, ports.MIND: {"open": False}}
        self._here = threading.local()

    @property
    def current(self):
        """The browser the call this thread is handling drives."""
        return self._here.browser

    def side(self):
        return self.guard.side() if self.guard is not None else ports.MIND

    def bind(self, start=False):
        """Choose this call's browser and check what holds its port. Raises guard.Denied when it
        may not be used. With `start`, a mind whose browser is not open gets Mind View's started
        (the first web call on a cold start carries on rather than failing). Returns the side."""
        if self.guard is None:
            self._here.browser = self.fixed
            return ports.MIND
        side = self.guard.side()
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
