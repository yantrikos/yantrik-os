"""Pressing, typing and waiting, the way a person's hands do it.

A click is the browser's own input: the pointer moves to the element, presses and releases. An
`el.click()` from script is a different event — no pointer, no hover, `isTrusted` false — and
enough sites ignore it that a click which "worked" often did nothing. Typing goes through
`Input.insertText`, which fires the input events a framework's controlled field listens for, so
the value it holds is the value typed.
"""

import time

# Keys a caller can name, as DevTools wants them: key, code, and the Windows key code the
# browser's own handlers still read.
KEYS = {
    "enter": ("Enter", "Enter", 13, "\r"),
    "tab": ("Tab", "Tab", 9, None),
    "escape": ("Escape", "Escape", 27, None),
    "backspace": ("Backspace", "Backspace", 8, None),
    "delete": ("Delete", "Delete", 46, None),
    "space": (" ", "Space", 32, " "),
    "arrowup": ("ArrowUp", "ArrowUp", 38, None),
    "arrowdown": ("ArrowDown", "ArrowDown", 40, None),
    "arrowleft": ("ArrowLeft", "ArrowLeft", 37, None),
    "arrowright": ("ArrowRight", "ArrowRight", 39, None),
    "pageup": ("PageUp", "PageUp", 33, None),
    "pagedown": ("PageDown", "PageDown", 34, None),
    "home": ("Home", "Home", 36, None),
    "end": ("End", "End", 35, None),
}
KEY_ALIASES = {"return": "enter", "esc": "escape", "up": "arrowup", "down": "arrowdown",
               "left": "arrowleft", "right": "arrowright", " ": "space"}
MODIFIERS = {"alt": 1, "ctrl": 2, "control": 2, "meta": 4, "shift": 8}


def key_named(name):
    """(key, code, keycode, text, modifiers) for `Enter`, `Tab`, `ctrl+a`, `shift+Tab`…, or None."""
    parts = [p for p in str(name or "").strip().lower().split("+") if p]
    if not parts:
        return None
    mods = 0
    for p in parts[:-1]:
        if p not in MODIFIERS:
            return None
        mods |= MODIFIERS[p]
    last = KEY_ALIASES.get(parts[-1], parts[-1])
    if last in KEYS:
        key, code, vk, text = KEYS[last]
    elif len(last) == 1 and last.isalnum():
        key, code, vk, text = last, ("Key" + last.upper()) if last.isalpha() else ("Digit" + last), ord(last.upper()), last
    else:
        return None
    if mods & (MODIFIERS["ctrl"] | MODIFIERS["meta"] | MODIFIERS["alt"]):
        text = None
    return key, code, vk, text, mods


def _dialog_opened(tab):
    """For input that may open a JavaScript dialog: an alert or a confirm holds the page, and the
    page holds the answer to the key or the click that opened it until someone answers the dialog."""
    return lambda: tab.dialog is not None


def click(browser, tab, x, y):
    s = tab.session
    browser.call("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": x, "y": y}, session=s)
    for kind in ("mousePressed", "mouseReleased"):
        browser.call("Input.dispatchMouseEvent", {"type": kind, "x": x, "y": y, "button": "left",
                                                  "buttons": 1 if kind == "mousePressed" else 0,
                                                  "clickCount": 1}, session=s,
                     until=_dialog_opened(tab))


def press(browser, tab, name):
    k = key_named(name)
    if k is None:
        raise ValueError(name)
    key, code, vk, text, mods = k
    s = tab.session
    down = {"type": "keyDown" if text else "rawKeyDown", "key": key, "code": code,
            "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk, "modifiers": mods}
    if text:
        down["text"] = text
        down["unmodifiedText"] = text
    browser.call("Input.dispatchKeyEvent", down, session=s, until=_dialog_opened(tab))
    browser.call("Input.dispatchKeyEvent", {"type": "keyUp", "key": key, "code": code,
                                            "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk,
                                            "modifiers": mods}, session=s, until=_dialog_opened(tab))


def insert_text(browser, tab, text):
    browser.call("Input.insertText", {"text": text}, session=tab.session)


def wheel(browser, tab, dy, x=None, y=None):
    browser.call("Input.dispatchMouseEvent", {"type": "mouseWheel", "x": x or 200, "y": y or 200,
                                              "deltaX": 0, "deltaY": dy}, session=tab.session)


# ── settling ──

QUIET_NETWORK = 0.5    # seconds with no request starting or finishing
LONG_POLL = 5.0        # a request open longer than this is a long poll, not the page loading


def settle(browser, tab, limit, since_navigations=None):
    """Wait until the page has stopped moving, or `limit` seconds: loaded, no request of its own
    in flight for QUIET_NETWORK, and no dialog waiting. Returns how long it took and why it
    stopped ("settled", "dialog", "limit")."""
    started = time.monotonic()
    deadline = started + limit
    with browser.cond:
        while True:
            now = time.monotonic()
            if tab.dialog:
                return round(now - started, 2), "dialog"
            live = [t for t in tab.inflight.values() if now - t < LONG_POLL]
            navigated = since_navigations is not None and tab.navigations > since_navigations
            loading = not tab.loaded
            quiet = not live and now - tab.last_network >= QUIET_NETWORK
            if not loading and quiet and (since_navigations is None or navigated or now - started > 0.6):
                return round(now - started, 2), "settled"
            if now >= deadline or not browser.alive:
                return round(now - started, 2), "limit"
            browser.cond.wait(min(0.1, max(0.01, deadline - now)))
