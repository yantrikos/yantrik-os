"""Blender's vocabulary: what this app offers, what each action costs, and how it reaches the scene.

The dispatch, the gate, the wire and the revision are not here any more. They are the surface
SDK's (`sdk/python/yantrik_surface`), the same package any author uses to put an app on this
desktop, and that package is the port of `yantrik-app-runtime::control` — same order of checks,
same sentences, same error codes — that this file used to carry a copy of. What stays here is
what only Blender has:

  * the action table: names, purposes, grades, arguments, and how long Blender's main thread
    may take over each (a render is the outlier by design);
  * the hop onto the main thread — every read and change of the scene crosses the bridge, and
    the guard, the handler and the post-snapshot cross as ONE job, so no other caller can move
    the scene between "is this revision current" and "here is what your action did";
  * the notice: a refusal is said twice, once to the caller and once in the state, and the
    next success clears it.

One thing changes for a caller: the SDK checks each argument against the type it is published
with, as the Rust `yantrik-surface` crate does. `samples` takes the integer 4, `metallic` the
number 0.2 and `location` the text `1,2,3` — which is what `yos act` sends for `samples=4`,
`metallic=0.2` and `location=1,2,3` — and the text "4" for `samples` is refused naming the
argument, where the scene used to parse it.
"""

from yantrik_surface import Action, NotAnswered, Param
from yantrik_surface import Surface as _Surface

from .bridge import BridgeTimeout
from .scene import Refusal

# How long the main thread gets per kind of turn. A render is the outlier by design: the
# honest timeout for "draw this scene" is "however long the scene takes", and 30 minutes is
# where guessing stops being useful. Everything else fails loudly rather than hanging a
# caller.
DESCRIBE_TIMEOUT = 10.0
DEFAULT_TIMEOUT = 30.0


def _action(name, purpose, permission, params, timeout=DEFAULT_TIMEOUT, open_ended=False):
    return Action(name, purpose, permission, params, timeout=timeout, open_ended=open_ended)


ACTIONS = [
    _action("new_scene",
            "Throw the current scene away and start an empty one. Anything unsaved in it is "
            "lost, and in a background Blender there is no undo to argue with.",
            "standard", [], timeout=60.0),
    _action("add_primitive",
            "Add a mesh primitive to the scene, where you say, at the size you say.",
            "standard", [
                Param("kind", description="cube, sphere, cylinder, plane or monkey"),
                Param("name", description="what to call it; Blender names it otherwise",
                      optional=True),
                Param("location", description="centre point, three metres like `1,2,3`",
                      optional=True),
                Param("scale", description="three factors like `1,1,1`, or one for all axes",
                      optional=True),
            ]),
    _action("delete_object",
            "Delete an object from the scene. Blender's own undo can bring it back in a "
            "window; past that undo it is not recoverable, and a background Blender has no "
            "undo at all.",
            "standard", [
                Param("name", description="the object's exact name, as describe lists it"),
            ]),
    _action("transform",
            "Move, rotate or scale an object. Give at least one of the three; rotation is "
            "in degrees.",
            "standard", [
                Param("name", description="the object's exact name, as describe lists it"),
                Param("location", description="three metres like `1,2,3`", optional=True),
                Param("rotation", description="three degrees like `0,0,90`", optional=True),
                Param("scale", description="three factors like `2,2,2`", optional=True),
            ]),
    _action("set_material",
            "Give an object a material: a base colour, how metallic it is, how rough it is. "
            "Give at least one of the three. Cycles and EEVEE shade it; Workbench draws the "
            "colour flat, and the answer says so.",
            "standard", [
                Param("name", description="the object's exact name, as describe lists it"),
                Param("color", description="`#rrggbb`, or `r,g,b` with each from 0 to 1",
                      optional=True),
                Param("metallic", type="number", description="0 to 1", optional=True),
                Param("roughness", type="number", description="0 to 1", optional=True),
            ]),
    _action("set_camera",
            "Place the scene's camera and point it at something. A scene without a camera "
            "gets one.",
            "standard", [
                Param("location", description="three metres like `4,-4,3`", optional=True),
                Param("look_at", description="the point to aim at, like `0,0,0`",
                      optional=True),
            ]),
    _action("set_light",
            "Add a light of a kind, or change the scene's existing one of that kind. Cycles "
            "and EEVEE light the scene with it; Workbench lights the scene itself, and the "
            "answer says so.",
            "standard", [
                Param("kind", description="point, sun, spot or area"),
                Param("energy", type="number", description="brightness in the engine's own "
                      "unit; must not be negative", optional=True),
                Param("location", description="three metres like `2,2,4`", optional=True),
            ]),
    _action("import_model",
            "Import a model file into the scene. It takes .obj, .stl and .glb.",
            "standard", [
                Param("path", description="the file to import"),
            ], timeout=300.0),
    _action("set_render",
            "Change how the scene will be rendered: the engine, the size of the image, the "
            "samples per pixel. Give at least one of the three.",
            "standard", [
                Param("engine", description="cycles, eevee or workbench", optional=True),
                Param("resolution", description="like `1920x1080`", optional=True),
                Param("samples", type="integer", description="per pixel; Cycles and EEVEE "
                      "have samples, Workbench does not", optional=True),
            ]),
    _action("render",
            "Render the scene to a PNG and report the path, the seconds it took and the "
            "size of the file. With Cycles on a big scene this can take minutes.",
            "sensitive", [
                Param("output", description="where to write the PNG"),
            ], timeout=1800.0),
    _action("save",
            "Save this Blender file to a path. It overwrites whatever is already there.",
            "sensitive", [
                Param("path", description="where to write the .blend"),
            ], timeout=120.0),
    _action("open",
            "Open a .blend file in place of the current scene. Refused while the current "
            "scene has unsaved changes — save it or start a new one first.",
            "sensitive", [
                Param("path", description="the .blend file to open"),
            ], timeout=120.0),
    _action("run_python",
            "Run arbitrary Python inside this Blender, with `bpy` in scope. It can do "
            "anything a person at the keyboard can do, including deleting files this user "
            "can reach. Anything it does before an error is still done — it is not "
            "recoverable.",
            "dangerous", [
                Param("code", description="the Python to run"),
            ], timeout=300.0,
            # It runs whatever it is given. It also says it is not recoverable, and that wins at
            # the gate: it asks every time, and no session rule covers it.
            open_ended=True),
    _action("screenshot",
            "Save what the 3D viewport shows as a PNG. Needs a window with a viewport "
            "open; a background Blender draws nothing, and `render` is the honest answer "
            "there.",
            "standard", [
                Param("output", description="where to write the PNG"),
            ], timeout=60.0),
]



class Surface(_Surface):
    """Blender's surface: the SDK's dispatch over `scene.Scene`, across the main-thread bridge.

    One Surface per serving Blender. `settings_path`, `mode_path` and `spend_grant` exist for
    the tests, which pin the ceiling and mode files and stand in for the shell's grant store.
    """

    describe_timeout = DESCRIBE_TIMEOUT
    act_timeout = DEFAULT_TIMEOUT

    def __init__(self, scene, bridge, app_id="blender", settings_path=None, mode_path=None,
                 spend_grant=None):
        super().__init__(app_id, settings_path=settings_path, mode_path=mode_path,
                         spend_grant=spend_grant)
        self.scene = scene
        self.bridge = bridge
        for spec in ACTIONS:
            self.add_action(spec, self._runner(spec.name))

    def snapshot(self):
        return self.scene.snapshot()

    def run_on_app_thread(self, fn, timeout):
        """Blender's main thread, reached through the bridge; a thread that does not turn up
        in time is the app not answering."""
        try:
            return self.bridge.submit(fn, timeout=timeout)
        except BridgeTimeout:
            raise NotAnswered() from None

    def _runner(self, name):
        def run(args):
            try:
                result = self.scene.run(name, args)
            except Refusal as refusal:
                self.scene.notice = str(refusal)
                raise
            self.scene.notice = ""
            return result
        return run
