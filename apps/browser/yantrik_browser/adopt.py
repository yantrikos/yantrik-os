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
ARRIVAL_RETRY = 0.5    # seconds between tries for the driver lock when the pipe arrives


def adopt_pages(browser):
    """Every open page gets its session and its fence now, not when a command first touches it.
    One tab that cannot be adopted is left to the call that reaches it; the others do not wait
    on it. The reload wait is bounded (ADOPT_WAIT), so the caller's lock is not held for long."""
    from .cdp import BrowserClosed, CdpError

    def adopt(tab):
        try:
            browser.attach(tab, wait=ADOPT_WAIT)
        except (CdpError, BrowserClosed):
            pass
    workers = [threading.Thread(target=adopt, args=(t,), daemon=True, name="cdp-adopt")
               for t in snapshot(browser)]
    for w in workers:
        w.start()
    end = time.monotonic() + ADOPT_WAIT * 2
    for w in workers:
        w.join(max(0.0, end - time.monotonic()))
    browser.adopted = True
    sweep(browser)


def snapshot(browser):
    """The open pages, taken under the lock the reader holds when it adds a tab."""
    with browser.cond:
        return browser.pages()


def after_commit(browser):
    """A tab's gap just closed: stale sessions may be detachable now."""
    if browser.stale and browser.adopted:
        browser.fence.arming.submit(sweep, browser, True)


def close_gap_unreloaded(browser, tab):
    """A pre-attached tab with no `file:` frame is not reloaded (a reload re-sends a POST without
    asking, and drops what the person typed). Fetch pauses only `file:` requests, and Blink refuses
    those from a non-`file:` document, so the one request such a tab could hold on the dead
    service's session is a `file:` navigation, which `Page.stopLoading` (answered by now) cancelled.
    Nothing can be paused on it: its gap is closed."""
    if tab.gap and not browser.held_at_start:
        tab.gap = False
        after_commit(browser)


def _retry(browser, tabs):
    """Adopt again a gap tab that is not ready (its adoption failed or timed out), so a tab that is
    never touched by a call cannot hold the cleanup back for the whole connection."""
    from .cdp import BrowserClosed, CdpError
    for tab in tabs:
        if tab.gap and not tab.ready:
            try:
                browser.attach(tab, wait=ADOPT_WAIT)
            except (CdpError, BrowserClosed):
                pass       # still a gap: the stale session stays (fail closed)


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


def person_arrived(driver, serial, holder, ports, errors):
    """The person's pipe was given. Connect to it, retrying until the driver's lock is taken or
    this hand-over is no longer the held one: an arrival is never dropped because a call is busy."""
    while True:
        held = holder.held
        if held is None or held.serial != serial:
            return
        if driver.lock.acquire(timeout=ARRIVAL_RETRY):
            break
    try:
        browser = driver.sides.ready(ports.PERSON)
        if browser is not None:
            browser.connect()
    except errors:
        pass
    finally:
        driver.lock.release()
