"""The Yantrik control surface, inside Blender.

Blender is not ours. It is a program this OS opens, like the browser. What makes it an app
of this desktop rather than a window a mind can only photograph is this addon: it binds
`app-blender.sock` in the session's socket directory and answers `app.describe` and
`app.act` in the same JSON-RPC every other app speaks, so `yos describe blender` reads a
scene the way `yos describe notes` reads a note — and the same ceiling, the same revision
guard and the same refusal vocabulary bind it, because it is built on the surface SDK
(`yantrik_surface`), the port of `yantrik-app-runtime::control`'s dispatch that any author
uses, rather than an opinion of its own.

Where the SDK comes from. It travels with the addon: a release copies it beside this package
(`share/blender/yantrik_surface`, next to `share/blender/yantrik_blender`), and in the source
tree it is `sdk/python`. Both are found relative to this file, so it works in whatever Python
this Blender runs — Debian's Blender uses the system Python and would see an installed copy,
but a blender.org build brings its own Python that sees no system packages at all — and the
addon always runs with the SDK it was released with.

Threading. The socket is served on a thread of its own; `bpy` is not thread-safe and every
read or change of the scene is marshalled onto Blender's main thread and waited for — the
same hop the Rust apps make with `slint::invoke_from_event_loop`. How the main thread picks
the work up differs by mode, because the two modes have different main threads:

  * windowed: a `bpy.app.timers` callback pumps the queue, so the hop lands between
    Blender's own event-loop turns;
  * background (`blender -b`): there is no event loop to time, so `start()` becomes the
    main loop and pumps the queue itself until the process is stopped.

Tests. Nothing below this file imports `bpy` at import time — `scene.py` is handed the
module — so `tests/blender-core` drives the whole dispatch layer against a fake. This file
is the only place that touches the real thing.
"""

import os
import sys
import threading
import time

APP_ID = "blender"

_here = os.path.dirname(os.path.abspath(__file__))
if _here not in sys.path:
    sys.path.insert(0, _here)


def _find_sdk():
    """Put the SDK this addon travels with first on the path: beside it in a release, at
    `sdk/python` in the source tree. A copy found some other way is used only when neither
    is there."""
    parent = os.path.dirname(_here)
    for candidate in (parent, os.path.join(parent, "..", "..", "..", "sdk", "python")):
        candidate = os.path.normpath(candidate)
        if os.path.isfile(os.path.join(candidate, "yantrik_surface", "__init__.py")):
            if candidate not in sys.path:
                sys.path.insert(0, candidate)
            return candidate
    return None


_find_sdk()

from .bridge import QueuedBridge  # noqa: E402
from .scene import Scene  # noqa: E402
from .surface import Surface  # noqa: E402

# Blender's addon registry reads this. The addon also runs without being installed — the
# launcher starts `blender --python .../bootstrap.py`, which calls start() directly — so
# nothing below depends on registration having happened.
bl_info = {
    "name": "Yantrik Surface",
    "author": "Yantrik OS",
    "version": (1, 0, 0),
    "blender": (4, 0, 0),
    "location": "Socket: $XDG_RUNTIME_DIR/yantrik/app-blender.sock",
    "description": "Publishes this Blender on the Yantrik control surface (app.describe / app.act)",
    "category": "System",
}

_lock = threading.Lock()
_running = None  # the _Session that is serving, or None


def quiet_first_window(bpy_mod, background):
    """Keep Blender's splash off the window this surface is about to drive. Returns True
    when the preference was turned off, False when there was nothing to do.

    Opened from the launcher, a stock Blender puts its splash over the viewport; on a
    machine whose desktop user has never saved preferences (no `userpref.blend` — every
    fresh Yantrik install) that splash is the first-run Quick Setup, and none of the
    surface's actions can dismiss it, so the OS goes on editing the scene behind a dialog
    the person has not answered (#119). Blender 4.3 checks `show_splash` first of all
    (`USER_SPLASH_DISABLE` in `wm_init_splash_show_on_startup_check`), and runs `--python`
    scripts before that check (`ARG_PASS_FINAL` precedes `WM_init_splash_on_startup` in
    creator.cc), so turning the preference off here is early enough and is the whole fix.

    Nothing is written to disk by this function. Blender marks the preference dirty and,
    with `use_preferences_save` on (its default), writes `userpref.blend` on a clean quit —
    which is also what marks first-run done for the person's own later starts. That is
    Blender's own behaviour for any preference change; the addon does not save preferences
    itself, because overwriting a person's preference file is not a thing a socket should
    decide.

    Background Blender has no window, so there is no splash and nothing to touch.
    """
    if background:
        return False
    preferences = getattr(getattr(bpy_mod, "context", None), "preferences", None)
    view = getattr(preferences, "view", None)
    if view is None or not hasattr(view, "show_splash"):
        return False
    try:
        view.show_splash = False
    except (AttributeError, TypeError):
        # A read-only or missing preference is a Blender we do not know; the surface still
        # serves, and a person can close the splash by hand as they always could.
        return False
    return True


class _Session:
    """One serving Blender: the bridge, the surface, the socket, and how to stop all three."""

    def __init__(self, bpy_mod, background):
        self.bpy = bpy_mod
        self.background = background
        self.bridge = QueuedBridge()
        self.surface = Surface(Scene(bpy_mod, self.bridge), self.bridge, app_id=APP_ID)
        self.server = None  # the wire.Server, once start() has bound app-blender.sock
        self.stopped = False
        self.splash_quieted = False

    def start(self):
        # Before the socket, so a bind failure (which keeps the window) still leaves it a
        # window without a splash over it; before the pump, so no action can land behind one.
        self.splash_quieted = quiet_first_window(self.bpy, self.background)
        self.server = self.surface.serve_in_thread()
        if self.background:
            return
        # Windowed: pump between Blender's own turns. The interval is a floor on how long a
        # `yos act` waits for the main thread to notice it, not a poll of anything; 50 ms is
        # below the resolution of a person watching and of a caller timing out.
        def pump_timer():
            if self.stopped:
                return None
            self.bridge.pump()
            return 0.05

        self.bpy.app.timers.register(pump_timer, first_interval=0.05, persistent=True)

    def run_foreground_loop(self):
        """The main loop of a background Blender. Returns when stop() is called."""
        while not self.stopped:
            if not self.bridge.pump():
                time.sleep(0.02)

    def stop(self):
        self.stopped = True
        self.surface.stop()
        self.bridge.wake()


def start():
    """Bind the socket and start answering. Blocks while Blender is in background mode.

    Called from the main thread — by `bootstrap.py`, or by Blender itself through
    `register()`. Failing to serve is not fatal and must not be: a Blender whose socket
    cannot be bound is still a working Blender, and taking the window down over that would
    be the worse outcome (the same choice `App::serve` makes in the Rust runtime).
    """
    global _running
    import bpy  # imported here, not at module level, so the tests never need it

    background = bool(getattr(bpy.app, "background", False))
    with _lock:
        if _running is not None:
            session = _running
        else:
            session = _Session(bpy, background)
            try:
                session.start()
            except OSError as e:
                # Said where a person will see it, and not raised: see the docstring. A second
                # Blender lands here too — the first one's live socket is its name, and the
                # SDK does not bind over it (`SocketBusy`).
                print("[yantrik] control surface not serving: %s" % e, file=sys.stderr)
                return None
            _running = session
    socket_path = session.server.path
    print("[yantrik] control surface on %s (%d actions, %s%s)"
          % (socket_path, len(session.surface.actions),
             "background" if background else "windowed",
             ", splash off" if session.splash_quieted else ""),
          file=sys.stderr)
    if background:
        try:
            session.run_foreground_loop()
        except KeyboardInterrupt:
            stop()
    return session


def stop():
    """Stop serving and unlink the socket. Safe to call when nothing is running."""
    global _running
    with _lock:
        session = _running
        _running = None
    if session is not None:
        session.stop()


def register():
    """Blender's addon hook: the surface comes up when the addon is enabled."""
    start()


def unregister():
    """Blender's addon hook: the surface goes away when the addon is disabled."""
    stop()
