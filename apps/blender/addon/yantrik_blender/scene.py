"""The scene: what it looks like, and what the actions do to it.

Everything here runs on Blender's main thread — the surface only calls into this file
inside a bridge turn. `bpy` is never imported at module level; the module is handed the
real one at construction (`Scene(bpy)`), which is also how the tests hand it a fake.

Two disciplines hold throughout:

  * **Validate, then touch.** Every argument is parsed and every precondition is checked
    before anything in the scene is changed, so a refusal mid-action cannot leave the scene
    half-mutated with the refusal explaining only the half that failed.
  * **Say what is true.** A refusal is a `Refusal` with a sentence in this app's own words:
    what was asked, what is wrong with it, and — where it matters — what cannot be taken
    back. Guessing that an operator succeeded is the failure mode this file exists to not
    have: renders are checked against the filesystem, deletes against the object list,
    imports against the count of objects that appeared.

The camera math (look-at → quaternion) is hand-rolled rather than imported from
`mathutils`, for the same testability reason as the rest: the functions are pure, and
`tests/blender-core` pins them against a known-good vector without Blender installed.
"""

import contextlib
import io
import math
import os
import time

MAX_OBJECTS_IN_STATE = 50

PRIMITIVES = {
    "cube": "primitive_cube_add",
    "sphere": "primitive_uv_sphere_add",
    "cylinder": "primitive_cylinder_add",
    "plane": "primitive_plane_add",
    "monkey": "primitive_monkey_add",
}

LIGHTS = {"point": "POINT", "sun": "SUN", "spot": "SPOT", "area": "AREA"}

# The engine ids have moved under us once already (EEVEE became EEVEE_NEXT in 4.2 and took
# the old name back later), so "eevee" is a list of candidates tried in order rather than a
# string, and the state reports whichever one this Blender actually has.
ENGINES = {
    "cycles": ("CYCLES",),
    "eevee": ("BLENDER_EEVEE", "BLENDER_EEVEE_NEXT"),
    "workbench": ("BLENDER_WORKBENCH",),
}
ENGINE_KEY = {
    "CYCLES": "cycles",
    "BLENDER_EEVEE": "eevee",
    "BLENDER_EEVEE_NEXT": "eevee",
    "BLENDER_WORKBENCH": "workbench",
}
ENGINE_DISPLAY = {"cycles": "Cycles", "eevee": "EEVEE", "workbench": "Workbench"}

# What a Workbench render colours objects by, per `scene.display.shading.color_type` (the
# render's own shading settings; the viewport has a separate copy). Only MATERIAL reads the
# material — and then its viewport-display colour, never its node tree. Every other choice
# ignores materials entirely, so a caller's `set_material` shows nothing under it.
WORKBENCH_COLOR_SOURCES = {
    "SINGLE": "one colour for every object",
    "OBJECT": "each object's own colour",
    "RANDOM": "a random colour per object",
    "VERTEX": "vertex colours",
    "TEXTURE": "image textures",
}

MATERIAL_NOTE_ENGINE_HINT = "`set_render engine=eevee` (or cycles) shades materials properly"


def material_note(engine, color_type):
    """What this render engine does with a material — the sentence a `set_material` or
    `render` answer carries, or None when there is nothing to add.

    Cycles and EEVEE shade the material's nodes: nothing to say. Workbench draws flat
    colour under its own studio light and never reads a node tree. With its colour type at
    MATERIAL (Blender's default) it draws the material's viewport-display colour, which
    `set_material` sets alongside the shader, so the colour asked for is the colour drawn —
    but drawn flat, which a caller expecting a shaded metal should hear. Under any other
    colour type the material shows nowhere at all, and that is the moment to say so: six
    accepted `set_material` calls and a uniformly grey render was how this surfaced
    (#120), on a surface that already knew Workbench "draws flat colour" but only said it
    when someone tried to set `samples`.
    """
    if engine != "workbench":
        return None
    if color_type == "MATERIAL":
        return ("Workbench draws materials as flat colour under its own studio light: "
                "metallic and roughness only shape the highlight, and the scene's lights do "
                "not reach it; %s" % MATERIAL_NOTE_ENGINE_HINT)
    source = WORKBENCH_COLOR_SOURCES.get(color_type, "`%s`" % color_type)
    return ("Workbench is colouring objects by %s (its colour type is `%s`) and ignores "
            "materials, so the colours set with `set_material` do not show in its renders; "
            "%s, or set Workbench's Color back to Material under Render Properties"
            % (source, color_type, MATERIAL_NOTE_ENGINE_HINT))


# How a Workbench render lights objects, per `scene.display.shading.light`. None of the
# three reads a scene light: STUDIO is the engine's own fixed light rig, MATCAP a baked
# sphere image, FLAT no lighting at all. A caller's `set_light` shows under none of them.
WORKBENCH_LIGHTING = {
    "STUDIO": "under its own studio light",
    "MATCAP": "with a MatCap, a baked image of a lit sphere",
    "FLAT": "flat, with no lighting at all",
}

LIGHT_NOTE_ENGINE_HINT = "`set_render engine=eevee` (or cycles) lights the scene with it"


def light_note(engine, light_mode):
    """What this render engine does with a scene light — the sentence a `set_light` answer
    carries, or None when there is nothing to add.

    Cycles and EEVEE light the scene with its lights: nothing to say. Workbench never does,
    whatever its lighting mode — its studio light is a fixed rig of the engine's own, a
    MatCap is a baked image, and flat is no lighting at all — so a light set on a Workbench
    scene changes nothing in its render. The light is still set, and a later EEVEE or
    Cycles render uses it; but the answer has to say, at the moment the light is set, that
    this engine will not draw it. Found alongside #120 (#128): `set_light` on a Workbench
    scene answered `accepted: True, settled: True`, and the render was the same picture.
    """
    if engine != "workbench":
        return None
    how = WORKBENCH_LIGHTING.get(light_mode, "in its `%s` lighting mode" % light_mode)
    return ("Workbench lights the scene itself, %s, and does not use scene lights, so this "
            "light changes nothing in its renders; %s" % (how, LIGHT_NOTE_ENGINE_HINT))


# The app declining, in a sentence a person can read: the surface SDK's own `Refusal`, so it
# travels over the bridge unchanged and comes out of the dispatch as a -32602 with this message.
from yantrik_surface import Refusal  # noqa: E402,F401


# ── pure parsing: no bpy, no scene, fully testable ──────────────────────────

def _as_float(value):
    if isinstance(value, bool):
        raise ValueError("not a number")
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str):
        return float(value.strip())
    raise ValueError("not a number")


def parse_vec3(value, arg):
    """Three numbers, as `1,2,3` or as a JSON array. The sentence names the argument."""
    if isinstance(value, (list, tuple)):
        parts = list(value)
    elif isinstance(value, str):
        parts = [p for p in value.replace(" ", ",").split(",") if p != ""]
    else:
        parts = None
    if parts is not None and len(parts) == 3:
        try:
            return tuple(_as_float(p) for p in parts)
        except (TypeError, ValueError):
            pass
    raise Refusal("`%s` must be three numbers like `1,2,3`; `%s` is not" % (arg, value))


def parse_scale(value, arg):
    """A scale: three factors, or one number meaning all three axes alike."""
    try:
        single = _as_float(value)
        return (single, single, single)
    except (TypeError, ValueError):
        pass
    return parse_vec3(value, arg)


def parse_color(value, arg):
    """`#rrggbb`, or three 0-1 components. Returns an RGBA tuple for the shader node."""
    if isinstance(value, str):
        text = value.strip()
        if text.startswith("#") and len(text) == 7:
            try:
                r = int(text[1:3], 16) / 255.0
                g = int(text[3:5], 16) / 255.0
                b = int(text[5:7], 16) / 255.0
                return (r, g, b, 1.0)
            except ValueError:
                pass
        try:
            rgb = parse_vec3(text, arg)
            if all(0.0 <= channel <= 1.0 for channel in rgb):
                return rgb + (1.0,)
        except Refusal:
            pass
    raise Refusal(
        "`%s` must be `#rrggbb` or three numbers from 0 to 1 like `0.8,0.2,0.1`; `%s` is not"
        % (arg, value))


def parse_unit_interval(value, arg):
    try:
        number = _as_float(value)
    except (TypeError, ValueError):
        raise Refusal("`%s` must be a number from 0 to 1; `%s` is not" % (arg, value))
    if not (0.0 <= number <= 1.0):
        raise Refusal("`%s` must be from 0 to 1; `%s` is not" % (arg, value))
    return number


def parse_nonnegative(value, arg):
    try:
        number = _as_float(value)
    except (TypeError, ValueError):
        raise Refusal("`%s` must be a number of at least 0; `%s` is not" % (arg, value))
    if number < 0:
        raise Refusal("`%s` must not be negative; `%s` is" % (arg, value))
    return number


def parse_samples(value, arg):
    try:
        number = _as_float(value)
    except (TypeError, ValueError):
        raise Refusal("`%s` must be a whole number of at least 1; `%s` is not" % (arg, value))
    if number != int(number) or number < 1:
        raise Refusal("`%s` must be a whole number of at least 1; `%s` is not" % (arg, value))
    return int(number)


def parse_resolution(value, arg):
    """`1920x1080` — the multiplication sign, the letter x, either case."""
    if isinstance(value, str):
        for separator in ("x", "X", "*"):
            if separator in value:
                left, _, right = value.partition(separator)
                try:
                    width = int(left.strip())
                    height = int(right.strip())
                    if width >= 1 and height >= 1:
                        return width, height
                except ValueError:
                    pass
    raise Refusal("`%s` must be like `1920x1080`; `%s` is not" % (arg, value))


def quaternion_from_axes(x_axis, y_axis, z_axis):
    """The rotation whose local axes are these world directions (columns of the matrix).

    The standard branch-on-trace conversion, written out because `mathutils` is only
    available inside Blender and this has to be testable outside it. Returns (w, x, y, z).
    """
    m = ((x_axis[0], y_axis[0], z_axis[0]),
         (x_axis[1], y_axis[1], z_axis[1]),
         (x_axis[2], y_axis[2], z_axis[2]))
    trace = m[0][0] + m[1][1] + m[2][2]
    if trace > 0:
        s = math.sqrt(trace + 1.0) * 2
        return (0.25 * s, (m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s,
                (m[1][0] - m[0][1]) / s)
    if m[0][0] > m[1][1] and m[0][0] > m[2][2]:
        s = math.sqrt(1.0 + m[0][0] - m[1][1] - m[2][2]) * 2
        return ((m[2][1] - m[1][2]) / s, 0.25 * s, (m[0][1] + m[1][0]) / s,
                (m[0][2] + m[2][0]) / s)
    if m[1][1] > m[2][2]:
        s = math.sqrt(1.0 + m[1][1] - m[0][0] - m[2][2]) * 2
        return ((m[0][2] - m[2][0]) / s, (m[0][1] + m[1][0]) / s, 0.25 * s,
                (m[1][2] + m[2][1]) / s)
    s = math.sqrt(1.0 + m[2][2] - m[0][0] - m[1][1]) * 2
    return ((m[1][0] - m[0][1]) / s, (m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s,
            0.25 * s)


def look_at_quaternion(location, target):
    """Point a camera at something: the quaternion whose -Z faces from here to there.

    A camera looks down its own -Z, so -forward is the local Z axis; world up breaks the
    remaining tie, falling back to +Y when the camera looks straight up or down and the two
    are parallel. Returns None when there is no direction to point in (target == location),
    because a rotation for that is not a choice this function gets to make.
    """
    fx = target[0] - location[0]
    fy = target[1] - location[1]
    fz = target[2] - location[2]
    length = math.sqrt(fx * fx + fy * fy + fz * fz)
    if length < 1e-9:
        return None
    z_axis = (-fx / length, -fy / length, -fz / length)

    def cross(a, b):
        return (a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0])

    x_axis = cross((0.0, 0.0, 1.0), z_axis)
    if x_axis[0] * x_axis[0] + x_axis[1] * x_axis[1] + x_axis[2] * x_axis[2] < 1e-12:
        x_axis = cross((0.0, 1.0, 0.0), z_axis)
    n = math.sqrt(x_axis[0] ** 2 + x_axis[1] ** 2 + x_axis[2] ** 2)
    x_axis = (x_axis[0] / n, x_axis[1] / n, x_axis[2] / n)
    y_axis = cross(z_axis, x_axis)
    return quaternion_from_axes(x_axis, y_axis, z_axis)


# ── the scene ───────────────────────────────────────────────────────────────

def _r3(values):
    """Rounded to 3 decimals: enough precision to place a camera, short enough that the
    canonical JSON never drifts into exponent notation and changes a revision for nothing."""
    return [round(float(v), 3) for v in values]


class Scene:
    """One Blender's scene, as the surface sees it and acts on it."""

    # Actions that change what would be written to disk, so they make the scene dirty; and
    # the three that settle it back to clean (a fresh empty scene, a save, an open). `render`
    # and `screenshot` write an image but change no scene data, so they are in neither set.
    # `run_python` is in the dirty set on purpose: it can mutate anything, and the honest
    # assumption after running arbitrary code is that the scene is no longer clean.
    MUTATES = frozenset({"add_primitive", "delete_object", "transform", "set_material",
                         "set_camera", "set_light", "import_model", "set_render", "run_python"})
    SETTLES = frozenset({"new_scene", "save", "open"})

    def __init__(self, bpy_mod):
        self.bpy = bpy_mod
        self.notice = ""          # the last app-level refusal; cleared on the next success
        self.last_render = None   # {path, seconds, bytes} of the last render this process did
        # The render in flight, if any: {output, started, as_of}. Set on the main thread just
        # before it blocks and cleared when it unblocks, so it can be read from the socket's
        # own thread while that thread is unavailable — see `busy_snapshot`.
        self.rendering = None
        # Our own dirty flag, tracked per action. `bpy.data.is_dirty` is Blender's, but in
        # `blender -b` it is stuck True (verified: True at start, after save, after open, on
        # a second read), so it cannot answer "are there unsaved changes" headless. Where
        # there is a window we still OR in is_dirty, because a person at the keyboard can
        # change the scene behind this surface's back; headless, nothing changes it except
        # the actions below, so this flag is the whole truth.
        self._modified = False
        self._dispatch = {
            "new_scene": self._do_new_scene,
            "add_primitive": self._do_add_primitive,
            "delete_object": self._do_delete_object,
            "transform": self._do_transform,
            "set_material": self._do_set_material,
            "set_camera": self._do_set_camera,
            "set_light": self._do_set_light,
            "import_model": self._do_import_model,
            "set_render": self._do_set_render,
            "render": self._do_render,
            "save": self._do_save,
            "open": self._do_open,
            "run_python": self._do_run_python,
            "screenshot": self._do_screenshot,
        }

    # ── describe ─────────────────────────────────────────────────────────────

    def snapshot(self):
        """(summary, state) — the glance and the truth behind it. Main thread only."""
        scene = self.bpy.context.scene
        render = scene.render
        filepath = self.bpy.data.filepath or ""
        objects = list(scene.objects)
        camera = scene.camera
        engine = self._engine_key()
        state = {
            "scene": scene.name,
            "file": filepath or None,
            "unsaved": self._unsaved(),
            "objects": [
                {
                    "name": o.name,
                    "type": o.type,
                    "location": _r3(getattr(o, "location", (0, 0, 0))),
                    "dimensions": _r3(getattr(o, "dimensions", (0, 0, 0))),
                }
                for o in objects[:MAX_OBJECTS_IN_STATE]
            ],
            "objects_total": len(objects),
            "camera": {"name": camera.name,
                       "location": _r3(camera.location)} if camera is not None else None,
            "render": {
                "engine": engine,
                "resolution": "%dx%d" % (int(render.resolution_x), int(render.resolution_y)),
                "samples": self._samples(engine),
                "output": self.bpy.path.abspath(render.filepath),
            },
            "last_render": self.last_render,
            "notice": self.notice,
            "background": bool(getattr(self.bpy.app, "background", False)),
        }
        return self._summary(state), state

    def _summary(self, state):
        """One line for the card and the logs. `Blender — "x.blend", 3 objects, Cycles
        1920x1080[, unsaved]`, or `unsaved scene` where there is no file — which already
        says it, so the trailing flag is only for a named file with changes on top."""
        where = '"%s"' % os.path.basename(state["file"]) if state["file"] else "unsaved scene"
        total = state["objects_total"]
        noun = "object" if total == 1 else "objects"
        engine = ENGINE_DISPLAY.get(state["render"]["engine"],
                                    state["render"]["engine"].title())
        parts = ['Blender — %s, %d %s, %s %s'
                 % (where, total, noun, engine, state["render"]["resolution"])]
        if state["unsaved"] and state["file"]:
            parts.append("unsaved")
        return ", ".join(parts)

    def busy_snapshot(self):
        """(summary, state) for a describe that could not reach the main thread, or None if
        nothing is in flight. Never touches `bpy`, so the socket's own thread may call it.

        The state is the scene as of the moment the render took the main thread — it cannot be
        more current than that, since reading it now is exactly what is impossible. So the
        line says what is running and for how long, and a caller sees a rendering app rather
        than a hung one.
        """
        job = self.rendering
        if job is None:
            return None
        summary, state = job["as_of"]
        state = dict(state)
        state["rendering"] = {
            "output": job["output"],
            "seconds": int(time.monotonic() - job["started"]),
        }
        return "%s, rendering to %s (%ds)" % (
            summary, os.path.basename(job["output"]), state["rendering"]["seconds"]), state

    def _engine_key(self):
        raw = self.bpy.context.scene.render.engine
        return ENGINE_KEY.get(raw, str(raw).lower())

    def _samples(self, engine):
        scene = self.bpy.context.scene
        try:
            if engine == "cycles":
                return int(scene.cycles.samples)
            if engine == "eevee":
                return int(scene.eevee.taa_render_samples)
        except (AttributeError, TypeError, ValueError):
            return None
        return None  # Workbench has no samples to report

    def _workbench_shading(self, name, default):
        """One of a Workbench render's shading settings (`scene.display.shading`). Read
        defensively: a `bpy` without it is assumed to be at Blender's default rather than
        refusing an action over a setting that only matters to one engine."""
        display = getattr(self.bpy.context.scene, "display", None)
        shading = getattr(display, "shading", None)
        return str(getattr(shading, name, default))

    def _workbench_color_type(self):
        """What a Workbench render colours objects by; MATERIAL is Blender's default."""
        return self._workbench_shading("color_type", "MATERIAL")

    def _workbench_light_mode(self):
        """How a Workbench render lights objects; STUDIO is Blender's default."""
        return self._workbench_shading("light", "STUDIO")

    def _material_note(self):
        return material_note(self._engine_key(), self._workbench_color_type())

    def _light_note(self):
        return light_note(self._engine_key(), self._workbench_light_mode())

    # ── act ──────────────────────────────────────────────────────────────────

    def run(self, action, args):
        """Run one action's work. Raises Refusal with the sentence to show the caller."""
        handler = self._dispatch.get(action)
        if handler is None:  # the surface checks first; this is the belt
            raise Refusal("unknown action `%s`" % action)
        result = handler(args)
        # Updated only after the handler returns, so a Refusal — which raises before this —
        # leaves the flag exactly as it was. A refused action changed nothing, and saying
        # otherwise would be the same lie the rest of the surface refuses to tell.
        if action in self.SETTLES:
            self._modified = False
        elif action in self.MUTATES:
            self._modified = True
        return result

    def _unsaved(self):
        """Are there changes that a save has not captured? Main thread only.

        Our tracked flag is the whole truth headless, where nothing else can touch the scene.
        Where there is a window, a person can edit behind this surface's back, so Blender's
        own `is_dirty` is OR'd in — and it is trustworthy there, unlike in `-b` where it is
        stuck True and would report every saved scene as unsaved.
        """
        if self._modified:
            return True
        if getattr(self.bpy.app, "background", False):
            return False
        return bool(self.bpy.data.is_dirty)

    # ── shared pieces ────────────────────────────────────────────────────────

    def _string(self, args, key):
        value = args.get(key)
        if not isinstance(value, str) or not value.strip():
            raise Refusal("`%s` must be a non-empty string" % key)
        return value.strip()

    def _object(self, name):
        for o in self.bpy.context.scene.objects:
            if o.name == name:
                return o
        raise Refusal("there is no object `%s` in this scene" % name)

    def _window_and_area(self):
        """The first window and, in it, a 3D viewport if there is one."""
        wm = getattr(self.bpy.context, "window_manager", None)
        windows = list(getattr(wm, "windows", None) or [])
        if not windows:
            return None, None
        window = windows[0]
        screen = getattr(window, "screen", None)
        for area in list(getattr(screen, "areas", None) or []):
            if getattr(area, "type", None) == "VIEW_3D":
                return window, area
        return window, None

    @contextlib.contextmanager
    def _op_context(self):
        """Run an operator with a window's context borrowed, where there is a window.

        A background Blender has none, and operators that poll for a window would fail
        there — but the operators used here accept a bare context too, so background calls
        go through unoverridden and windowed ones get the real window (and viewport, when
        one is open) so selection and undo land where a person watching can see them.
        """
        window, area = self._window_and_area()
        temp = getattr(self.bpy.context, "temp_override", None)
        if temp is None or window is None:
            yield
            return
        kwargs = {"window": window}
        if area is not None:
            kwargs["area"] = area
        with temp(**kwargs):
            yield

    def _output_path(self, value, arg, suffix):
        """A writable output path ending in `suffix`, with its directory made. Validated
        before any scene change touches the render settings."""
        if not isinstance(value, str) or not value.strip():
            raise Refusal("`%s` must be a non-empty string" % arg)
        text = value.strip()
        if not text.lower().endswith(suffix):
            raise Refusal("`%s` must end in %s; `%s` does not" % (arg, suffix, value))
        path = os.path.abspath(os.path.expanduser(text))
        directory = os.path.dirname(path)
        if directory:
            try:
                os.makedirs(directory, exist_ok=True)
            except OSError as e:
                raise Refusal("there is no way to make a directory for `%s`: %s" % (path, e))
        return path

    # ── the actions ──────────────────────────────────────────────────────────

    def _do_new_scene(self, args):
        try:
            self.bpy.ops.wm.read_homefile(use_empty=True)
        except RuntimeError as e:
            raise Refusal("Blender would not start a new scene: %s" % e)
        return {"scene": self.bpy.context.scene.name}

    def _do_add_primitive(self, args):
        kind = self._string(args, "kind").lower()
        if kind not in PRIMITIVES:
            raise Refusal("`kind` must be one of %s; `%s` is not one"
                          % (", ".join(sorted(PRIMITIVES)), args.get("kind")))
        location = parse_vec3(args["location"], "location") if "location" in args else None
        scale = parse_scale(args["scale"], "scale") if "scale" in args else None
        new_name = self._string(args, "name") if "name" in args else None

        scene = self.bpy.context.scene
        before = {o.name for o in scene.objects}
        op = getattr(self.bpy.ops.mesh, PRIMITIVES[kind])
        try:
            with self._op_context():
                if location is not None:
                    op(location=location)
                else:
                    op()
        except RuntimeError as e:
            raise Refusal("Blender would not add the %s: %s" % (kind, e))
        added = [o for o in scene.objects if o.name not in before]
        if not added:
            raise Refusal("the %s was not added to the scene, and Blender gave no reason"
                          % kind)
        obj = added[0]
        if scale is not None:
            obj.scale = scale
        if new_name:
            obj.name = new_name
        return {"object": obj.name, "type": obj.type, "location": _r3(obj.location)}

    def _do_delete_object(self, args):
        name = self._string(args, "name")
        obj = self._object(name)
        scene = self.bpy.context.scene
        for other in scene.objects:
            try:
                other.select_set(False)
            except Exception:  # noqa: BLE001 - an unselectable object is not this action's fault
                pass
        obj.select_set(True)
        try:
            scene.view_layer.objects.active = obj
        except Exception:  # noqa: BLE001 - some contexts have no view layer; delete still polls
            pass
        try:
            with self._op_context():
                self.bpy.ops.object.delete()
        except RuntimeError as e:
            raise Refusal("Blender would not delete `%s`: %s" % (name, e))
        if self._object_present(name):
            raise Refusal("`%s` is still in the scene; the delete did not take" % name)
        return {"deleted": name}

    def _object_present(self, name):
        return any(o.name == name for o in self.bpy.context.scene.objects)

    def _do_transform(self, args):
        name = self._string(args, "name")
        changes = [k for k in ("location", "rotation", "scale") if k in args]
        if not changes:
            raise Refusal("`transform` was given nothing to change; "
                          "it takes location, rotation or scale")
        location = parse_vec3(args["location"], "location") if "location" in args else None
        rotation = parse_vec3(args["rotation"], "rotation") if "rotation" in args else None
        scale = parse_vec3(args["scale"], "scale") if "scale" in args else None
        obj = self._object(name)
        if location is not None:
            obj.location = location
        if rotation is not None:
            obj.rotation_euler = tuple(math.radians(v) for v in rotation)
        if scale is not None:
            obj.scale = scale
        return {
            "object": obj.name,
            "location": _r3(obj.location),
            "rotation_degrees": _r3(math.degrees(v) for v in obj.rotation_euler),
            "scale": _r3(obj.scale),
        }

    def _do_set_material(self, args):
        name = self._string(args, "name")
        changes = [k for k in ("color", "metallic", "roughness") if k in args]
        if not changes:
            raise Refusal("`set_material` was given nothing to change; "
                          "it takes color, metallic or roughness")
        color = parse_color(args["color"], "color") if "color" in args else None
        metallic = (parse_unit_interval(args["metallic"], "metallic")
                    if "metallic" in args else None)
        roughness = (parse_unit_interval(args["roughness"], "roughness")
                     if "roughness" in args else None)

        obj = self._object(name)
        materials = getattr(getattr(obj, "data", None), "materials", None)
        slots = list(getattr(obj, "material_slots", None) or [])
        material = slots[0].material if slots and slots[0].material is not None else None
        if material is None:
            if materials is None:
                raise Refusal("`%s` is a %s; it has no material to set"
                              % (name, getattr(obj, "type", "object").lower()))
            material = self.bpy.data.materials.new(name="%s Material" % obj.name)
            materials.append(material)
        material.use_nodes = True

        node_tree = getattr(material, "node_tree", None)
        bsdf = node_tree.nodes.get("Principled BSDF") if node_tree is not None else None
        if (metallic is not None or roughness is not None) and bsdf is None:
            raise Refusal("`%s` has no Principled BSDF node; metallic and roughness "
                          "live on it, and this material has none" % material.name)
        # Two homes for every value, set together. The Principled BSDF is what Cycles and
        # EEVEE shade. The material's viewport-display trio (`diffuse_color`, `metallic`,
        # `roughness`) is what Workbench draws — and what the Solid viewport a person is
        # looking at draws — and neither ever reads the node tree. Setting only the shader
        # was how six accepted `set_material` calls rendered a uniformly grey scene (#120):
        # measured on Blender 4.3.2, a Workbench render of a cube whose BSDF base colour was
        # red came out (0.60, 0.61, 0.61); with the viewport colour red too, (0.67, 0.22, 0.19).
        if color is not None:
            if bsdf is not None:
                bsdf.inputs["Base Color"].default_value = color
            material.diffuse_color = color
        if metallic is not None:
            bsdf.inputs["Metallic"].default_value = metallic
            material.metallic = metallic
        if roughness is not None:
            bsdf.inputs["Roughness"].default_value = roughness
            material.roughness = roughness

        reported = {"object": obj.name, "material": material.name}
        if bsdf is not None:
            reported["color"] = [round(float(v), 3)
                                 for v in bsdf.inputs["Base Color"].default_value]
            reported["metallic"] = round(float(bsdf.inputs["Metallic"].default_value), 3)
            reported["roughness"] = round(float(bsdf.inputs["Roughness"].default_value), 3)
        elif color is not None:
            reported["color"] = [round(float(v), 3) for v in color]
        # Said at the moment it matters: a material set on a Workbench scene is drawn flat,
        # or — under a colour type other than Material — not drawn at all.
        note = self._material_note()
        if note is not None:
            reported["note"] = note
        return reported

    def _do_set_camera(self, args):
        changes = [k for k in ("location", "look_at") if k in args]
        if not changes:
            raise Refusal("`set_camera` was given nothing to change; "
                          "it takes location or look_at")
        location = parse_vec3(args["location"], "location") if "location" in args else None
        target = parse_vec3(args["look_at"], "look_at") if "look_at" in args else None

        scene = self.bpy.context.scene
        camera = scene.camera
        if camera is None:
            data = self.bpy.data.cameras.new(name="Camera")
            camera = self.bpy.data.objects.new(name="Camera", object_data=data)
            scene.collection.objects.link(camera)
            scene.camera = camera
        if location is not None:
            camera.location = location
        if target is not None:
            quat = look_at_quaternion(tuple(float(v) for v in camera.location), target)
            if quat is None:
                raise Refusal("`look_at` is where the camera already is; "
                              "there is no direction to point in")
            camera.rotation_mode = "QUATERNION"
            camera.rotation_quaternion = quat
        return {
            "camera": camera.name,
            "location": _r3(camera.location),
            "look_at": _r3(target) if target is not None else None,
        }

    def _do_set_light(self, args):
        kind = self._string(args, "kind").lower()
        if kind not in LIGHTS:
            raise Refusal("`kind` must be one of %s; `%s` is not one"
                          % (", ".join(sorted(LIGHTS)), args.get("kind")))
        energy = parse_nonnegative(args["energy"], "energy") if "energy" in args else None
        location = parse_vec3(args["location"], "location") if "location" in args else None

        scene = self.bpy.context.scene
        light_obj = None
        for o in scene.objects:
            if o.type == "LIGHT" and getattr(o.data, "type", None) == LIGHTS[kind]:
                light_obj = o
                break
        if light_obj is None:
            name = kind.capitalize()
            data = self.bpy.data.lights.new(name=name, type=LIGHTS[kind])
            light_obj = self.bpy.data.objects.new(name=name, object_data=data)
            scene.collection.objects.link(light_obj)
        if energy is not None:
            light_obj.data.energy = energy
        if location is not None:
            light_obj.location = location
        reported = {
            "light": light_obj.name,
            "kind": kind,
            "energy": round(float(light_obj.data.energy), 3),
            "location": _r3(light_obj.location),
        }
        # Said at the moment it matters: the light is set, and a later EEVEE or Cycles
        # render uses it, but a Workbench render lights the scene itself and will not draw it.
        note = self._light_note()
        if note is not None:
            reported["note"] = note
        return reported

    def _do_import_model(self, args):
        raw = self._string(args, "path")
        path = os.path.abspath(os.path.expanduser(raw))
        extension = os.path.splitext(path)[1].lower()
        if extension not in (".obj", ".stl", ".glb", ".gltf"):
            raise Refusal("`%s` is not a format this can import; it takes .obj, .stl or .glb"
                          % (extension or path))
        if not os.path.isfile(path):
            raise Refusal("there is no file at `%s`" % path)

        scene = self.bpy.context.scene
        before = {o.name for o in scene.objects}
        try:
            with self._op_context():
                if extension == ".obj":
                    if hasattr(self.bpy.ops.wm, "obj_import"):
                        self.bpy.ops.wm.obj_import(filepath=path)
                    else:
                        self.bpy.ops.import_scene.obj(filepath=path)
                elif extension == ".stl":
                    if hasattr(self.bpy.ops.wm, "stl_import"):
                        self.bpy.ops.wm.stl_import(filepath=path)
                    else:
                        self.bpy.ops.import_scene.stl(filepath=path)
                else:
                    self.bpy.ops.import_scene.gltf(filepath=path)
        except RuntimeError as e:
            raise Refusal("Blender could not import `%s`: %s" % (path, e))
        added = [o for o in scene.objects if o.name not in before]
        if not added:
            raise Refusal("the import ran but added no objects; `%s` may be empty, "
                          "or not really a %s file" % (path, extension))
        return {"imported": path, "objects_added": len(added)}

    def _do_set_render(self, args):
        changes = [k for k in ("engine", "resolution", "samples") if k in args]
        if not changes:
            raise Refusal("`set_render` was given nothing to change; "
                          "it takes engine, resolution or samples")
        scene = self.bpy.context.scene

        engine_key = None
        if "engine" in args:
            wanted = self._string(args, "engine").lower()
            if wanted not in ENGINES:
                raise Refusal("`engine` must be one of %s; `%s` is not one"
                              % (", ".join(sorted(ENGINES)), args.get("engine")))
            engine_key = wanted
        resolution = (parse_resolution(args["resolution"], "resolution")
                      if "resolution" in args else None)
        samples = parse_samples(args["samples"], "samples") if "samples" in args else None

        # Samples are validated against the engine the scene will have *after* this action,
        # before anything is changed: Workbench has no samples, and half-applying the rest
        # on the way to that refusal would be a mutation the refusal does not mention.
        target_engine = engine_key or self._engine_key()
        if samples is not None and target_engine == "workbench":
            raise Refusal("Workbench has no samples setting; it draws flat colour, "
                          "and there is nothing to sample")

        if engine_key is not None:
            assigned = False
            for candidate in ENGINES[engine_key]:
                try:
                    scene.render.engine = candidate
                    assigned = True
                    break
                except (TypeError, AttributeError):
                    continue
            if not assigned:
                raise Refusal("this Blender has no %s to switch to" % engine_key)
        if resolution is not None:
            scene.render.resolution_x, scene.render.resolution_y = resolution
        if samples is not None:
            if target_engine == "cycles":
                scene.cycles.samples = samples
            else:
                scene.eevee.taa_render_samples = samples

        final_engine = self._engine_key()
        return {
            "engine": final_engine,
            "resolution": "%dx%d" % (int(scene.render.resolution_x),
                                     int(scene.render.resolution_y)),
            "samples": self._samples(final_engine),
        }

    def _do_render(self, args):
        output = self._output_path(args.get("output"), "output", ".png")
        scene = self.bpy.context.scene
        scene.render.filepath = output
        started = time.monotonic()
        # Published before the render takes this thread and cleared after it gives the thread
        # back, because a render holds the main thread for as long as the scene takes — up to
        # the 30 minutes `render` is published with — and a describe arriving in that window
        # would otherwise be indistinguishable from a dead app. `as_of` is read here, on the
        # main thread, one statement before reading it stops being possible.
        self.rendering = {"output": output, "started": started, "as_of": self.snapshot()}
        try:
            return self._render(output, started)
        finally:
            self.rendering = None

    def _render(self, output, started):
        try:
            outcome = self.bpy.ops.render.render(write_still=True)
        except RuntimeError as e:
            raise Refusal("Blender would not render: %s" % e)
        seconds = round(time.monotonic() - started, 2)
        if outcome is not None and "FINISHED" not in outcome:
            raise Refusal("the render did not finish (engine `%s`); nothing was written. "
                          "EEVEE and Workbench need a GPU this session may not have — "
                          "Cycles on CPU renders anywhere" % self._engine_key())
        if not os.path.isfile(output):
            raise Refusal("the render reported done but wrote no file at `%s`" % output)
        size = os.path.getsize(output)
        if size == 0:
            raise Refusal("the render wrote an empty file at `%s`" % output)
        self.last_render = {"path": output, "seconds": seconds, "bytes": size}
        reported = {"path": output, "seconds": seconds, "bytes": size}
        # The same sentence `set_material` carries, so a caller who set colours before
        # switching to Workbench still hears what this picture did with them.
        note = self._material_note()
        if note is not None:
            reported["note"] = note
        return reported

    def _do_screenshot(self, args):
        output = self._output_path(args.get("output"), "output", ".png")
        # Background first, and on app.background rather than on the window list: a real
        # `blender -b` still carries a phantom window with a viewport in it, and asking
        # that for an OpenGL render gets back an operator error instead of the truth.
        if getattr(self.bpy.app, "background", False):
            raise Refusal("there is no 3D viewport to screenshot — a background Blender "
                          "draws nothing; `render` draws the scene without a viewport")
        window, area = self._window_and_area()
        if window is None or area is None:
            raise Refusal("this Blender has no 3D viewport open to screenshot; "
                          "`render` draws the scene without one")
        scene = self.bpy.context.scene
        scene.render.filepath = output
        started = time.monotonic()
        try:
            temp = self.bpy.context.temp_override
            with temp(window=window, area=area):
                outcome = self.bpy.ops.render.opengl(write_still=True)
        except (AttributeError, RuntimeError) as e:
            raise Refusal("the viewport would not draw to a file: %s" % e)
        seconds = round(time.monotonic() - started, 2)
        if outcome is not None and "FINISHED" not in outcome:
            raise Refusal("the viewport render did not finish; nothing was written")
        if not os.path.isfile(output):
            raise Refusal("the screenshot reported done but wrote no file at `%s`" % output)
        size = os.path.getsize(output)
        if size == 0:
            raise Refusal("the screenshot wrote an empty file at `%s`" % output)
        return {"path": output, "seconds": seconds, "bytes": size}

    def _do_save(self, args):
        raw = self._string(args, "path")
        if not raw.lower().endswith(".blend"):
            raise Refusal("a Blender file ends in `.blend`; `%s` does not" % raw)
        path = os.path.abspath(os.path.expanduser(raw))
        directory = os.path.dirname(path)
        if directory:
            try:
                os.makedirs(directory, exist_ok=True)
            except OSError as e:
                raise Refusal("there is no way to make a directory for `%s`: %s" % (path, e))
        try:
            self.bpy.ops.wm.save_as_mainfile(filepath=path)
        except RuntimeError as e:
            raise Refusal("Blender would not save to `%s`: %s" % (path, e))
        return {"saved": path, "scene": self.bpy.context.scene.name}

    def _do_open(self, args):
        raw = self._string(args, "path")
        if not raw.lower().endswith(".blend"):
            raise Refusal("a Blender file ends in `.blend`; `%s` does not" % raw)
        path = os.path.abspath(os.path.expanduser(raw))
        if not os.path.isfile(path):
            raise Refusal("there is no file at `%s`" % path)
        # Guarded by the honest dirty flag, not `bpy.data.is_dirty` directly: in `-b` that
        # is stuck True, so it would refuse every open — including straight after the save
        # or new_scene this very sentence recommends as the way out. `_unsaved()` is False
        # once the scene is settled, so a saved or freshly-emptied scene opens freely, and
        # a scene with unsaved work still refuses.
        if self._unsaved():
            raise Refusal("the current scene has unsaved changes; `save` it first, "
                          "or `new_scene` to throw it away")
        try:
            self.bpy.ops.wm.open_mainfile(filepath=path)
        except RuntimeError as e:
            raise Refusal("Blender would not open `%s`: %s" % (path, e))
        return {"opened": path, "scene": self.bpy.context.scene.name}

    def _do_run_python(self, args):
        code = args.get("code")
        if not isinstance(code, str) or not code.strip():
            raise Refusal("`code` must be a non-empty string of Python")
        buffer = io.StringIO()
        namespace = {"bpy": self.bpy, "__name__": "__yantrik__"}
        try:
            with contextlib.redirect_stdout(buffer):
                exec(compile(code, "<yantrik run_python>", "exec"), namespace)
        except Exception as e:  # noqa: BLE001 - the code's own faults are the answer
            raise Refusal("%s: %s. Anything it did before failing is still done — "
                          "it is not recoverable." % (type(e).__name__, e))
        printed = buffer.getvalue()
        result = {"ran": True, "printed": printed[:4000]}
        if len(printed) > 4000:
            result["printed_truncated"] = True
        return result
