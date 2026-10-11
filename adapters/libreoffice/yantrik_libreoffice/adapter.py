"""The adapter as a program: serve LibreOffice's surface for as long as there is a LibreOffice to
serve it for.

How it is started, and when it stops:

  * **by the desktop** — the shell opens LibreOffice through `yantrik-libreoffice.desktop`, whose
    `Exec` starts LibreOffice listening on the UNO pipe, and starts this program beside it
    (`X-Yantrik-Adapter`) with `YANTRIK_SURFACE` (the id to bind) and `YANTRIK_APP_PID` (the
    process it serves) in its environment. It stops on SIGTERM, which the shell sends when that
    process exits, and on its own once that process has gone and no LibreOffice answers on the
    pipe — a LibreOffice that was already running takes a second launch's request and the second
    process exits at once, and the adapter keeps serving the one that is still there.
  * **by hand** — `yantrik-libreoffice-adapter` with no `YANTRIK_APP_PID` serves until stopped,
    and describe says whether LibreOffice is there.
  * **headless** — `--headless` starts a LibreOffice of its own with no window and a private
    profile, and stops it on the way out: for scripts, tests and a machine with no display.
"""

import argparse
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from yantrik_surface import SocketBusy, StopOnSignal

from .office import PIPE, Office
from .surface import APP_ID, build

# How long after the app process has gone the adapter waits for a LibreOffice to answer before
# it decides there is nothing left to serve (`YANTRIK_LIBREOFFICE_GRACE` seconds, for tests).
GRACE = 10.0
# Between two looks at the app process, the pipe and our own LibreOffice.
TICK = 1.0


def alive(pid):
    """Whether a process with this pid exists (as far as this user can tell)."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def reachable(office):
    try:
        office.desktop()
        return True
    except Exception:  # noqa: BLE001 - not reachable, for whatever reason
        return False


def start_headless(soffice, pipe, profile):
    """A LibreOffice with no window and a profile of its own, listening on `pipe`."""
    command = [soffice, "--headless", "--invisible", "--nologo", "--norestore", "--nodefault",
               "--nolockcheck", "-env:UserInstallation=file://%s" % profile,
               "--accept=pipe,name=%s;urp;" % pipe]
    return subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, start_new_session=True)


def stop_headless(process, office, patience=10.0):
    """Ask our LibreOffice to quit, then make sure it has."""
    office.terminate()
    try:
        process.wait(timeout=patience)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except OSError:
            process.kill()
        process.wait()


def surface_id():
    """The id to bind: the shell's `YANTRIK_SURFACE`, or `libreoffice`."""
    wanted = (os.environ.get("YANTRIK_SURFACE") or "").strip() or APP_ID
    words = wanted.split("-")
    if not all(w and w.isascii() and w.isalnum() and w == w.lower() for w in words):
        raise SystemExit("yantrik-libreoffice-adapter: YANTRIK_SURFACE=%r is not a surface name "
                         "(lowercase words joined by -)" % wanted)
    return wanted


def main(argv=None, office=None):
    parser = argparse.ArgumentParser(
        prog="yantrik-libreoffice-adapter",
        description="Serve LibreOffice's Yantrik surface (app-libreoffice.sock) over UNO.")
    parser.add_argument("--pipe", default=os.environ.get("YANTRIK_LIBREOFFICE_PIPE") or PIPE,
                        help="the UNO pipe LibreOffice listens on (default: %(default)s)")
    parser.add_argument("--headless", action="store_true",
                        help="start a LibreOffice of its own, with no window and a private "
                             "profile, and stop it on the way out")
    parser.add_argument("--soffice", default=os.environ.get("YANTRIK_SOFFICE") or "soffice",
                        help="the LibreOffice program for --headless (default: %(default)s)")
    args = parser.parse_args(argv)

    app_id = surface_id()
    try:
        grace = float(os.environ.get("YANTRIK_LIBREOFFICE_GRACE") or GRACE)
    except ValueError:
        grace = GRACE
    app_pid = os.environ.get("YANTRIK_APP_PID", "").strip()
    app_pid = int(app_pid) if app_pid.isdigit() else None

    headless, profile = None, None
    pipe = args.pipe
    if args.headless:
        if shutil.which(args.soffice) is None:
            print("yantrik-libreoffice-adapter: --headless needs LibreOffice, and there is no %r "
                  "on PATH" % args.soffice, file=sys.stderr)
            return 1
        # A pipe and a profile of our own, so a LibreOffice the person has open is never touched.
        pipe = "%s-%d" % (args.pipe, os.getpid())
        profile = tempfile.mkdtemp(prefix="yantrik-libreoffice-profile-")
        headless = start_headless(args.soffice, pipe, profile)

    # Before the socket exists, not after. The socket is what tells the shell — and a test — that
    # the adapter is up, and the server thread answers on it at once; installed after the bind, a
    # SIGTERM sent on that cue could arrive before the main thread got here and kill the adapter
    # with Python's default action: no "stopped" line, and the socket left behind. Seen in CI as a
    # return code of -15 on a loaded runner. A stop asked for before the bind finishes is kept in
    # `stop`, and the loop below ends on its first look. A flag, not a threading.Event: a handler
    # calling Event.set() while this thread is inside Event.wait() deadlocks (yantrik_surface's
    # stopping.py); that is the hang this file's clean-stop test still hit after the move.
    stop = StopOnSignal()

    office = office if office is not None else Office(pipe, hidden=args.headless)
    surface = build(office, app_id)
    try:
        server = surface.serve_in_thread()
    except (SocketBusy, OSError) as e:
        print("yantrik-libreoffice-adapter: not serving: %s" % e, file=sys.stderr)
        if headless is not None:
            stop_headless(headless, office)
            shutil.rmtree(profile, ignore_errors=True)
        return 1

    print("[yantrik] %s answering on %s for the LibreOffice on the pipe `%s`%s"
          % (app_id, server.path, pipe,
             " (serving pid %d)" % app_pid if app_pid else " (headless)" if headless else ""),
          file=sys.stderr, flush=True)

    why, gone_since = "asked to stop", None
    try:
        while not stop.wait(TICK):
            if headless is not None and headless.poll() is not None:
                why = "its LibreOffice exited"
                break
            if app_pid is None or alive(app_pid):
                gone_since = None
                continue
            # The process the desktop started is gone. A LibreOffice still answering on the pipe
            # is one that process handed its request to: keep serving it.
            if reachable(office):
                gone_since = None
                continue
            gone_since = gone_since or time.monotonic()
            if time.monotonic() - gone_since >= grace:
                why = "LibreOffice has gone"
                break
    finally:
        surface.stop()
        if headless is not None:
            stop_headless(headless, office)
            shutil.rmtree(profile, ignore_errors=True)
    print("[yantrik] %s stopped: %s" % (app_id, why), file=sys.stderr, flush=True)
    return 0
