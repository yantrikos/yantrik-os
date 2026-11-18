"""The page's console (#1152), read back: what it wrote and the exceptions nobody caught, as
`read` and every act's answer carry them and as `console` lists them (a part of driver.py, moved
out of it).

A Mind that runs its own build must see "Uncaught SyntaxError at line 4091" without DevTools. The
entries themselves are kept per tab by cdp.py from `Runtime.exceptionThrown`, `consoleAPICalled`
and `Log.entryAdded`, each already folded to one line. Every line answered here is marked as the
page's own words, or the browser's about the page, and never the OS's.
"""

import time

from .cdp import short

CONSOLE_IN_READ = 8       # recent errors and warnings a reading or an act's answer carries
CONSOLE_LIMIT = 30        # lines `console` lists at once
ERRORS = ("error", "warning")


def _favicon_noise(entry):
    """The one error a clean page produces: the browser asking for /favicon.ico and not finding
    it. Left out of the errors a reading carries (it says nothing about the page's code); kept
    where `everything` is asked for."""
    return entry.get("kind") == "browser.network" and str(entry.get("at", "")).endswith("/favicon.ico")


class ConsoleMixin:
    """The console methods of `Driver` (a part of driver.py, moved out of it). It relies on the
    driver's `held`, `_tab` and the tab's `console` ring."""

    @staticmethod
    def _console_lines(tab, since=0, this_page=True, levels=ERRORS, limit=CONSOLE_IN_READ):
        """The console lines kept for `tab` after entry `since`: of this page only (since its
        last navigation) unless told otherwise, at the `levels` given (None for all). Every line
        is marked `from: page`: these are the page's words, or the browser's about the page, and
        never the OS's. Returns (the last `limit` of them, how many there were)."""
        now = time.monotonic()
        picked = [e for e in tab.console
                  if e["seq"] > since and (not this_page or e["nav"] == tab.navigations)
                  and (levels is None or (e["level"] in levels and not _favicon_noise(e)))]
        lines = [{"level": e["level"], "kind": e["kind"], "text": e["text"], "at": e["at"],
                  "from": "page", "ago": round(now - e["when"], 1)} for e in picked[-limit:]]
        return lines, len(picked)

    def _with_console(self, out, tab, since=0):
        """Put the page's recent errors and warnings (after `since`) on an answer, when there are any."""
        lines, total = self._console_lines(tab, since)
        if lines:
            out["console"] = lines
            if total > len(lines):
                out["console_more"] = total - len(lines)
        return out

    def console(self, tab=None, everything=False, limit=CONSOLE_LIMIT):
        """What the page wrote to its console and the exceptions nobody caught: its errors and
        warnings since it was last loaded, or with `everything` its logs too and the lines kept
        from the pages before it in this tab."""
        limit = max(1, min(int(limit or CONSOLE_LIMIT), 200))
        with self.held():
            t = self._tab(tab)
            lines, total = self._console_lines(t, 0, this_page=not everything,
                                               levels=None if everything else ERRORS, limit=limit)
            return {"console": lines, "total": total, "url": t.url, "title": t.title, "tab": short(t.id),
                    "showing": ("every line kept for this tab, logs included"
                                if everything else "errors and warnings since this page was loaded")}
