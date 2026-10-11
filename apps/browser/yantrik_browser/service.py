"""The browser as an app of this desktop: `app-browser`, served on the person's side of the door.

Until this, a mind drove the browser over its DevTools port directly, and every rule about what
it may press lived in the mind's own process (the MCP bridge) — which any code running as the
mind could step around, and which the port itself could not tell apart from anyone else on the
machine (#477). Now the port answers the person and nobody else, and minds reach the browser
the way they reach every other app: `describe browser`, `act browser click ref=e12`, through
the mind door, under the same grades, mode and cards.

Grades:
  safe       read, find, text, tabs, media, wait, scroll — looking
  standard   go, back, forward, reload, click, type, press, select, new_tab, switch_tab,
             close_tab, dialog                       — using a page as a person does
  sensitive  commit                                  — the one press that cannot be taken back:
             a control whose label reads as buying, sending or deleting. Its description says it
             cannot be undone, which the gate reads on every door: it asks in every mode but
             full bypass (`bypass_all`) — plain bypass included — and no "allow for this
             session" covers the next one.
"""

import typing

from yantrik_surface import Later, Refusal, Surface

from . import driver as drv

Ref = typing.Annotated[str, "an element's ref from read or find, like e12"]
Tab = typing.Annotated[str, "optional: a tab's id from tabs; the active tab when left out"]


def build(driver=None, **surface_options):
    d = driver or drv.Driver()

    def summary():
        s = d.status()
        if not s.get("open"):
            return "Browser — not open"
        tabs = s.get("tabs") or []
        active = next((t for t in tabs if t["active"]), tabs[0] if tabs else None)
        if not active:
            return "Browser — open, no tab"
        more = " · %d tabs" % len(tabs) if len(tabs) > 1 else ""
        dialog = " · a dialog is waiting" if s.get("dialog") else ""
        return "Browser — \"%s\" (%s)%s%s" % (active["title"][:70] or active["url"][:70],
                                             drv.host_of(active["url"]) or active["url"][:40], more, dialog)

    s = Surface("browser", summary=summary, aliases=("chromium",), **surface_options)

    @s.view
    def state():
        return d.status()

    def now(fn):
        try:
            return fn()
        except drv.Refused as e:
            raise Refusal(str(e)) from None

    def later(fn):
        return Later(lambda: now(fn))

    # ── looking ──

    @s.action(grade="safe")
    def read(tab: Tab = "", all: bool = False) -> dict:
        """What is on the page: everything that can be clicked, typed into or chosen, in view (or
        all of it), each with a ref, its role, name, state and box, among the headings, alerts and
        dialogs that say where it is. Refs stay the same while the element exists"""
        return now(lambda: d.read(tab or None, all))

    @s.action(grade="safe")
    def find(query: str, tab: Tab = "") -> dict:
        """Where something is, anywhere on the page, not only in view: the elements whose name,
        value or link holds these words"""
        return now(lambda: d.find(query, tab or None))

    @s.action(grade="safe")
    def text(tab: Tab = "", limit: int = 20000) -> dict:
        """What the page says, as a reader sees it: its text, not its controls"""
        return now(lambda: d.text(tab or None, limit))

    @s.action(grade="safe")
    def tabs() -> dict:
        """The tabs that are open, and which one actions go to"""
        return now(d.tabs)

    @s.action(grade="safe")
    def media(tab: Tab = "") -> dict:
        """The page's videos and sounds: whether each is playing, where it is, and whether the
        player says an advert is showing"""
        return now(lambda: d.media(tab or None))

    @s.action(grade="safe", expected_seconds=5)
    def wait(text: str = "", seconds: float = 5.0, tab: Tab = "") -> dict:
        """Wait for words to appear on the page, or for it to stop loading, up to `seconds`
        (at most 30)"""
        return later(lambda: d.wait(text or None, seconds, tab or None))

    @s.action(grade="safe")
    def scroll(direction: typing.Literal["down", "up", "top", "bottom"] = "down", ref: str = "",
               tab: Tab = "") -> dict:
        """Scroll the page (down, up, top, bottom) or bring one ref into view; answers with what
        is in view then"""
        return now(lambda: d.scroll(direction, ref or None, tab or None))

    # ── using the page ──

    @s.action(grade="standard", expected_seconds=4)
    def go(url: str, new_tab: bool = False, tab: Tab = "") -> dict:
        """Open a web page, and wait for it to load; answers with what is on it"""
        return later(lambda: d.go(url, tab or None, new_tab))

    @s.action(grade="standard", expected_seconds=3)
    def back(tab: Tab = "") -> dict:
        """The page before this one in the tab's history"""
        return later(lambda: d.history(-1, tab or None))

    @s.action(grade="standard", expected_seconds=3)
    def forward(tab: Tab = "") -> dict:
        """The page after this one in the tab's history"""
        return later(lambda: d.history(1, tab or None))

    @s.action(grade="standard", expected_seconds=3)
    def reload(tab: Tab = "") -> dict:
        """Load the page again"""
        return later(lambda: d.reload(tab or None))

    @s.action(grade="standard", expected_seconds=2)
    def click(ref: Ref, tab: Tab = "") -> dict:
        """Press an element as a person would, with the pointer, and answer with what changed. A
        control that reads as buying, sending or deleting is refused here: that is commit"""
        return later(lambda: d.click(ref, tab or None))

    @s.action(grade="standard", expected_seconds=2)
    def type(ref: Ref, text: str, clear: bool = True, enter: bool = False, tab: Tab = "") -> dict:
        """Type into a field (replacing what is in it unless clear is false), and press Enter after
        if asked — unless Enter would submit a form whose button reads as a commitment"""
        return later(lambda: d.type(ref, text, clear, enter, tab or None))

    @s.action(grade="standard", expected_seconds=2)
    def press(key: typing.Annotated[str, "Enter, Tab, Escape, ArrowDown, PageDown, ctrl+a …"],
              ref: str = "", tab: Tab = "") -> dict:
        """Press a key, in the element given or wherever the focus is"""
        return later(lambda: d.press(key, ref or None, tab or None))

    @s.action(grade="standard", expected_seconds=2)
    def select(ref: Ref, option: typing.Annotated[str, "the option's text as read lists it"],
               tab: Tab = "") -> dict:
        """Choose an option in a list (a <select>)"""
        return later(lambda: d.select(ref, option, tab or None))

    @s.action(grade="standard", expected_seconds=2)
    def dialog(accept: bool = True, text: str = "", tab: Tab = "") -> dict:
        """Answer the page's alert, confirm or prompt dialog. A confirm that reads as a commitment
        is accepted with commit ref=dialog instead"""
        return later(lambda: d.dialog(accept, text or None, tab or None))

    @s.action(grade="standard")
    def switch_tab(tab: typing.Annotated[str, "the tab's id from tabs"]) -> dict:
        """Bring a tab to the front and send actions to it"""
        return now(lambda: d.switch(tab))

    @s.action(grade="standard")
    def close_tab(tab: typing.Annotated[str, "the tab's id from tabs"]) -> dict:
        """Close a tab"""
        return now(lambda: d.close_tab(tab))

    # ── the press that cannot be taken back ──

    @s.action(grade="sensitive", expected_seconds=3, description=(
        "Press a control whose label reads as a commitment — buy, pay, send, post, delete, "
        "confirm — on the named site. What it does cannot be undone: an order placed or a message "
        "sent stays so. So it asks the person every time. label must be the control's label as "
        "read shows it and site the page's site, and nothing is pressed if either has changed"))
    def commit(ref: typing.Annotated[str, "the control's ref, or `dialog` for a confirm dialog"],
               label: typing.Annotated[str, "the control's label, exactly as read shows it"],
               site: typing.Annotated[str, "the page's site, e.g. shop.example.com"],
               tab: Tab = "") -> dict:
        return later(lambda: d.commit(ref, label, site, tab or None))

    return s, d
