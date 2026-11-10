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

from yantrik_surface import files as home_files
from yantrik_surface.surface import Refusal as SurfaceRefusal

from . import cdp, commit, guard, hands, judgement, sides

NAVIGATE_LIMIT = 15.0     # seconds a navigation is given to settle
ACT_LIMIT = 4.0           # seconds a click or a key is given
TEXT_LIMIT = 20000


class Refused(Exception):
    """An action declining, with the sentence the caller reads."""


class PageRefused(Refused):
    """The page said no and nothing was pressed or typed: a ref that is gone, a control covered
    or disabled, a commitment. Answered with `PAGE:` in front, which the MCP bridge reads as a
    policy answer — "REFUSED — nothing was run" — rather than a failure to try again."""

    def __init__(self, sentence):
        super().__init__("PAGE: " + sentence)


BUSY_WAIT = 20.0      # seconds an act waits for the one before it to finish

# The page's console (#1152): what `read` and every act carry of it, and what `console` lists.
CONSOLE_IN_READ = 8       # recent errors and warnings a reading or an act's answer carries
CONSOLE_LIMIT = 30        # lines `console` lists at once
ERRORS = ("error", "warning")

# Controls that Enter or Space presses when they have the focus.
PRESSED_BY_KEYS = {"button", "link", "clickable", "menuitem", "menuitemcheckbox", "menuitemradio",
                   "tab", "switch", "checkbox", "radio", "option", "treeitem"}


def short(target_id):
    return target_id[:8]


def host_of(url):
    try:
        return (urllib.parse.urlsplit(url).hostname or "").lower()
    except ValueError:
        return ""


def file_path_of(url):
    """The absolute path a `file:` URL names, or a Refused saying why it names none: a host
    (`file://server/share`), a relative path, an empty one. The query and fragment are not part
    of the path (`?v=3` is a cache-buster, and Chromium ignores it for files)."""
    parts = urllib.parse.urlsplit(url)
    if parts.netloc not in ("", "localhost"):
        raise Refused("a local file has no host: `file://%s` is not a file on this machine" % parts.netloc)
    path = urllib.parse.unquote(parts.path or "")
    if not path.startswith("/"):
        raise Refused("a local file is named by its whole path, like file:///home/you/site/index.html")
    return path


def fold_site(site):
    s = str(site or "").strip().lower()
    if "://" in s:
        s = host_of(s)
    s = s.split("/")[0]
    return s[4:] if s.startswith("www.") else s


def commitment_of(element):
    """The commitment a control's label carries.

    A link with an address navigates, which Back undoes: Hacker News's "submit" opens the form, it
    does not submit anything. So a link is let through on the soft words (commit.LINK_WORDS) —
    and only when nothing but its address decides what it does. A link that says delete, pay or
    unsubscribe, or one with data-method or an onclick on it, is judged like the button it is."""
    word = commit.reads_as_commitment(element.get("name"))
    if (word and element.get("role") == "link" and element.get("href")
            and not element.get("scripted") and word in commit.LINK_WORDS):
        return None
    return word


def _favicon_noise(entry):
    """The one error a clean page produces: the browser asking for /favicon.ico and not finding
    it. Left out of the errors a reading carries (it says nothing about the page's code); kept
    where `everything` is asked for."""
    return entry.get("kind") == "browser.network" and str(entry.get("at", "")).endswith("/favicon.ico")


class Driver:
    """The browser surface's actions; `browser` is this call's: a mind's is Mind View's, the person's theirs."""

    def __init__(self, browser=None, judge=None, guard_=None, file_rule=None, home=None):
        self.sides = sides.Sides(browser, guard_)
        # The person's decision model, asked when the words say nothing (judgement.py). `False`
        # turns it off (tests of the words alone); None is the shell's `decide`.
        self.judge = None if judge is False else (judge or judgement.Judgement())
        # The OS's own rule for a file an agent names (yantrik_surface.files: in the home, outside
        # its protected places, asked of the shell). `False` turns it off (a test's own home).
        self.file_rule = None if file_rule is False else (file_rule or home_files.may_read_file)
        self.home = home or (lambda: os.path.expanduser("~"))
        self.lock = threading.RLock()

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
            t = self.browser.tab(tab)
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

    # ── local files ──

    def _vet_local_file(self, url):
        """The real path of the file a `file:` URL may open, or Refused: a regular file under the
        home, reached through no link that leaves it — and, for an agent, one the OS's file rule
        allows (the shell's `files_stat`: outside the home's protected places). The rule is asked
        with the path as written; it walks the links itself."""
        path = file_path_of(url)
        home = os.path.realpath(self.home())
        real = os.path.realpath(path)
        if not (real == home or real.startswith(home.rstrip(os.sep) + os.sep)):
            linked = "" if real == os.path.normpath(path) else " (it is a link to %s)" % real
            raise Refused("go opens local files only under the home, %s; %s is outside it%s. "
                          "Nothing was opened." % (home, path, linked))
        if not os.path.lexists(real) and not os.path.exists(real):
            raise Refused("there is no file at %s; nothing was opened" % path)
        if os.path.isdir(real):
            raise Refused("%s is a folder; go opens a file — name one inside it (index.html, say)" % path)
        if not os.path.isfile(real):
            raise Refused("%s is not a regular file; nothing was opened" % path)
        if self.file_rule is not None:
            try:
                self.file_rule(path)
            except SurfaceRefusal as e:
                raise PageRefused("%s. Nothing was opened." % str(e).rstrip(".")) from None
        return real

    def _still_allowed(self, t):
        """A tab that has gone to a local file (a link the page held, a window it opened) is held
        to the same rule as `go`: outside the home, it is not read — it is sent to about:blank and
        the act is refused saying so. Checked once per address; `go` passes `leaving` to skip it."""
        url = str(t.url or "")
        if not url.lower().startswith("file:") or getattr(t, "vetted_file", None) == url:
            return
        try:
            self._vet_local_file(url)
        except Refused as e:
            try:
                self._call("Page.navigate", {"url": "about:blank"}, t)
                hands.settle(self.browser, t, 3.0)
            except Refused:
                pass
            raise PageRefused("this tab had gone to %s, which the Browser does not open: %s It was "
                              "taken to about:blank, and nothing of it was read." % (url, str(e))) from None
        t.vetted_file = url

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

    # ── the page's console ──

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
        if url.startswith("~/"):
            url = os.path.join(self.home(), url[2:])
        if url.startswith("/"):
            # A path is a local file (#1152: a Mind opening the build it just wrote).
            url = "file://" + urllib.parse.quote(url)
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
                self._tab(None, leaving=True)
                before = {t.id for t in self.browser.pages()}
                t = self.browser.new_tab(url)
                if scheme == "file":
                    t.vetted_file = url
                took, why = hands.settle(self.browser, t, NAVIGATE_LIMIT)
                return self._consequence(t, "opened %s in a new tab" % url, -1, "", before, took, why, False)
            t = self._tab(tab, leaving=True)
            if scheme == "file":
                t.vetted_file = url        # vetted above; the answer's own check need not ask again
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
            return self._act(t, "went %s to %s" % ("back" if step < 0 else "forward", entries[index].get("url")),
                             lambda: self._call("Page.navigateToHistoryEntry", {"entryId": entries[index]["id"]}, t),
                             limit=NAVIGATE_LIMIT, watch=False)

    def reload(self, tab=None):
        with self.held():
            t = self._tab(tab)
            return self._act(t, "reloaded %s" % t.url, lambda: self._call("Page.reload", {}, t),
                             limit=NAVIGATE_LIMIT, watch=False)

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
            self._call("Target.activateTarget", {"targetId": t.id})
            self.browser.active = t
            return {"did": "switched to tab %s" % short(t.id), "url": t.url, "title": t.title}

    def close_tab(self, tab):
        with self.held():
            t = self._tab(tab)
            if len(self.browser.pages()) <= 1:
                raise Refused("that is the browser's last tab; closing it would close the browser")
            self._call("Target.closeTarget", {"targetId": t.id})
            if self.browser.active is t:
                self.browser.active = None
            return {"did": "closed tab %s (%s)" % (short(t.id), t.url)}
