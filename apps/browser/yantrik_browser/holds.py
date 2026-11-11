"""A key a Mind holds down is never left down (#1170; a part of inputs.py, moved out of it).

`key_down` puts a key down and leaves it down until `key_up`. A page's own key logic — a game, a
shortcut handler that tracks a modifier — sees only that the key is down; it cannot tell that the
Mind that pressed it has gone. So a key is released on every way its hold can end: `key_up`; the
end of a timed hold, whether it ran out or failed; a navigation, a tab change or a tab closed
(driver.py asks `release_all` first); and, for a hold nobody ends, after HOLD_MAX seconds, by a
timer that needs no caller. Released means a real `keyUp` is sent, then the tab forgets the key.
"""

import threading

from . import hands

HOLD_MAX = 60.0       # seconds a key stays down with nobody to release it


def _name(name):
    return str(name).strip().lower()


def expire_later(browser, tab, name):
    """Release `name` after HOLD_MAX seconds, if it is still down by then."""
    key = _name(name)
    timer = threading.Timer(HOLD_MAX, _expire, (browser, tab, name))
    timer.daemon = True
    old = tab.hold_timers.pop(key, None)
    if old:
        old.cancel()
    tab.hold_timers[key] = timer
    timer.start()


def _expire(browser, tab, name):
    if _name(name) in tab.held_keys:
        release_all(browser, tab, [name])


def release(browser, tab, name):
    """Send the `keyUp` and forget the key — forgotten even when the browser did not answer."""
    key = _name(name)
    timer = tab.hold_timers.pop(key, None)
    if timer:
        timer.cancel()
    try:
        hands.key_up(browser, tab, name)
    finally:
        tab.held_keys.discard(key)


def release_all(browser, tab, names=None):
    """Release the keys named, or every key this tab holds: best effort, since the page or the
    browser may already be gone, and the caller is on its way to leaving."""
    for name in sorted(names or tab.held_keys):
        try:
            release(browser, tab, name)
        except Exception:  # noqa: BLE001 - nobody is left to tell
            pass
