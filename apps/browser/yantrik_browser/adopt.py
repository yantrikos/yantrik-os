"""Taking over the person's open tabs when the service connects (gate 5, bar 4).

The person's Chromium outlives this service, so a restart leaves the dead service's sessions
attached to every tab, with `Fetch` still holding `file:` requests for a decider that is gone.
Here: the tabs are adopted at once (a session, the fence, a reload only where a `file:` frame is
loaded), the arrival of the pipe is serviced until the lock is taken, and the stale sessions
are learned and, once nothing is paused on them and no tab can still hold what the dead
service left paused, detached. That last condition is the one that matters: detaching a session
lets what it holds continue, so it is done only when a navigation has committed behind the new
fence on every tab that session might have been on. If one adoption failed or timed out, the
stale session stays (failing closed; it costs a little growth, never an unvetted request).
"""

import threading
import time

ADOPT_WAIT = 3.0       # seconds adoption waits on one tab's reload: far below the driver's BUSY_WAIT
CLOSE_WAIT = 5.0       # seconds a connection waits for the pages it closed to be gone
ARRIVAL_RETRY = 0.5    # seconds between tries for the driver lock when the pipe arrives


def _try_adopt(browser, tab, wait=ADOPT_WAIT):
    """Adopt one tab; a failure leaves it a gap (the stale session stays: fail closed)."""
    from .cdp import BrowserClosed, CdpError
    try:
        browser.attach(tab, wait=wait)
    except (CdpError, BrowserClosed):
        pass


def adopt_pages(browser):
    """Every open page gets its session and its fence now, not when a command first touches it.
    One tab that cannot be adopted is left to the call that reaches it; the others do not wait
    on it. The reload wait is bounded (ADOPT_WAIT), so the caller's lock is not held for long."""
    workers = [threading.Thread(target=_try_adopt, args=(browser, t), daemon=True, name="cdp-adopt")
               for t in snapshot(browser)]
    for w in workers:
        w.start()
    end = time.monotonic() + ADOPT_WAIT * 2
    for w in workers:
        w.join(max(0.0, end - time.monotonic()))
    browser.adopted = True
    sweep(browser)


def close_drawn_pages(browser, infos):
    """A browser held from its start (Mind View's) is connected after it has been running for a
    while: the service takes its stream only once the shell has offered it and the checks have
    passed, and again after every restart of the service. What a page drew in that window was never
    fenced, and would stay on screen (and in a screenshot of the window) until a call touched its
    tab. A page cannot be judged untouched by its URL: an `about:blank` page that a `file:` page
    wrote into still shows what it wrote. So, with the fence already up for new pages, ONE fresh
    `about:blank` is opened first (closing the last page would close the browser, and Mind View is
    never left with none), and then EVERY page target that existed before it is closed, whatever
    its URL. Returns the targets that stay: the fresh page and what is not a page. A page that
    cannot be closed fails the connection: nothing of it is left on screen to be trusted."""
    old = [i for i in infos if i.get("type") == "page"]
    fresh = browser.call("Target.createTarget", {"url": "about:blank"})
    for info in old:
        browser.call("Target.closeTarget", {"targetId": info["targetId"]})
    gone = {i["targetId"] for i in old}
    # The browser closes a page after it answers, so the page is waited for to be gone: on return
    # nothing of it is on screen.
    end = time.monotonic() + CLOSE_WAIT
    while True:
        left = [i for i in browser.call("Target.getTargets").get("targetInfos", []) if i.get("targetId") in gone]
        if not left:
            kept = [i for i in infos if i.get("targetId") not in gone]
            if fresh.get("targetId"):
                kept.append({"targetId": fresh["targetId"], "type": "page", "url": "about:blank"})
            return kept
        if time.monotonic() > end:
            from .cdp import CdpError
            raise CdpError("a page the browser drew before it was held would not close")
        time.sleep(0.05)


def snapshot(browser):
    """The open pages, taken under the lock the reader holds when it adds a tab."""
    with browser.cond:
        return browser.pages()


def after_commit(browser):
    """A tab's gap just closed: stale sessions may be detachable now."""
    if browser.stale and browser.adopted:
        browser.fence.arming.submit(sweep, browser, True)


def stop_preattached(browser, tab):
    """Stop what the dead service's session may have paused on a pre-attached tab (a `file:`
    navigation has no event for anyone). `preattached` is cleared only once the browser has
    answered the stop, so every later pass through attach sends it again."""
    if tab.preattached and tab.session and not browser.held_at_start:
        browser.call("Page.stopLoading", session=tab.session)
        tab.preattached = False


def close_gap_unreloaded(browser, tab):
    """A pre-attached tab with no `file:` frame is not reloaded (a reload re-sends a POST without
    asking, and drops what the person typed). Fetch pauses only `file:` requests, and Blink refuses
    those from a non-`file:` document, so the one request such a tab could hold on the dead
    service's session is a `file:` navigation, which a successful `Page.stopLoading` cancelled.
    Without that stop the gap stays (fail closed)."""
    if tab.gap and not tab.preattached and not browser.held_at_start:
        tab.gap = False
        tab.watch = True
        after_commit(browser)


def regap(browser, tab):
    """A main-frame commit on a watched tab. To a `file:` URL we did not start (a navigation the
    dead service let through that was already commit-pending, so no stop cancelled it): the gap is
    open again, and the tab is loaded once more behind the fence. Any other commit ends the watch.
    Called from the reader's thread, so the reload is done on another."""
    if not tab.watch:
        return False
    if not str(tab.url).startswith("file:"):
        tab.watch = False
        return False
    tab.watch = False
    tab.gap, tab.fence_nav = True, tab.navigations
    tab.ready = False                    # the next attach reloads it (a `file:` frame is in it)
    browser.fence.arming.submit(_try_adopt, browser, tab)
    return True


def _retry(browser, tabs):
    """Adopt again a gap tab that is not ready (its adoption failed or timed out), so a tab that is
    never touched by a call cannot hold the cleanup back for the whole connection."""
    for tab in tabs:
        if tab.gap and not tab.ready:
            _try_adopt(browser, tab)


def learn(browser, session):
    """An event came on a session no tab here owns: a stale one of the dead service's."""
    if session and not browser.held_at_start and session not in browser.by_session \
            and session not in browser.stale:
        browser.stale.add(session)
        after_commit(browser)


def sweep(browser, retry=False):
    """Detach stale sessions, never while any tab that was held by another session has not had a
    navigation commit behind the new fence, and never while something paused on them is undecided.
    Pages opened after adoption have no gap, so they do not hold the cleanup back."""
    if browser.held_at_start or not browser.adopted or not browser.stale:
        return
    if retry:
        with browser.cond:
            pages = browser.pages()
        _retry(browser, pages)
    with browser.cond:
        pages = browser.pages()
        tabs = list(browser.tabs.values())
        gap = any(t.gap for t in pages)
    if gap:                             # a page that may hold a gap request: leave it failing closed
        return
    for session in list(browser.stale):
        if session in browser.by_session or any(t.session == session for t in tabs):
            browser.stale.discard(session)          # it was ours: only seen before it was known
            continue
        if browser.fence.pending_on(session):
            continue
        browser.stale.discard(session)
        try:
            browser.call("Target.detachFromTarget", {"sessionId": session})
        except Exception:  # noqa: BLE001 - gone already, or the browser is
            pass


def arrived(driver, side, serial, holder, errors):
    """A browser's pipe was given. Connect to it, retrying until the driver's lock is taken or
    this hand-over is no longer the held one: an arrival is never dropped because a call is busy.
    Mind View's browser connects here too, which holds and fences whatever page it already has."""
    while True:
        held = holder.held
        if held is None or held.serial != serial:
            return
        if driver.lock.acquire(timeout=ARRIVAL_RETRY):
            break
    try:
        browser = driver.sides.ready(side)
        if browser is not None:
            browser.connect()
    except errors:
        pass
    finally:
        driver.lock.release()
