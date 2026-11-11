"""What the browser surface does, one method per action, each answering with what happened.

Every act answers with its consequence, not just "done": whether the page navigated, what it is
now, what text appeared (the "Invalid password" a click produced), whether a dialog or a new tab
opened, and — when the page is a new one — what is on it. A caller that has to read the page
again after every step to learn whether the step worked spends a round trip per step and still
misses the error that flashed and went; this is the one reading that answers it.

Before pressing anything it checks what a person would see: that the element is still there,
that nothing covers it, that it is not disabled, and that its label does not read as a
commitment — which only `commit` presses, and `commit` asks the person.
"""

import os
import threading
import time
import urllib.parse

from . import cdp, commit, guard, hands, holds, judgement, ports, sides
from .cdp import short
from .commit import commitment_of, fold_site
from .console import ConsoleMixin
from .inputs import InputsMixin
from .local_files import LocalFilesMixin, file_rule_of
from .refusals import PageRefused, Refused
from .shot import ShotMixin

NAVIGATE_LIMIT, ACT_LIMIT = hands.NAVIGATE_LIMIT, hands.ACT_LIMIT
TEXT_LIMIT = 20000


BUSY_WAIT = 20.0      # seconds an act waits for the one before it to finish

class Driver(LocalFilesMixin, ConsoleMixin, ShotMixin, InputsMixin):
    """The browser surface's actions; `browser` is this call's: a mind's is Mind View's, the person's theirs."""

    def __init__(self, browser=None, judge=None, guard_=None, file_rule=None, home=None):
        self.sides = sides.Sides(browser, guard_)
        # The person's decision model, asked when the words say nothing (judgement.py). `False`
        # turns it off (tests of the words alone); None is the shell's `decide`.
        self.judge = None if judge is False else (judge or judgement.Judgement())
        self.file_rule = file_rule_of(file_rule)
        self.home = home or (lambda: os.path.expanduser("~"))
        self.lock = threading.RLock()
        # The person's Browser outlives this service: when its pipe is handed over, connect to it at
        # once, so every open tab is fenced and a `file:` page reloaded without waiting for a call.
        self.sides.pipes.on_give = lambda: threading.Thread(
            target=self._person_arrived, daemon=True, name="person-arrived").start()

    def _person_arrived(self):
        if not self.lock.acquire(timeout=BUSY_WAIT):
            return                      # a call is at work; it connects (and adopts) itself
        try:
            browser = self.sides.ready(ports.PERSON)
            if browser is not None:
                browser.fence.gate = self._asset_gate()
                browser.connect()
        except (cdp.BrowserClosed, cdp.CdpError, guard.Denied):
            pass
        finally:
            self.lock.release()

    @property
    def browser(self):
        return self.sides.current

    def held(self, wait=BUSY_WAIT):
        """The driver, for one action: waited for up to `wait` seconds, then refused as busy."""
        driver = self

        class Held:
            def __enter__(self):
                decision = driver.sides.decide()        # who it is for, before any waiting
                if not driver.lock.acquire(timeout=wait):
                    raise Refused("the browser is busy with another action (a page may be slow to "
                                  "answer); try again in a few seconds")
                try:
                    driver.sides.bind(start=True, decision=decision)
                except BaseException as e:
                    driver.lock.release()
                    raise (PageRefused(str(e)) if isinstance(e, guard.Denied) else e) from None
                return driver

            def __exit__(self, *exc):
                driver.lock.release()
                return False
        return Held()

    # ── plumbing ──

    def _tab(self, tab=None, leaving=False):
        try:
            self.browser.fence.gate = self._asset_gate()     # what a local page may draw
            t = self.browser.tab(tab, agent=self._agent_calling())
        except cdp.BrowserClosed:
            self.browser.close()
            raise Refused("the browser is not open, so there is no page to read or act on. Open "
                          "it: act shell open_app name=browser — then read.") from None
        except cdp.CdpError as e:
            raise Refused(str(e)) from None
        except guard.Denied as e:
            raise PageRefused(str(e)) from None
        if not leaving:
            self._still_allowed(t)
        return t

    def _run(self, tab, fn, *args):
        try:
            return self.browser.run(tab, fn, *args)
        except cdp.BrowserClosed:
            raise Refused("the browser closed while this was being done") from None
        except cdp.CdpError as e:
            raise Refused(str(e)) from None

    def _call(self, method, params=None, tab=None):
        try:
            return self.browser.call(method, params, session=tab.session if tab else None)
        except cdp.BrowserClosed:
            raise Refused("the browser closed while this was being done") from None
        except cdp.CdpError as e:
            raise Refused(str(e)) from None

    def _why_commitment(self, control, t):
        """Why pressing `control` is a commitment, in a phrase, or None: the word list first, then
        the decision model. A judge can only add a commitment, never remove one the words found."""
        word = commitment_of(control)
        if word:
            return "reads as a commitment (\"%s\")" % word
        if self.judge is not None:
            judged = self.judge.commitment(control, t.url, t.title)
            if judged:
                return "was %s" % judged
        return None

    def _target(self, tab, ref):
        t = self._run(tab, "target", ref)
        if not t or t.get("gone"):
            raise PageRefused("%s is no longer on the page — it changed since it was read. Read it "
                              "again and use the new ref." % ref)
        return t

    # ── what an act answers with ──

    def _act(self, tab, doing, work, limit=ACT_LIMIT, watch=True):
        """Run `work()` on `tab` and answer with what it did to the page."""
        before_nav = tab.navigations
        before_url = tab.url
        before_tabs = {t.id for t in self.browser.pages()}
        before_console = tab.console_seq
        if watch:
            self._run(tab, "watch")
        work()
        took, why = hands.settle(self.browser, tab, limit, since_navigations=before_nav)
        if why == "limit" and limit < NAVIGATE_LIMIT and (tab.navigations > before_nav or not tab.loaded):
            # A click that started a page load is given a page load's time, not a click's.
            more, why = hands.settle(self.browser, tab, NAVIGATE_LIMIT - limit, since_navigations=before_nav)
            took = round(took + more, 2)
        return self._consequence(tab, doing, before_nav, before_url, before_tabs, took, why, watch,
                                 before_console)

    def _consequence(self, tab, doing, before_nav, before_url, before_tabs, took, why, watched,
                     before_console=0):
        out = {"did": doing, "settled_in": took}
        if why == "limit":
            out["still_loading"] = True
        new = [t for t in self.browser.pages() if t.id not in before_tabs]
        if new:
            out["new_tabs"] = [{"tab": short(t.id), "url": t.url, "title": t.title} for t in new]
            # A link that opened a tab: the work continues there, as it does for a person.
            opened_here = [t for t in new if t.opened_by == tab.id] or new
            self.browser.active = self.browser.attach(opened_here[-1])
            hands.settle(self.browser, self.browser.active, NAVIGATE_LIMIT / 2)
            out["now_in_tab"] = short(self.browser.active.id)
            tab = self.browser.active
        if tab.dialog:
            out["dialog"] = dict(tab.dialog)
            out["url"] = tab.url
            return self._with_console(out, tab, before_console)
        self._still_allowed(tab)       # a link or a window that went to a local file outside the home
        navigated = tab.navigations > before_nav or tab.url != before_url
        if not navigated and watched and not new:
            try:
                changes = self.browser.run(tab, "changes")
            except (cdp.CdpError, cdp.BrowserClosed):
                changes = None
            if changes and changes.get("appeared"):
                out["appeared"] = changes["appeared"]
            if changes and changes.get("context_menu"):
                out["menu"] = changes["context_menu"]
        page = self._marked(self._run(tab, "snapshot", False))
        out["url"] = page.get("url")
        out["title"] = page.get("title")
        if navigated or new:
            out["navigated"] = True
            out["page"] = page
        elif page.get("modal"):
            out["modal"] = page["modal"]
        # The page's errors and warnings that appeared while this ran (#1152): a click whose
        # handler threw, a page whose script did not parse. The run-and-look loop's one reading.
        return self._with_console(out, tab, before_console)

    # ── reading ──

    def status(self):
        """The view: whether the browser is open, its tabs, and the one actions go to, for the
        caller's own browser. It never waits behind an act for long: while one runs it answers
        with what that browser last showed (the caller's side, never the other's). A browser the
        caller may not use is `open: false` with the sentence in `refused`."""
        if not self.lock.acquire(timeout=1.0):
            return dict(self.sides.last[self.sides.side()], busy=True)
        try:
            try:
                side = self.sides.bind()
                return self._status(side)
            except guard.Denied as e:
                return {"open": False, "refused": str(e)}
        finally:
            self.lock.release()

    def _status(self, side):
        try:
            self.browser.connect()
        except (cdp.BrowserClosed, cdp.CdpError):
            self.browser.close()
            return {"open": False}
        pages = self.browser.pages()
        active = self.browser.active if self.browser.active in pages else (pages[0] if pages else None)
        if active is not None and active.session:
            # The target list's title arrives a moment after the page has it; ask the page.
            try:
                here = self.browser.run(active, "where", timeout=3)
                active.url, active.title = here.get("url", active.url), here.get("title", active.title)
            except (cdp.CdpError, cdp.BrowserClosed):
                pass
        self.sides.last[side] = {
            "open": True,
            "tabs": [{"tab": short(t.id), "title": str(t.title)[:100], "url": str(t.url)[:200],
                      "active": t is active} for t in pages],
            "dialog": active.dialog if active else None,
        }
        return self.sides.last[side]

    def _marked(self, page):
        """A reading with every control that reads as a commitment marked: the reader learns
        before trying that `click` will refuse it and `commit` will ask."""
        for e in page.get("elements") or []:
            if not e.get("context"):
                word = commitment_of(e)
                if word:
                    e["commitment"] = word
        return page

    def read(self, tab=None, all=False):
        with self.held():
            t = self._tab(tab)
            page = self._marked(self._run(t, "snapshot", bool(all)))
            page["tab"] = short(t.id)
            if t.dialog:
                page["dialog"] = dict(t.dialog)
            return self._with_console(page, t)

    def media(self, tab=None):
        with self.held():
            t = self._tab(tab)
            return {"media": self._run(t, "media"), "url": t.url, "title": t.title}

    def find(self, query, tab=None):
        if not str(query or "").strip():
            raise Refused("find needs words to look for")
        with self.held():
            t = self._tab(tab)
            got = self._run(t, "search", query, 40, True)
            found = self._marked({"elements": got["found"]})["elements"]
            return {"query": query, "found": found, "total": got["total"], "text": got["text"],
                    "loading": got["loading"], "url": t.url, "tab": short(t.id)}

    def text(self, tab=None, limit=TEXT_LIMIT):
        with self.held():
            t = self._tab(tab)
            got = self._run(t, "text", max(200, min(int(limit or TEXT_LIMIT), 200000)))
            got["url"] = t.url
            got["title"] = t.title
            return got

    def tabs(self):
        with self.held():          # a mind looking at the tabs starts Mind View's browser, as any web call does
            out = self.status()
        if out.get("refused"):
            raise PageRefused(out["refused"])
        return out

    def wait(self, text=None, seconds=5.0, tab=None):
        """Wait for words, or for the page to settle. The browser is held for one look at a time,
        not for the whole wait, so a thirty-second wait holds nobody else up."""
        seconds = max(0.5, min(float(seconds or 5), 30.0))
        deadline = time.monotonic() + seconds
        if not text:
            with self.held():
                t = self._tab(tab)
                took, why = hands.settle(self.browser, t, seconds)
                return {"settled_in": took, "still_loading": why == "limit", "url": t.url}
        while True:
            with self.held():
                t = self._tab(tab)
                found = self._run(t, "find", text, 1)
                if found:
                    return {"appeared": True, "element": found[0], "url": t.url}
                body = self._run(t, "text", 200000)
                if text.lower() in (body.get("text") or "").lower():
                    return {"appeared": True, "url": t.url}
            if time.monotonic() >= deadline:
                return {"appeared": False, "waited": seconds, "url": t.url}
            time.sleep(0.4)

    # ── going places ──

    def go(self, url, tab=None, new_tab=False):
        url = str(url or "").strip()
        if not url:
            raise Refused("go needs a url")
        url = self._local_url(url)
        if "://" not in url and not url.startswith(("about:", "chrome:")):
            url = "https://" + url
        scheme = urllib.parse.urlsplit(url).scheme.lower()
        if scheme == "file":
            self._vet_local_file(url)
        elif scheme not in ("http", "https", "about"):
            raise Refused("go opens web pages (http, https) and local files under the home "
                          "(file:///home/you/... or the path itself); `%s:` is not one" % scheme)
        with self.held():
            if new_tab:
                holds.release_all(self.browser, self._tab(None, leaving=True))
                before = {t.id for t in self.browser.pages()}
                t = self.browser.new_tab(url, agent=self._agent_calling())
                took, why = hands.settle(self.browser, t, NAVIGATE_LIMIT)
                return self._consequence(t, "opened %s in a new tab" % url, -1, "", before, took, why, False)
            t = self._tab(tab, leaving=True)
            holds.release_all(self.browser, t)
            return self._act(t, "went to %s" % url,
                             lambda: self._call("Page.navigate", {"url": url}, t),
                             limit=NAVIGATE_LIMIT, watch=False)

    def history(self, step, tab=None):
        with self.held():
            t = self._tab(tab)
            h = self._call("Page.getNavigationHistory", tab=t)
            index = h.get("currentIndex", 0) + step
            entries = h.get("entries", [])
            if not 0 <= index < len(entries):
                raise Refused("there is no page %s this one in this tab's history"
                              % ("before" if step < 0 else "after"))
            holds.release_all(self.browser, t)
            return self._act(t, "went %s to %s" % ("back" if step < 0 else "forward", entries[index].get("url")),
                             lambda: self._call("Page.navigateToHistoryEntry", {"entryId": entries[index]["id"]}, t),
                             limit=NAVIGATE_LIMIT, watch=False)

    def reload(self, tab=None):
        """Load the page again, from the network and not the cache (a Mind editing a file and
        reloading must see the edit); and say so when the browser still served part of it from a
        cache it could not bypass (a service worker's, a prefetch)."""
        with self.held():
            t = self._tab(tab)
            before = t.cache_hits
            holds.release_all(self.browser, t)
            out = self._act(t, "reloaded %s" % t.url, lambda: self._call("Page.reload", {"ignoreCache": True}, t),
                            limit=NAVIGATE_LIMIT, watch=False)
            hits = t.cache_hits - before
            out["from_cache"] = hits
            out["cache_bypassed"] = hits == 0 and not t.document_cached
            if out["cache_bypassed"]:
                out["did"] += ", bypassing the cache: everything came from the network"
            else:
                out["did"] += (", asking the browser to bypass its cache — but %d response%s still came "
                               "from a cache (a service worker's, or prefetched)%s, so the page may not "
                               "be fresh" % (hits, "" if hits == 1 else "s",
                                             "; the page itself did" if t.document_cached else ""))
            return out

    # ── hands ──

    def _pressable(self, t, ref, allow_commitment=False):
        target = self._target(t, ref)
        if target.get("frame_element"):
            raise PageRefused("%s is a frame from another site: what is inside it cannot be read, so it "
                              "is not pressed blind. Nothing was pressed." % ref)
        if target.get("not_a_control"):
            raise PageRefused("%s is a %s (\"%s\"), not a control: press one of the controls read lists "
                              "inside it. Nothing was pressed." % (ref, target["role"], target["name"]))
        if target.get("invisible"):
            raise PageRefused("%s (%s \"%s\") has no size on the page: it cannot be pressed. Read the "
                          "page again; it may be inside something closed." % (ref, target["role"], target["name"]))
        if target.get("disabled"):
            raise PageRefused("%s (%s \"%s\") is disabled: the page will not let it be pressed yet — "
                          "usually a field it needs is empty or invalid." % (ref, target["role"], target["name"]))
        cover = target.get("covered_by")
        if cover:
            raise PageRefused("%s (\"%s\") is covered by %s \"%s\" (%s): pressing there would press that "
                          "instead. Deal with it first — often a cookie or sign-in dialog — then try again."
                          % (ref, target["name"], cover.get("role"), cover.get("name"), cover.get("ref")))
        landing = target.get("lands_on")
        why = self._why_commitment(landing, t) if landing else None
        if why:
            # The press would land on a control inside this one, and that control is the
            # commitment: it is the one to name, and commit presses it by its own ref.
            raise PageRefused("pressing %s would press %s \"%s\" (%s) inside it, which %s. It is pressed "
                              "with commit ref=%s label=\"%s\" site=%s, which asks the person first. Nothing "
                              "was pressed."
                              % (ref, landing["role"], landing["name"], landing["ref"], why,
                                 landing["ref"], landing["name"], fold_site(t.url)))
        why = None if allow_commitment else self._why_commitment(target, t)
        if why:
            raise PageRefused("%s (%s \"%s\") %s: pressing it may spend money, send something or remove "
                              "something, and that cannot be taken back. It is pressed with commit ref=%s "
                              "label=\"%s\" site=%s, which asks the person first. Nothing was pressed."
                              % (ref, target["role"], target["name"], why, ref, target["name"], fold_site(t.url)))
        return target

    def click(self, ref, tab=None):
        with self.held():
            t = self._tab(tab)
            target = self._pressable(t, ref)
            return self._act(t, "clicked %s \"%s\" (%s)" % (target["role"], target["name"], ref),
                             lambda: hands.click(self.browser, t, target["x"], target["y"]))

    def commit(self, ref, label, site, tab=None):
        with self.held():
            t = self._tab(tab)
            here = fold_site(t.url)
            if fold_site(site) != here:
                raise PageRefused("this tab is on %s, not %s: nothing was pressed. The page changed since "
                              "the person was asked." % (here, fold_site(site)))
            if ref == "dialog":
                if not t.dialog:
                    raise PageRefused("no dialog is open on %s: nothing was accepted" % here)
                if not commit.same_label(t.dialog.get("message"), label):
                    raise PageRefused("the dialog says \"%s\", and label must be that whole message, as the "
                                      "person is shown it: nothing was accepted"
                                      % (t.dialog.get("message") or "")[:200])
                self._call("Page.handleJavaScriptDialog", {"accept": True}, t)
                return {"did": "accepted the dialog \"%s\"" % t.dialog.get("message"), "url": t.url}
            target = self._pressable(t, ref, allow_commitment=True)
            if not commit.same_label(target["name"], label):
                raise PageRefused("%s now reads \"%s\", not \"%s\": nothing was pressed. The page changed "
                              "since the person was asked." % (ref, target["name"], label))
            return self._act(t, "pressed %s \"%s\" (%s) on %s" % (target["role"], target["name"], ref, here),
                             lambda: hands.click(self.browser, t, target["x"], target["y"]))

    def type(self, ref, text, clear=True, enter=False, tab=None):
        with self.held():
            t = self._tab(tab)
            target = self._target(t, ref)
            if target.get("covered_by"):
                cover = target["covered_by"]
                raise PageRefused("%s (\"%s\") is covered by %s \"%s\" (%s): deal with that first. Nothing "
                                  "was typed." % (ref, target["name"], cover.get("role"), cover.get("name"),
                                                  cover.get("ref")))
            if enter:
                self._key_is_safe(t, ref, "Enter")
            focused = self._run(t, "focus", ref, bool(clear))
            if not focused or focused.get("gone"):
                raise PageRefused("%s is no longer on the page; read it again" % ref)
            if not focused.get("focused"):
                raise PageRefused("%s (%s \"%s\") would not take the focus, so nothing was typed. It may "
                              "not be a field; read the page again." % (ref, focused.get("role"), focused.get("name")))

            def work():
                if clear:
                    hands.press(self.browser, t, "delete")
                hands.insert_text(self.browser, t, str(text))
                if enter:
                    self._key_is_safe(t, ref, "Enter", typed=True)
                    hands.press(self.browser, t, "enter")
            shown = "(%d characters)" % len(str(text)) if focused.get("password") else "\"%s\"" % str(text)[:80]
            return self._act(t, "typed %s into %s \"%s\" (%s)%s" % (
                shown, focused.get("role"), focused.get("name"), ref, " and pressed Enter" if enter else ""),
                work, limit=NAVIGATE_LIMIT if enter else ACT_LIMIT)

    def press(self, key, ref=None, tab=None):
        k = hands.key_named(key)
        if k is None:
            raise Refused("`%s` is not a key this knows. Keys: %s, a letter or digit, with ctrl+, "
                          "shift+, alt+ or meta+ in front" % (key, ", ".join(sorted(hands.KEYS))))
        with self.held():
            t = self._tab(tab)
            if ref:
                focused = self._run(t, "focus", ref, False)
                if not focused or focused.get("gone"):
                    raise PageRefused("%s is no longer on the page; read it again" % ref)
            self._key_is_safe(t, ref, k[0])

            def work():
                self._key_is_safe(t, ref, k[0])
                hands.press(self.browser, t, key)
            return self._act(t, "pressed %s%s" % (key, (" in %s" % ref) if ref else ""), work,
                             limit=NAVIGATE_LIMIT if k[0] == "Enter" else ACT_LIMIT)

    def select(self, ref, option, tab=None):
        with self.held():
            t = self._tab(tab)
            peek = self._run(t, "pick", ref, str(option), True) or {}
            if peek.get("would_choose"):
                word = commit.reads_as_commitment(peek["would_choose"])
                if word:
                    raise PageRefused("choosing \"%s\" in %s reads as a commitment (\"%s\"): a list that "
                                      "acts when it changes would do it at once. That is the person's to "
                                      "choose; nothing was chosen." % (peek["would_choose"], ref, word))
            got = {}

            def work():
                got.update(self._run(t, "pick", ref, str(option)) or {})
            out = self._act(t, "chose \"%s\" in %s" % (option, ref), work)
            if got.get("gone"):
                raise PageRefused("%s is no longer on the page; read it again" % ref)
            if got.get("not_select"):
                raise PageRefused("%s is a %s, not a list to choose from: click it to open it, then click "
                              "the option" % (ref, got.get("role") or "control"))
            if got.get("no_option"):
                raise PageRefused("%s has no option \"%s\". It has: %s" % (ref, option, ", ".join(got.get("options", []))))
            out["did"] = "chose \"%s\" in %s" % (got.get("chosen"), ref)
            return out

    def scroll(self, direction="down", ref=None, tab=None):
        with self.held():
            t = self._tab(tab)
            if ref:
                moved = self._run(t, "scrollBy", 0, ref)
                if moved.get("gone"):
                    raise Refused("%s is no longer on the page; read it again" % ref)
            else:
                d = str(direction or "down").lower()
                page = self._run(t, "scrollBy", 0, None)
                h = page["viewport"][1]
                dy = {"down": h * 0.85, "up": -h * 0.85, "top": -10 ** 7, "bottom": 10 ** 7}.get(d)
                if dy is None:
                    raise Refused("scroll goes up, down, top or bottom — or to a ref")
                self._run(t, "scrollBy", int(dy), None)
            hands.settle(self.browser, t, 1.5)
            page = self._marked(self._run(t, "snapshot", False))
            page["tab"] = short(t.id)
            return page

    def dialog(self, accept=True, text=None, tab=None):
        with self.held():
            t = self._tab(tab)
            if not t.dialog:
                raise PageRefused("no dialog is open in this tab")
            message = " ".join(str(t.dialog.get("message", "")).split())
            word = commit.reads_as_commitment(message)
            if accept and word and t.dialog.get("type") in ("confirm", "prompt", "beforeunload"):
                raise PageRefused("the dialog asks \"%s\", which reads as a commitment (\"%s\"). It is accepted "
                              "with commit ref=dialog label=\"%s\" site=%s, which asks the person first."
                              % (message[:120], word, message[:120], fold_site(t.url)))
            params = {"accept": bool(accept)}
            if text is not None and t.dialog.get("type") == "prompt":
                params["promptText"] = str(text)
            self._call("Page.handleJavaScriptDialog", params, t)
            took, why = hands.settle(self.browser, t, ACT_LIMIT)
            return {"did": "%s the %s \"%s\"" % ("accepted" if accept else "dismissed",
                                                 t.dialog and t.dialog.get("type") or "dialog", message[:120]),
                    "url": t.url, "settled_in": took}

    def switch(self, tab):
        with self.held():
            t = self._tab(tab)
            if self.browser.active is not None and self.browser.active is not t:
                holds.release_all(self.browser, self.browser.active)
            self._call("Target.activateTarget", {"targetId": t.id})
            self.browser.active = t
            return {"did": "switched to tab %s" % short(t.id), "url": t.url, "title": t.title}

    def close_tab(self, tab):
        with self.held():
            t = self._tab(tab)
            if len(self.browser.pages()) <= 1:
                raise Refused("that is the browser's last tab; closing it would close the browser")
            holds.release_all(self.browser, t)
            self._call("Target.closeTarget", {"targetId": t.id})
            if self.browser.active is t:
                self.browser.active = None
            return {"did": "closed tab %s (%s)" % (short(t.id), t.url)}
