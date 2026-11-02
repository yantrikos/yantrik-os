"""A browser of its own for the pages a Mind made: headless, a throwaway profile, no network.

A Mind that builds a page has to look at it (#1068). That page is the Mind's own code, and in the
desktop's browser (the one on :9222, the person's profile) it would run with the person's cookies
and network. So a `file:` address from a Mind never goes there. It goes to this instance:

  - headless, with a profile made for this run and removed when it ends: no cookies, no sign-ins;
  - no network: names do not resolve (`--host-resolver-rules`), every request goes to a proxy that
    is not there, loopback included (`--proxy-bypass-list=<-loopback>`) - the desktop's own debug
    port is on loopback - and `Fetch` fails whatever is not a local file the Mind may read
    (`guard`, below). A page's script cannot reach the person's browser or the network;
  - no second window or tab a page opens for itself (`--block-new-web-contents`);
  - its debug port is its own, on loopback, chosen free, and allows no web origin but itself.

It still runs as the person, so it can read what the person can; `local_page.check` is asked for
every `file:` request of every page and frame, and a request outside what a Mind may read fails.
"""

import atexit
import os
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.request

from . import cdp, local_page

CHROME_NAMES = ("chromium", "chromium-browser", "google-chrome", "google-chrome-stable", "chrome")
START_LIMIT = 15.0

# What a page may load besides local files: nothing from the network, only what it carries itself.
INLINE = ("data:", "blob:", "about:", "chrome-error:", "filesystem:")


# A file a page asked for and the shell allowed, for a few seconds: a page of thirty images asks
# the shell once, not thirty times. Short, so a link swapped in meanwhile is seen soon.
ALLOWED_FOR = 3.0
_ALLOWED = {}


def chrome_path():
    for name in CHROME_NAMES:
        path = shutil.which(name)
        if path:
            return path
    return None


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def args_for(profile, port):
    """The browser's arguments. Pure, so a test can read them."""
    return [
        "--headless=new",
        "--disable-gpu",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--disable-background-networking",
        "--block-new-web-contents",
        "--user-data-dir=" + profile,
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=%d" % port,
        "--remote-allow-origins=http://127.0.0.1:%d" % port,
        "--host-resolver-rules=MAP * ~NOTFOUND",
        "--proxy-server=socks5://127.0.0.1:9",
        "--proxy-bypass-list=<-loopback>",
        "about:blank",
    ]


def guard(url, stat=None):
    """Judge one request of a Mind's page: a local file the Mind may read, or something inline.
    Raises `local_page.LocalRefused` for anything else (the network included)."""
    if url.lower().startswith(INLINE):
        return
    if local_page.is_file_url(url):
        now = time.monotonic()
        if stat is None and _ALLOWED.get(url, 0) > now:
            return
        local_page.check(url, stat)
        if stat is None:
            if len(_ALLOWED) > 512:
                _ALLOWED.clear()
            _ALLOWED[url] = now + ALLOWED_FOR
        return
    raise local_page.LocalRefused("a page of your own loads nothing from the network")


class Instance:
    """The render browser: started on first use, stopped when this process ends."""

    def __init__(self):
        self.proc = None
        self.profile = None
        self.log = None
        self.browser = None

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def start(self):
        """The `cdp.Browser` for the render instance, starting it if it is not running."""
        if self.alive() and self.browser is not None:
            return self.browser
        self.stop()
        path = chrome_path()
        if not path:
            raise local_page.LocalRefused("there is no browser on this machine to look at a page with")
        self.profile = tempfile.mkdtemp(prefix="yantrik-render-")
        port = free_port()
        self.log = os.path.join(self.profile + ".log")
        with open(self.log, "wb") as out:
            self.proc = subprocess.Popen([path] + args_for(self.profile, port), stdin=subprocess.DEVNULL,
                                         stdout=out, stderr=out)
        atexit.register(self.stop)
        address = "http://127.0.0.1:%d" % port
        deadline = time.monotonic() + START_LIMIT
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                break
            try:
                with urllib.request.urlopen(address + "/json/version", timeout=1):
                    break
            except OSError:
                time.sleep(0.2)
        else:
            raise self._failed()
        if self.proc.poll() is not None:
            raise self._failed()
        self.browser = cdp.Browser(address)
        self.browser.guard = guard
        return self.browser

    def _failed(self):
        """Why it did not start, in the browser's own last words."""
        try:
            with open(self.log, "rb") as f:
                f.seek(0, os.SEEK_END)
                f.seek(max(0, f.tell() - 600))
                tail = f.read().decode("utf-8", "replace").strip().splitlines()[-4:]
        except OSError:
            tail = []
        self.stop()
        return local_page.LocalRefused("the browser for your own pages did not start" +
                                       (": " + " | ".join(tail) if tail else ""))

    def stop(self):
        if self.browser is not None:
            self.browser.close()
            self.browser = None
        if self.proc is not None:
            if self.proc.poll() is None:
                self.proc.terminate()
                try:
                    self.proc.wait(5)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
            self.proc = None
        if self.profile:
            shutil.rmtree(self.profile, ignore_errors=True)
            try:
                os.remove(self.profile + ".log")
            except OSError:
                pass
            self.profile = None
