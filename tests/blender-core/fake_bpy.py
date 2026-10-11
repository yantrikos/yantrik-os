"""A bpy that is not bpy: enough of Blender's Python API for the addon's tests to run anywhere.

The addon's rule — nothing but `__init__.py` imports `bpy` — exists for exactly this file.
`Scene` is handed a module and cannot tell the difference, so the whole dispatch layer, every
action's validation and every refusal sentence is testable on a machine with no Blender on it.

What the fake models is what the addon *observes*, and no more:

  * objects, materials with a Principled BSDF, lights and cameras, enough for the actions to
    have something to change and for `snapshot()` to have something to report;
  * the operators the addon calls, with their real names — and like the real thing, they
    mutate what they claim to and mark the file dirty;
  * `render.render` writing a real (if tiny) PNG to the configured path, so the addon's
    "did a file actually appear" check is exercised against a filesystem rather than mocked;
  * an engine enum that raises TypeError on unknown ids, like a real RNA enum property, so
    the EEVEE-fallback path is a real code path and not a decoration.

What it does not model is anything the addon does not touch: no depsgraph, no undo stack, no
GPU. A test that passes here says the addon's logic is right, not that Blender agrees; that
second half is what the headless verification in the design doc is for.
"""

import os


class FakeVector(list):
    """A list that also behaves like bpy's Vector for the addon's purposes (indexing, iteration,
    tuple assignment). `float(v)` on its elements is all scene.py ever does."""


def _vec(values):
    return FakeVector(float(v) for v in values)


class FakeInput:
    def __init__(self, default_value):
        self.default_value = default_value


class FakeBsdfNode:
    def __init__(self):
        self.inputs = {
            "Base Color": FakeInput((1.0, 1.0, 1.0, 1.0)),
            "Metallic": FakeInput(0.0),
            "Roughness": FakeInput(0.5),
        }


class FakeNodeTree:
    def __init__(self, with_bsdf=True):
        self._bsdf = FakeBsdfNode() if with_bsdf else None

    @property
    def nodes(self):
        tree = self

        class _Nodes:
            def get(self, name):
                return tree._bsdf if name == "Principled BSDF" else None

        return _Nodes()


class FakeMaterial:
    def __init__(self, name, with_bsdf=True):
        self.name = name
        # The viewport-display trio, at Blender's own defaults. Workbench and the Solid
        # viewport draw these and never read the node tree, which is why they are modelled.
        self.diffuse_color = (0.8, 0.8, 0.8, 1.0)
        self.metallic = 0.0
        self.roughness = 0.4
        self._use_nodes = False
        self._with_bsdf = with_bsdf
        self.node_tree = None

    @property
    def use_nodes(self):
        return self._use_nodes

    @use_nodes.setter
    def use_nodes(self, value):
        # Like the real property: switching it on is what builds the tree.
        self._use_nodes = bool(value)
        if self._use_nodes and self.node_tree is None:
            self.node_tree = FakeNodeTree(self._with_bsdf)


class FakeMaterialSlot:
    def __init__(self, material):
        self.material = material


class FakeMesh:
    object_type = "MESH"

    def __init__(self):
        self.materials = []


class FakeCameraData:
    object_type = "CAMERA"


class FakeLightData:
    object_type = "LIGHT"

    def __init__(self, name, type):
        self.name = name
        self.type = type
        self.energy = 1000.0


class FakeObject:
    def __init__(self, name, object_data=None, type="MESH"):
        self.name = name
        self.data = object_data
        self.type = type if object_data is None else object_data.object_type
        self.location = _vec((0.0, 0.0, 0.0))
        self.scale = _vec((1.0, 1.0, 1.0))
        self.rotation_euler = _vec((0.0, 0.0, 0.0))
        self.rotation_mode = "XYZ"
        self.rotation_quaternion = (1.0, 0.0, 0.0, 0.0)
        self.dimensions = _vec((2.0, 2.0, 2.0))
        self._selected = False

    @property
    def material_slots(self):
        # Real slots mirror data.materials; so do these, by being computed rather than stored.
        materials = getattr(self.data, "materials", None)
        return [FakeMaterialSlot(m) for m in materials] if materials is not None else []

    def select_set(self, state):
        self._selected = bool(state)


class FakeObjectsCollection:
    def __init__(self, owner):
        self._owner = owner

    def link(self, obj):
        self._owner.objects.append(obj)

    def __iter__(self):
        return iter(self._owner.objects)


class FakeCollection:
    def __init__(self, scene):
        self.objects = FakeObjectsCollection(scene)


class FakeActiveObjects:
    def __init__(self):
        self.active = None


class FakeViewLayer:
    def __init__(self):
        self.objects = FakeActiveObjects()


class FakeRenderSettings:
    """The enum behaves like an RNA enum: an id this Blender does not have raises TypeError."""

    def __init__(self, engines):
        self._engines = set(engines)
        self._engine = "CYCLES" if "CYCLES" in self._engines else sorted(self._engines)[0]
        self.filepath = "/tmp/"
        self.resolution_x = 1920
        self.resolution_y = 1080

    @property
    def engine(self):
        return self._engine

    @engine.setter
    def engine(self, value):
        if value not in self._engines:
            raise TypeError("bpy_struct: item.attr = val: enum \"%s\" not found in '%s'"
                            % (value, "RenderSettings.engine"))
        self._engine = value


class FakeCycles:
    def __init__(self):
        self.samples = 128


class FakeEevee:
    def __init__(self):
        self.taa_render_samples = 64


class FakeShading:
    """`scene.display.shading`: the shading a Workbench *render* uses (the viewport has its
    own). `color_type` is what Workbench colours objects by; MATERIAL is Blender's default."""

    def __init__(self):
        self.color_type = "MATERIAL"
        self.light = "STUDIO"


class FakeDisplay:
    def __init__(self):
        self.shading = FakeShading()


class FakeScene:
    def __init__(self, name="Scene", engines=("CYCLES", "BLENDER_EEVEE", "BLENDER_WORKBENCH")):
        self.name = name
        self.objects = []
        self.camera = None
        self.render = FakeRenderSettings(engines)
        self.display = FakeDisplay()
        self.cycles = FakeCycles()
        self.eevee = FakeEevee()
        self.collection = FakeCollection(self)
        self.view_layer = FakeViewLayer()


class FakeData:
    def __init__(self, scene):
        self.filepath = ""
        self.is_dirty = False
        self._scene = scene
        self.materials = _Materials(self)
        self.cameras = _Cameras()
        self.lights = _Lights()
        self.objects = _ObjectsFactory()

    @property
    def objects_list(self):
        return self._scene.objects


class _Materials:
    def __init__(self, data):
        self._data = data
        self._without_bsdf = False

    def new(self, name):
        return FakeMaterial(name, with_bsdf=not self._without_bsdf)


class _Cameras:
    def new(self, name):
        data = FakeCameraData()
        data.name = name
        return data


class _Lights:
    def new(self, name, type):
        return FakeLightData(name, type)


class _ObjectsFactory:
    def new(self, name, object_data=None):
        return FakeObject(name, object_data)


# ── operators ───────────────────────────────────────────────────────────────

PNG_BYTES = (b"\x89PNG\r\n\x1a\n" + b"\x00" * 4096)  # not a decodable image; a real file


class FakeOps:
    """The operators the addon calls. Each mutates the fake scene the way its real
    counterpart would, marks the file dirty, and returns {'FINISHED'} unless a test says
    otherwise."""

    def __init__(self, fake):
        self._fake = fake
        self.wm = _WmOps(fake)
        self.mesh = _MeshOps(fake)
        self.object = _ObjectOps(fake)
        self.render = _RenderOps(fake)
        self.import_scene = _LegacyImportOps(fake)


class _OpGroup:
    def __init__(self, fake):
        self._fake = fake

    def _touch(self):
        self._fake.context.scene  # the scene an operator works on
        self._fake.data.is_dirty = True


class _WmOps(_OpGroup):
    def read_homefile(self, use_empty=False):
        scene = self._fake.context.scene
        scene.objects.clear()
        scene.camera = None
        self._fake.data.filepath = ""
        # Models is_dirty as it behaves where there is a window: a fresh empty scene is clean.
        # A real `blender -b` leaves is_dirty stuck True here, but the addon does not read
        # is_dirty in background mode — it tracks dirtiness itself — so the fake models the
        # value the addon would trust, and the background tests exercise the tracked flag.
        self._fake.data.is_dirty = False
        return {"FINISHED"}

    def save_as_mainfile(self, filepath):
        self._fake.data.filepath = filepath
        self._fake.data.is_dirty = False
        return {"FINISHED"}

    def open_mainfile(self, filepath):
        self._fake.data.filepath = filepath
        self._fake.data.is_dirty = False
        self._fake.context.scene.name = os.path.splitext(os.path.basename(filepath))[0]
        return {"FINISHED"}

    def obj_import(self, filepath):
        return self._fake._import(filepath, "obj")

    def stl_import(self, filepath):
        return self._fake._import(filepath, "stl")


class _MeshOps(_OpGroup):
    def _add(self, kind, location=None):
        scene = self._fake.context.scene
        name = "%s.%03d" % (kind.capitalize(), len(scene.objects) + 1)
        obj = FakeObject(name, FakeMesh())
        if location is not None:
            obj.location = _vec(location)
        scene.objects.append(obj)
        self._touch()
        return obj

    def primitive_cube_add(self, location=None):
        self._add("Cube", location)
        return {"FINISHED"}

    def primitive_uv_sphere_add(self, location=None):
        self._add("Sphere", location)
        return {"FINISHED"}

    def primitive_cylinder_add(self, location=None):
        self._add("Cylinder", location)
        return {"FINISHED"}

    def primitive_plane_add(self, location=None):
        self._add("Plane", location)
        return {"FINISHED"}

    def primitive_monkey_add(self, location=None):
        self._add("Suzanne", location)
        return {"FINISHED"}


class _ObjectOps(_OpGroup):
    def delete(self):
        scene = self._fake.context.scene
        doomed = [o for o in scene.objects if getattr(o, "_selected", False)]
        if not doomed:
            # The real operator's poll fails when nothing is selected; RuntimeError is how
            # bpy says that, and the addon catches exactly this.
            raise RuntimeError("Operator bpy.ops.object.delete.poll() failed, "
                               "context is incorrect")
        for obj in doomed:
            scene.objects.remove(obj)
        self._touch()
        return {"FINISHED"}


class _RenderOps(_OpGroup):
    def render(self, write_still=False):
        # A real render reports progress on the main thread as it draws; the addon's
        # render_progress handler pumps the bridge from inside those callbacks. The fake
        # calls any registered handler a few times, and honours `render_seconds` so a test
        # can observe a render that is still in flight.
        import time as _time
        seconds = getattr(self._fake, "render_seconds", 0.0)
        handlers = getattr(self._fake.app.handlers, "render_progress", [])
        for step in range(3):
            for handler in list(handlers):
                handler(self._fake.context.scene)
            if seconds:
                _time.sleep(seconds / 3.0)
        outcome = getattr(self._fake, "render_outcome", {"FINISHED"})
        if "FINISHED" in outcome and write_still:
            path = self._fake.context.scene.render.filepath
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as f:
                f.write(PNG_BYTES)
        return set(outcome)

    def opengl(self, write_still=False):
        return self.render(write_still=write_still)


class _LegacyImportOps(_OpGroup):
    def gltf(self, filepath):
        return self._fake._import(filepath, "gltf")

    def obj(self, filepath):
        return self._fake._import(filepath, "obj")

    def stl(self, filepath):
        return self._fake._import(filepath, "stl")


# ── context, windows, timers ────────────────────────────────────────────────

class FakeArea:
    def __init__(self, type):
        self.type = type


class FakeScreen:
    def __init__(self, areas):
        self.areas = areas


class FakeWindow:
    def __init__(self, areas):
        self.screen = FakeScreen(areas)


class FakeWindowManager:
    def __init__(self, windows):
        self.windows = windows


class FakeTempOverride:
    """Callable returning a context manager, like bpy.context.temp_override."""

    def __call__(self, **kwargs):
        self.kwargs = kwargs
        return self

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class FakePreferencesView:
    def __init__(self):
        self.show_splash = True  # Blender's default; a fresh install shows the splash


class FakePreferences:
    def __init__(self):
        self.view = FakePreferencesView()


class FakeContext:
    def __init__(self, scene, windows):
        self.scene = scene
        self.window_manager = FakeWindowManager(windows)
        self.temp_override = FakeTempOverride()
        self.preferences = FakePreferences()


class FakeApp:
    def __init__(self, background):
        self.background = background
        self.timers = FakeTimers()
        self.handlers = FakeHandlers()


class FakeHandlers:
    """`bpy.app.handlers`: the render_progress list the addon registers into."""

    def __init__(self):
        self.render_progress = []


class FakeTimers:
    def __init__(self):
        self.registered = []

    def register(*args, **kwargs):  # noqa: N805 - mirrors bpy's (fn, first_interval, persistent)
        self = args[0]
        self.registered.append((args[1:], kwargs))


class FakePath:
    @staticmethod
    def abspath(path):
        return os.path.abspath(path)


class FakeBpy:
    """The module stand-in. `make_bpy()` builds one; tests poke at it directly."""

    def __init__(self, background=True, windows=0,
                 engines=("CYCLES", "BLENDER_EEVEE", "BLENDER_WORKBENCH"),
                 scene_name="Scene"):
        scene = FakeScene(name=scene_name, engines=engines)
        self.app = FakeApp(background)
        self.context = FakeContext(scene, [FakeWindow([FakeArea("VIEW_3D")])
                                           for _ in range(windows)])
        self.data = FakeData(scene)
        self.ops = FakeOps(self)
        self.path = FakePath()
        self.render_outcome = {"FINISHED"}

    def _import(self, filepath, kind):
        scene = self.context.scene
        obj = FakeObject(os.path.basename(filepath), FakeMesh())
        scene.objects.append(obj)
        self.data.is_dirty = True
        return {"FINISHED"}


def make_bpy(**kwargs):
    return FakeBpy(**kwargs)
