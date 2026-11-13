"""The hands the surface lacked (#1152): a drag, a held key, a right click — the moves a Mind
needs to test the window manager, the game and the menu it wrote (a part of driver.py, moved
out of it).

A drag begins with a press and ends with a release, so both are judged as `click` judges a press:
the element pressed by `_pressable` (visible, uncovered, enabled, a control, not a frame from
another site, not a commitment), and the point released at by the same words and decision model
and never inside such a frame: "drag to Trash" is refused and nothing moves. Held Enter and Space go through `_key_is_safe` exactly as `press`. The browser's
own context menu is never shown (nothing on this surface could read or close it); the page's
own, if it makes one, is what `read` lists.
"""

import time

from . import hands, holds
from .commit import commitment_of, fold_site
from .refusals import PageRefused, Refused

HOLD_LIMIT = 10.0         # seconds key_down holds a key for when asked to hold and release
DRAG_LIMIT = 4000         # pixels a drag may move in either direction
# Controls that Enter or Space presses when they have the focus.
PRESSED_BY_KEYS = {"button", "link", "clickable", "menuitem", "menuitemcheckbox", "menuitemradio",
                   "tab", "switch", "checkbox", "radio", "option", "treeitem"}


class InputsMixin:
    """The drag, held-key and right-click methods of `Driver`, and the rule Enter and Space are
    held to. They rely on the driver's `held`, `_tab`, `_target`, `_pressable`, `_why_commitment`,
    `_run` and `_act`."""

    def _key_is_safe(self, t, ref, key, typed=False):
        """Refuse Enter or Space where it could press a commitment: on a focused control that reads
        as one, or from a field whose form (or, with no form, whose surroundings — a chat box's
        Send) has one. Asked again just before the key is sent, after any typing, since a label
        can change as a field fills in."""
        if key not in ("Enter", " "):
            return
        found = self._run(t, "pressables", ref) or {}
        focused = found.get("focused") or {}
        # A field's own label says nothing about what Enter does ("Order note" is a note): only a
        # focused control that a key presses is judged by its own name.
        why = self._why_commitment(focused, t) if focused.get("role") in PRESSED_BY_KEYS else None
        if why:
            raise PageRefused("%s would press %s \"%s\" (%s), which %s%s. It is pressed with commit ref=%s "
                              "label=\"%s\" site=%s, which asks the person first."
                              % ("Enter" if key == "Enter" else "Space", focused.get("role"), focused.get("name"),
                                 focused.get("ref"), why, "; the text was typed and the key was not pressed"
                                 if typed else "; nothing was pressed", focused.get("ref"), focused.get("name"),
                                 fold_site(t.url)))
        if key != "Enter":
            return
        # The buttons Enter could press: the words for all of them, the model for the first few.
        for i, c in enumerate(found.get("around") or []):
            why = (self._why_commitment(c, t) if i < 4 else
                   ("reads as a commitment (\"%s\")" % commitment_of(c) if commitment_of(c) else None))
            if why:
                raise PageRefused("Enter here could press %s \"%s\" (%s), which %s%s. Press it with commit "
                                  "ref=%s label=\"%s\" site=%s, which asks the person first."
                                  % (c.get("role"), c.get("name"), c.get("ref"), why,
                                     "; the text was typed and Enter was not pressed" if typed
                                     else "; nothing was pressed", c.get("ref"), c.get("name"), fold_site(t.url)))

    def context_click(self, ref, tab=None):
        """A right click on an element, held to the rules of click: a control that reads as a
        commitment is refused, since many handlers act on mousedown whichever button it is. The
        browser's own menu is never shown (nothing on this surface could read or close it); the
        page's own, if it makes one, is what read lists."""
        with self.held():
            t = self._tab(tab)
            target = self._pressable(t, ref)       # mousedown fires for the right button too
            out = self._act(t, "right-clicked %s \"%s\" (%s)" % (target["role"], target["name"], ref),
                            lambda: hands.context_click(self.browser, t, target["x"], target["y"]))
            menu = out.pop("menu", None)
            out["menu"] = {"page": "the page showed a menu of its own: read lists what is in it",
                           "none": "the page has no menu of its own here (the browser's own menu is not "
                                   "shown to this surface)",
                           "unknown": "the page stopped the event before it could be told; if the "
                                      "browser's own menu opened, press Escape"}.get(menu, "no contextmenu "
                                                                                       "event reached the page")
            return out

    def drag(self, ref, dx, dy, tab=None):
        """Press on an element, move the pointer by (dx, dy) with the button held, release there:
        a window moved by its title bar, a slider, a resize handle."""
        try:
            dx, dy = int(dx), int(dy)
        except (TypeError, ValueError):
            raise Refused("drag needs dx and dy in pixels") from None
        if dx == 0 and dy == 0:
            raise Refused("drag needs a distance: dx and dy are both 0, and a press that does not move is click")
        if abs(dx) > DRAG_LIMIT or abs(dy) > DRAG_LIMIT:
            raise Refused("drag moves at most %d pixels either way" % DRAG_LIMIT)
        with self.held():
            t = self._tab(tab)
            target = self._pressable(t, ref)        # a press, judged as click judges it
            x, y = target["x"], target["y"]
            landing = self._run(t, "at", x + dx, y + dy)
            if landing and landing.get("frame_element"):
                raise PageRefused("dropping %s there would release the button inside a frame from another "
                                  "site, which cannot be read, so it is not released blind. Nothing was "
                                  "moved." % ref)
            why = self._why_commitment(landing, t) if landing else None
            if why:
                raise PageRefused("dropping %s there would land on %s \"%s\" (%s), which %s: what a drop "
                                  "onto it does may not be undone, and that is the person's to do. Nothing "
                                  "was moved." % (ref, landing["role"], landing["name"], landing["ref"], why))
            before = target.get("box")
            out = self._act(t, "dragged %s \"%s\" (%s) by (%d, %d)" % (target["role"], target["name"], ref, dx, dy),
                            lambda: hands.drag(self.browser, t, x, y, dx, dy))
            out["from"] = [x, y]
            out["to"] = [x + dx, y + dy]
            after = self._run(t, "target", ref)
            if after and not after.get("gone") and after.get("box") and before:
                out["box"] = after["box"]
                out["moved"] = [after["box"][0] - before[0], after["box"][1] - before[1]]
            return out

    def key_down(self, key, ref=None, seconds=0, tab=None):
        """Put a key down and leave it down — until key_up, or for `seconds` and then release
        it (a game's held arrow). Enter and Space are held to the same rule as press."""
        k = hands.key_named(key)
        if k is None:
            raise Refused("`%s` is not a key this knows. Keys: %s, a letter or digit, with ctrl+, "
                          "shift+, alt+ or meta+ in front" % (key, ", ".join(sorted(hands.KEYS))))
        try:
            seconds = max(0.0, min(float(seconds or 0), HOLD_LIMIT))
        except (TypeError, ValueError):
            raise Refused("seconds is how long to hold the key, at most %d" % HOLD_LIMIT) from None
        with self.held():
            t = self._tab(tab)
            if ref:
                focused = self._run(t, "focus", ref, False)
                if not focused or focused.get("gone"):
                    raise PageRefused("%s is no longer on the page; read it again" % ref)
            self._key_is_safe(t, ref, k[0])

            def work():
                self._key_is_safe(t, ref, k[0])
                hands.key_down(self.browser, t, key)
                if not seconds:
                    holds.expire_later(self.browser, t, key)
                    return
                try:
                    time.sleep(seconds)
                finally:
                    holds.release(self.browser, t, key)
            doing = ("held %s for %.1f s%s" % (key, seconds, (" in %s" % ref) if ref else "") if seconds
                     else "pressed %s down%s; it stays down until key_up, or for at most %d s" % (
                         key, (" in %s" % ref) if ref else "", holds.HOLD_MAX))
            out = self._act(t, doing, work, limit=hands.NAVIGATE_LIMIT if k[0] == "Enter" else hands.ACT_LIMIT)
            out["held"] = sorted(t.held_keys)
            return out

    def key_up(self, key=None, tab=None):
        """Release a key key_down put down — the one named, or every one still down."""
        with self.held():
            t = self._tab(tab)
            keys = [key] if key else sorted(t.held_keys)
            if not keys:
                raise Refused("no key is down in this tab")
            for name in keys:
                if hands.key_named(name) is None:
                    raise Refused("`%s` is not a key this knows" % name)

            def work():
                for name in keys:
                    holds.release(self.browser, t, name)
            out = self._act(t, "released %s" % ", ".join(keys), work)
            out["held"] = sorted(t.held_keys)
            return out
