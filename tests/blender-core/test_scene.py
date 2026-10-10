"""The scene layer: every action against a fake bpy, happy paths and refusal sentences.

Run with no Blender installed — that is the point. `Scene` is handed `fake_bpy.make_bpy()`
and cannot tell; what these tests pin is the addon's own logic: argument validation, the
sentences a caller reads when it gets something wrong, the discipline that nothing is
touched until every argument has been validated, and the checks that an action's claim is
true (a render produced a file, a delete removed the object, an import added one).
"""

import math
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)),
                 "..", "..", "apps", "blender", "addon")))
sys.path.insert(0, os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)),
                 "..", "..", "sdk", "python")))

import fake_bpy  # noqa: E402
from yantrik_blender.scene import (  # noqa: E402
    Refusal,
    Scene,
    light_note,
    look_at_quaternion,
    material_note,
    parse_color,
    parse_resolution,
    parse_vec3,
)


def make_scene(**kwargs):
    """A Scene over a fresh fake bpy; tests that need the fake keep the handle."""
    fake = fake_bpy.make_bpy(**kwargs)
    return Scene(fake), fake


class TestPureParsers(unittest.TestCase):
    def test_vec3_takes_a_string_or_a_list(self):
        self.assertEqual(parse_vec3("1,2,3", "location"), (1.0, 2.0, 3.0))
        self.assertEqual(parse_vec3([1, 2.5, -3], "location"), (1.0, 2.5, -3.0))
        self.assertEqual(parse_vec3(" 1 , 2 , 3 ", "location"), (1.0, 2.0, 3.0))

    def test_vec3_refusal_names_the_argument_and_shows_what_was_given(self):
        with self.assertRaises(Refusal) as caught:
            parse_vec3("somewhere over there", "location")
        self.assertEqual(str(caught.exception),
                         "`location` must be three numbers like `1,2,3`; "
                         "`somewhere over there` is not")

    def test_color_takes_hex_or_components_and_rejects_out_of_range(self):
        self.assertEqual(parse_color("#ff0000", "color"), (1.0, 0.0, 0.0, 1.0))
        self.assertEqual(parse_color("0.5,0.25,1", "color"), (0.5, 0.25, 1.0, 1.0))
        with self.assertRaises(Refusal):
            parse_color("2,0,0", "color")
        with self.assertRaises(Refusal):
            parse_color("#ff00", "color")

    def test_resolution(self):
        self.assertEqual(parse_resolution("1920x1080", "resolution"), (1920, 1080))
        self.assertEqual(parse_resolution("640X480", "resolution"), (640, 480))
        with self.assertRaises(Refusal) as caught:
            parse_resolution("big", "resolution")
        self.assertEqual(str(caught.exception),
                         "`resolution` must be like `1920x1080`; `big` is not")
        with self.assertRaises(Refusal):
            parse_resolution("0x100", "resolution")

    def test_look_at_the_known_vector(self):
        # A camera at (0,-5,0) looking at the origin looks along +Y: a +90° turn about X,
        # which as a quaternion is (w,x,y,z) ≈ (0.7071, 0.7071, 0, 0). Computed here rather
        # than taken from mathutils, and checked against that known-good value.
        quat = look_at_quaternion((0.0, -5.0, 0.0), (0.0, 0.0, 0.0))
        self.assertAlmostEqual(quat[0], math.sqrt(0.5), places=6)
        self.assertAlmostEqual(quat[1], math.sqrt(0.5), places=6)
        self.assertAlmostEqual(quat[2], 0.0, places=6)
        self.assertAlmostEqual(quat[3], 0.0, places=6)

    def test_look_at_straight_down_does_not_degenerate(self):
        # Forward parallel to world up: the fallback axis keeps the basis orthonormal.
        quat = look_at_quaternion((0.0, 0.0, 5.0), (0.0, 0.0, 0.0))
        norm = math.sqrt(sum(c * c for c in quat))
        self.assertAlmostEqual(norm, 1.0, places=6)

    def test_look_at_at_itself_is_none_not_a_guess(self):
        self.assertIsNone(look_at_quaternion((1.0, 1.0, 1.0), (1.0, 1.0, 1.0)))


class TestSnapshot(unittest.TestCase):
    def test_an_empty_unsaved_scene_reports_itself_honestly(self):
        scene, fake = make_scene()
        summary, state = scene.snapshot()
        self.assertEqual(summary, "Blender — unsaved scene, 0 objects, Cycles 1920x1080")
        self.assertIsNone(state["file"])
        self.assertFalse(state["unsaved"])
        self.assertEqual(state["objects"], [])
        self.assertEqual(state["objects_total"], 0)
        self.assertIsNone(state["camera"])
        self.assertEqual(state["render"]["engine"], "cycles")
        self.assertEqual(state["render"]["samples"], 128)
        self.assertTrue(state["background"])
        self.assertEqual(state["notice"], "")
        self.assertIsNone(state["last_render"])
        # The tracked flag, not the raw is_dirty: in background mode the addon reports its
        # own tracking, because Blender's is_dirty is stuck True headless (see TestUnsaved).
        self.assertFalse(scene._modified)

    def test_summary_carries_the_file_the_count_and_the_dirty_flag(self):
        scene, fake = make_scene()
        fake.data.filepath = "/home/pranab/monkey.blend"
        scene.run("add_primitive", {"kind": "monkey"})
        scene.run("add_primitive", {"kind": "cube"})
        summary, state = scene.snapshot()
        self.assertEqual(summary,
                         'Blender — "monkey.blend", 2 objects, Cycles 1920x1080, unsaved')
        self.assertEqual(state["objects_total"], 2)
        self.assertTrue(state["unsaved"])
        self.assertEqual(state["objects"][0]["type"], "MESH")

    def test_one_object_is_not_two_objects(self):
        scene, _ = make_scene()
        scene.run("add_primitive", {"kind": "plane"})
        summary, _ = scene.snapshot()
        self.assertIn("1 object,", summary)

    def test_the_object_list_is_capped_but_the_count_is_not(self):
        scene, _ = make_scene()
        for _ in range(60):
            scene.run("add_primitive", {"kind": "cube"})
        _, state = scene.snapshot()
        self.assertEqual(len(state["objects"]), 50)
        self.assertEqual(state["objects_total"], 60)


class TestAddPrimitive(unittest.TestCase):
    def test_monkey_with_a_name_a_place_and_a_scale(self):
        scene, fake = make_scene()
        result = scene.run("add_primitive", {
            "kind": "monkey", "name": "Suzanne", "location": "1,2,3", "scale": "2"})
        self.assertEqual(result["object"], "Suzanne")
        self.assertEqual(result["type"], "MESH")
        self.assertEqual(result["location"], [1.0, 2.0, 3.0])
        obj = fake.context.scene.objects[0]
        self.assertEqual(list(obj.scale), [2.0, 2.0, 2.0])
        self.assertTrue(scene._modified)  # adding made the scene dirty, by our own tracking
        self.assertTrue(scene.snapshot()[1]["unsaved"])

    def test_kind_is_one_of_five_and_the_refusal_says_so(self):
        scene, _ = make_scene()
        for kind in ("cube", "sphere", "cylinder", "plane", "monkey"):
            scene.run("add_primitive", {"kind": kind})
        with self.assertRaises(Refusal) as caught:
            scene.run("add_primitive", {"kind": "torus"})
        self.assertEqual(str(caught.exception),
                         "`kind` must be one of cube, cylinder, monkey, plane, sphere; "
                         "`torus` is not one")

    def test_a_bad_location_is_refused_before_anything_is_added(self):
        scene, fake = make_scene()
        with self.assertRaises(Refusal):
            scene.run("add_primitive", {"kind": "cube", "location": "here"})
        self.assertEqual(len(fake.context.scene.objects), 0,
                         "a refusal must not leave a half-made scene behind")


class TestDeleteObject(unittest.TestCase):
    def test_deletes_and_checks_the_object_is_actually_gone(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        result = scene.run("delete_object", {"name": "Box"})
        self.assertEqual(result, {"deleted": "Box"})
        self.assertEqual(len(fake.context.scene.objects), 0)

    def test_no_such_object_is_said_in_words(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("delete_object", {"name": "Ghost"})
        self.assertEqual(str(caught.exception),
                         "there is no object `Ghost` in this scene")


class TestTransform(unittest.TestCase):
    def test_moves_rotates_and_scales(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        result = scene.run("transform", {
            "name": "Box", "location": "1,0,-1", "rotation": "0,0,90", "scale": "2,2,2"})
        self.assertEqual(result["location"], [1.0, 0.0, -1.0])
        self.assertAlmostEqual(result["rotation_degrees"][2], 90.0, places=3)
        obj = fake.context.scene.objects[0]
        self.assertAlmostEqual(obj.rotation_euler[2], math.pi / 2, places=6)

    def test_nothing_to_change_is_refused_rather_than_guessed(self):
        scene, _ = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        with self.assertRaises(Refusal) as caught:
            scene.run("transform", {"name": "Box"})
        self.assertEqual(str(caught.exception),
                         "`transform` was given nothing to change; it takes location, "
                         "rotation or scale")


class TestSetMaterial(unittest.TestCase):
    def test_a_new_material_gets_the_colour_the_metal_and_the_roughness(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "monkey", "name": "Suzanne"})
        result = scene.run("set_material", {
            "name": "Suzanne", "color": "#ff0000", "metallic": 0.9, "roughness": 0.1})
        self.assertEqual(result["color"], [1.0, 0.0, 0.0, 1.0])
        self.assertEqual(result["metallic"], 0.9)
        self.assertEqual(result["roughness"], 0.1)
        obj = fake.context.scene.objects[0]
        material = obj.material_slots[0].material
        self.assertTrue(material.use_nodes)

    def test_a_second_call_changes_the_same_material(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        scene.run("set_material", {"name": "Box", "color": "#00ff00"})
        scene.run("set_material", {"name": "Box", "roughness": 0.25})
        obj = fake.context.scene.objects[0]
        self.assertEqual(len(obj.data.materials), 1, "one object, one material, changed twice")

    def test_out_of_range_values_are_refused_before_the_material_exists(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        with self.assertRaises(Refusal) as caught:
            scene.run("set_material", {"name": "Box", "metallic": 4})
        self.assertEqual(str(caught.exception), "`metallic` must be from 0 to 1; `4` is not")
        self.assertEqual(len(fake.context.scene.objects[0].data.materials), 0,
                         "validation happens before the material is made, not after")

    def test_a_camera_has_no_material_and_says_so(self):
        scene, _ = make_scene()
        scene.run("set_camera", {"location": "1,2,3"})
        with self.assertRaises(Refusal) as caught:
            scene.run("set_material", {"name": "Camera", "color": "#ffffff"})
        self.assertEqual(str(caught.exception),
                         "`Camera` is a camera; it has no material to set")

    def test_metallic_without_a_bsdf_node_is_refused_not_dropped(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        fake.data.materials._without_bsdf = True
        with self.assertRaises(Refusal) as caught:
            scene.run("set_material", {"name": "Box", "metallic": 0.5})
        self.assertIn("no Principled BSDF node", str(caught.exception))

    def test_nothing_to_change(self):
        scene, _ = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        with self.assertRaises(Refusal) as caught:
            scene.run("set_material", {"name": "Box"})
        self.assertEqual(str(caught.exception),
                         "`set_material` was given nothing to change; it takes color, "
                         "metallic or roughness")


class TestSetCamera(unittest.TestCase):
    def test_a_scene_without_a_camera_gets_one(self):
        scene, fake = make_scene()
        result = scene.run("set_camera", {"location": "4,-4,3", "look_at": "0,0,0"})
        self.assertEqual(result["camera"], "Camera")
        self.assertEqual(result["look_at"], [0.0, 0.0, 0.0])
        self.assertIs(fake.context.scene.camera, fake.context.scene.objects[0])

    def test_look_at_points_the_camera_the_right_way(self):
        scene, fake = make_scene()
        scene.run("set_camera", {"location": "0,-5,0", "look_at": "0,0,0"})
        camera = fake.context.scene.camera
        self.assertEqual(camera.rotation_mode, "QUATERNION")
        w, x, y, z = camera.rotation_quaternion
        self.assertAlmostEqual(w, math.sqrt(0.5), places=6)
        self.assertAlmostEqual(x, math.sqrt(0.5), places=6)
        self.assertAlmostEqual(y, 0.0, places=6)
        self.assertAlmostEqual(z, 0.0, places=6)

    def test_looking_at_itself_is_refused(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("set_camera", {"location": "1,1,1", "look_at": "1,1,1"})
        self.assertEqual(str(caught.exception),
                         "`look_at` is where the camera already is; there is no direction "
                         "to point in")

    def test_nothing_to_change(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("set_camera", {})
        self.assertEqual(str(caught.exception),
                         "`set_camera` was given nothing to change; it takes location or "
                         "look_at")


class TestSetLight(unittest.TestCase):
    def test_creates_then_reuses_the_light_of_a_kind(self):
        scene, fake = make_scene()
        first = scene.run("set_light", {"kind": "sun", "energy": 3, "location": "0,0,10"})
        self.assertEqual(first["kind"], "sun")
        self.assertEqual(first["energy"], 3.0)
        second = scene.run("set_light", {"kind": "sun", "energy": 5})
        self.assertEqual(second["light"], first["light"], "one sun, changed twice")
        self.assertEqual(len([o for o in fake.context.scene.objects
                              if o.type == "LIGHT"]), 1)

    def test_bad_kind_and_negative_energy_are_refused(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("set_light", {"kind": "candle"})
        self.assertEqual(str(caught.exception),
                         "`kind` must be one of area, point, spot, sun; `candle` is not one")
        with self.assertRaises(Refusal) as caught:
            scene.run("set_light", {"kind": "point", "energy": -5})
        self.assertEqual(str(caught.exception),
                         "`energy` must not be negative; `-5` is")


class TestImportModel(unittest.TestCase):
    def test_a_missing_file_is_refused_by_name(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("import_model", {"path": "/tmp/no-such-model.obj"})
        self.assertEqual(str(caught.exception),
                         "there is no file at `/tmp/no-such-model.obj`")

    def test_a_format_it_does_not_take_is_refused(self):
        scene, _ = make_scene()
        with tempfile.NamedTemporaryFile(suffix=".fbx") as tmp:
            with self.assertRaises(Refusal) as caught:
                scene.run("import_model", {"path": tmp.name})
        self.assertEqual(str(caught.exception),
                         "`.fbx` is not a format this can import; it takes .obj, .stl or .glb")

    def test_an_obj_imports_and_reports_what_appeared(self):
        scene, _ = make_scene()
        with tempfile.NamedTemporaryFile(suffix=".obj") as tmp:
            tmp.write(b"v 0 0 0\n")
            tmp.flush()
            result = scene.run("import_model", {"path": tmp.name})
        self.assertEqual(result["objects_added"], 1)
        self.assertEqual(result["imported"], tmp.name)


class TestSetRender(unittest.TestCase):
    def test_engine_resolution_and_samples_all_land(self):
        scene, fake = make_scene()
        result = scene.run("set_render", {
            "engine": "eevee", "resolution": "640x480", "samples": "16"})
        self.assertEqual(result, {"engine": "eevee", "resolution": "640x480", "samples": 16})
        self.assertEqual(fake.context.scene.eevee.taa_render_samples, 16)

    def test_samples_for_cycles_go_to_cycles(self):
        scene, fake = make_scene()
        scene.run("set_render", {"samples": 32})
        self.assertEqual(fake.context.scene.cycles.samples, 32)

    def test_workbench_has_no_samples_and_says_so_before_changing_anything(self):
        scene, fake = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("set_render", {"engine": "workbench", "samples": 8})
        self.assertEqual(str(caught.exception),
                         "Workbench has no samples setting; it draws flat colour, and "
                         "there is nothing to sample")
        self.assertEqual(fake.context.scene.render.engine, "CYCLES",
                         "a refused set_render changed the engine on the way to refusing")

    def test_samples_alone_on_workbench_are_also_refused(self):
        scene, fake = make_scene()
        scene.run("set_render", {"engine": "workbench"})
        with self.assertRaises(Refusal):
            scene.run("set_render", {"samples": 8})

    def test_an_eevee_next_blender_still_answers_to_eevee(self):
        # 4.2 renamed the engine; the addon tries both ids and reports the one that took.
        scene, fake = make_scene(engines=("CYCLES", "BLENDER_EEVEE_NEXT", "BLENDER_WORKBENCH"))
        result = scene.run("set_render", {"engine": "eevee"})
        self.assertEqual(result["engine"], "eevee")
        self.assertEqual(fake.context.scene.render.engine, "BLENDER_EEVEE_NEXT")

    def test_nothing_to_change(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("set_render", {})
        self.assertEqual(str(caught.exception),
                         "`set_render` was given nothing to change; it takes engine, "
                         "resolution or samples")

    def test_a_bad_engine_names_the_three(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("set_render", {"engine": "octane"})
        self.assertEqual(str(caught.exception),
                         "`engine` must be one of cycles, eevee, workbench; `octane` is "
                         "not one")


class TestRender(unittest.TestCase):
    def test_renders_a_png_and_reports_path_seconds_and_bytes(self):
        scene, _ = make_scene()
        with tempfile.TemporaryDirectory() as tmp:
            out = os.path.join(tmp, "nested", "monkey.png")
            result = scene.run("render", {"output": out})
            self.assertTrue(os.path.isfile(out), "the addon checks this itself; so do we")
            self.assertEqual(os.path.getsize(out), result["bytes"])
        self.assertEqual(result["path"], out)
        self.assertGreater(result["bytes"], 0)
        self.assertGreaterEqual(result["seconds"], 0)
        self.assertEqual(scene.last_render["path"], out)

    def test_a_render_that_writes_nothing_is_not_reported_as_a_render(self):
        scene, fake = make_scene()
        fake.render_outcome = {"CANCELLED"}
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(Refusal) as caught:
                scene.run("render", {"output": os.path.join(tmp, "x.png")})
        self.assertIn("did not finish", str(caught.exception))
        self.assertIsNone(scene.last_render)

    def test_output_must_be_a_png(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("render", {"output": "/tmp/frame.exr"})
        self.assertEqual(str(caught.exception),
                         "`output` must end in .png; `/tmp/frame.exr` does not")

    def test_mid_render_it_says_what_is_running_over_the_scene_as_of_then(self):
        # #95: a render holds the main thread for as long as the scene takes, so a describe
        # arriving mid-render cannot read the scene and used to be reported as a dead app. The
        # moment to look is inside the render op — that window is where such a describe lands.
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        seen = {}
        real = fake.ops.render.render

        def spy(write_still=False):
            seen["mid"] = scene.busy_snapshot()
            return real(write_still=write_still)

        fake.ops.render.render = spy
        with tempfile.TemporaryDirectory() as tmp:
            out = os.path.join(tmp, "x.png")
            as_of = scene.snapshot()
            scene.run("render", {"output": out})

            self.assertIsNone(scene.busy_snapshot(),
                              "the render is over; nothing is in flight to report")
            summary, state = seen["mid"]
            self.assertEqual(summary, "%s, rendering to x.png (0s)" % as_of[0],
                             "the line is the scene as of the moment the render took the "
                             "thread, plus what is running now")
            self.assertEqual(state["rendering"], {"output": out, "seconds": 0})
            self.assertEqual(state["objects_total"], 1,
                             "the state is the as-of one; it cannot be more current than "
                             "the last moment the scene could be read")


class TestScreenshot(unittest.TestCase):
    def test_a_background_blender_says_what_to_do_instead(self):
        # windows=1 on purpose: a real `blender -b` carries a phantom window with a
        # viewport that draws nothing (found in the live headless run), so the refusal
        # must key off app.background, not off the absence of a window.
        scene, _ = make_scene(background=True, windows=1)
        with self.assertRaises(Refusal) as caught:
            scene.run("screenshot", {"output": "/tmp/shot.png"})
        self.assertEqual(str(caught.exception),
                         "there is no 3D viewport to screenshot — a background Blender "
                         "draws nothing; `render` draws the scene without a viewport")

    def test_with_a_viewport_open_it_draws_one(self):
        scene, _ = make_scene(background=False, windows=1)
        with tempfile.TemporaryDirectory() as tmp:
            out = os.path.join(tmp, "shot.png")
            result = scene.run("screenshot", {"output": out})
            self.assertGreater(result["bytes"], 0)
            self.assertTrue(os.path.isfile(out))


class TestSaveOpen(unittest.TestCase):
    def test_save_clears_the_dirty_flag_and_reports_the_path(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube"})
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "scene.blend")
            result = scene.run("save", {"path": path})
            self.assertEqual(result["saved"], path)
            self.assertEqual(fake.data.filepath, path)
            self.assertFalse(scene._modified)  # a save settles the scene
            self.assertFalse(scene.snapshot()[1]["unsaved"])

    def test_save_refuses_a_path_that_is_not_a_blend_file(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("save", {"path": "/tmp/scene.zip"})
        self.assertEqual(str(caught.exception),
                         "a Blender file ends in `.blend`; `/tmp/scene.zip` does not")

    def test_open_refuses_while_the_scene_is_dirty_and_says_the_way_out(self):
        scene, _ = make_scene()
        scene.run("add_primitive", {"kind": "cube"})
        with tempfile.NamedTemporaryFile(suffix=".blend") as tmp:
            with self.assertRaises(Refusal) as caught:
                scene.run("open", {"path": tmp.name})
        self.assertEqual(str(caught.exception),
                         "the current scene has unsaved changes; `save` it first, or "
                         "`new_scene` to throw it away")

    def test_open_after_new_scene_works(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube"})
        scene.run("new_scene", {})
        # new_scene settles the scene, so the guard — which reads the honest tracked flag,
        # not Blender's is_dirty (stuck True headless) — must let the open through. This is
        # the exact refusal the live headless run surfaced: the old guard named new_scene as
        # the way out, then refused the open new_scene had just made possible.
        self.assertFalse(scene._modified)
        with tempfile.NamedTemporaryFile(suffix=".blend") as tmp:
            result = scene.run("open", {"path": tmp.name})
        self.assertEqual(result["opened"], tmp.name)
        self.assertEqual(fake.data.filepath, tmp.name)

    def test_open_after_save_works(self):
        # The other half of the same bug: a saved scene is clean, so opening a file must not
        # be refused as having unsaved changes. The fake's save sets no bytes, so the target
        # is made real here — what is under test is the dirty guard, not the write.
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube"})
        with tempfile.TemporaryDirectory() as tmp:
            first = os.path.join(tmp, "first.blend")
            scene.run("save", {"path": first})
            self.assertFalse(scene._modified)
            with open(first, "wb") as f:
                f.write(b"BLENDER")
            result = scene.run("open", {"path": first})
        self.assertEqual(result["opened"], first)

    def test_open_refuses_a_file_that_is_not_there(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("open", {"path": "/tmp/no-such.blend"})
        self.assertEqual(str(caught.exception), "there is no file at `/tmp/no-such.blend`")


class TestUnsaved(unittest.TestCase):
    """`bpy.data.is_dirty` is stuck True in `blender -b` (verified on a live 4.0.2 run: True
    at start, after save_as, after save_mainfile, after open, on a second read). So headless,
    the addon cannot use it — it tracks dirtiness itself. Where there is a window, is_dirty is
    trustworthy and is OR'd in, because a person can edit the scene behind the surface's back.
    These tests pin both halves, and the background half against a fake is_dirty forced True
    to reproduce exactly what the real headless Blender does."""

    def test_background_ignores_a_stuck_dirty_flag_on_a_clean_scene(self):
        scene, fake = make_scene(background=True)
        fake.data.is_dirty = True  # what real `-b` reports no matter what
        self.assertFalse(scene._modified)
        self.assertFalse(scene.snapshot()[1]["unsaved"],
                         "a headless scene with no tracked change is not unsaved, "
                         "whatever Blender's stuck flag says")

    def test_background_reports_unsaved_only_from_tracked_changes(self):
        scene, fake = make_scene(background=True)
        fake.data.is_dirty = True
        scene.run("add_primitive", {"kind": "cube"})
        self.assertTrue(scene.snapshot()[1]["unsaved"])
        scene.run("save", {"path": "/tmp/x.blend"})
        self.assertFalse(scene.snapshot()[1]["unsaved"],
                         "a save settles it, even though is_dirty is still stuck True")

    def test_windowed_trusts_is_dirty_for_edits_the_surface_did_not_make(self):
        scene, fake = make_scene(background=False, windows=1)
        # Nobody acted through the surface, but a person at the keyboard changed something:
        fake.data.is_dirty = True
        self.assertFalse(scene._modified)
        self.assertTrue(scene.snapshot()[1]["unsaved"],
                        "where there is a window, Blender's own flag is believed")

    def test_a_refused_action_does_not_mark_the_scene_dirty(self):
        scene, fake = make_scene(background=True)
        with self.assertRaises(Refusal):
            scene.run("add_primitive", {"kind": "torus"})  # refused before anything is added
        self.assertFalse(scene._modified,
                         "a refusal changed nothing, so it must not claim unsaved work")

    def test_run_python_is_treated_as_a_change(self):
        scene, fake = make_scene(background=True)
        scene.run("run_python", {"code": "pass"})
        self.assertTrue(scene._modified,
                        "arbitrary code can mutate anything; the honest assumption is dirty")


class TestNewScene(unittest.TestCase):
    def test_throws_the_scene_away(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "monkey"})
        result = scene.run("new_scene", {})
        self.assertEqual(len(fake.context.scene.objects), 0)
        self.assertIsNone(fake.context.scene.camera)
        self.assertIn("scene", result)


class TestRunPython(unittest.TestCase):
    def test_runs_with_bpy_in_scope_and_captures_what_it_prints(self):
        scene, fake = make_scene()
        result = scene.run("run_python", {
            "code": "print(len(bpy.context.scene.objects))\n"
                    "bpy.ops.mesh.primitive_cube_add()\n"
                    "print('added one')"})
        self.assertTrue(result["ran"])
        self.assertIn("added one", result["printed"])
        self.assertEqual(len(fake.context.scene.objects), 1)

    def test_a_failure_midway_says_what_still_happened(self):
        scene, fake = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("run_python", {
                "code": "bpy.ops.mesh.primitive_cube_add()\nraise ValueError('halfway')"})
        sentence = str(caught.exception)
        self.assertTrue(sentence.startswith("ValueError: halfway."), sentence)
        self.assertTrue(sentence.endswith(
            "Anything it did before failing is still done — it is not recoverable."),
            sentence)
        self.assertEqual(len(fake.context.scene.objects), 1,
                         "the cube the failing code made is still there, and the sentence "
                         "says so")

    def test_empty_code_is_refused(self):
        scene, _ = make_scene()
        with self.assertRaises(Refusal) as caught:
            scene.run("run_python", {"code": "   "})
        self.assertEqual(str(caught.exception), "`code` must be a non-empty string of Python")


class TestWhatARenderDoesWithAMaterial(unittest.TestCase):
    """#120: six accepted `set_material` calls, a uniformly grey Workbench render, and the
    sentence that would have explained it said only when someone tried to set `samples`.

    Two halves. The pure decision — engine and Workbench colour type in, what the render
    will show out — and the actions that carry it: `set_material` and `render` answer with
    the note on a Workbench scene and without one on Cycles or EEVEE. And the cause itself:
    Workbench draws a material's viewport-display colour, never its node tree, so the colour
    is now set in both places (measured on Blender 4.3.2: BSDF-only red rendered grey,
    (0.60, 0.61, 0.61); with the viewport colour set too it rendered red, (0.67, 0.22, 0.19)).
    """

    FLAT = ("Workbench draws materials as flat colour under its own studio light: metallic "
            "and roughness only shape the highlight, and the scene's lights do not reach it; "
            "`set_render engine=eevee` (or cycles) shades materials properly")

    def test_cycles_and_eevee_shade_materials_and_there_is_nothing_to_add(self):
        self.assertIsNone(material_note("cycles", "MATERIAL"))
        self.assertIsNone(material_note("eevee", "MATERIAL"))
        # The colour type belongs to Workbench; under another engine it is not consulted.
        self.assertIsNone(material_note("cycles", "OBJECT"))

    def test_workbench_colouring_by_material_draws_it_flat_and_says_so(self):
        self.assertEqual(material_note("workbench", "MATERIAL"), self.FLAT)

    def test_workbench_colouring_by_anything_else_does_not_draw_it_and_says_how_to(self):
        note = material_note("workbench", "OBJECT")
        self.assertEqual(
            note,
            "Workbench is colouring objects by each object's own colour (its colour type is "
            "`OBJECT`) and ignores materials, so the colours set with `set_material` do not "
            "show in its renders; `set_render engine=eevee` (or cycles) shades materials "
            "properly, or set Workbench's Color back to Material under Render Properties")
        for color_type in ("SINGLE", "RANDOM", "VERTEX", "TEXTURE"):
            self.assertIn("ignores materials", material_note("workbench", color_type),
                          color_type)

    def test_a_colour_type_this_addon_has_not_heard_of_is_still_named_not_guessed_at(self):
        note = material_note("workbench", "HOLOGRAM")
        self.assertIn("colouring objects by `HOLOGRAM`", note)
        self.assertIn("ignores materials", note)

    def test_set_material_sets_the_viewport_colour_workbench_actually_draws(self):
        scene, fake = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        scene.run("set_material", {"name": "Box", "color": "#ff0000",
                                   "metallic": 0.9, "roughness": 0.1})
        material = fake.context.scene.objects[0].material_slots[0].material
        self.assertEqual(tuple(material.diffuse_color), (1.0, 0.0, 0.0, 1.0),
                         "the shader was set but the colour Workbench draws was left grey")
        self.assertEqual(material.metallic, 0.9)
        self.assertEqual(material.roughness, 0.1)
        # And the shader still has it: Cycles and EEVEE read the node, not the display trio.
        bsdf = material.node_tree.nodes.get("Principled BSDF")
        self.assertEqual(tuple(bsdf.inputs["Base Color"].default_value), (1.0, 0.0, 0.0, 1.0))

    def test_set_material_on_a_cycles_scene_carries_no_note(self):
        scene, _ = make_scene()
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        result = scene.run("set_material", {"name": "Box", "color": "#ff0000"})
        self.assertNotIn("note", result)

    def test_set_material_on_a_workbench_scene_says_it_is_drawn_flat(self):
        scene, _ = make_scene()
        scene.run("set_render", {"engine": "workbench"})
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        result = scene.run("set_material", {"name": "Box", "color": "#ff0000"})
        self.assertEqual(result["color"], [1.0, 0.0, 0.0, 1.0], "still accepted, still set")
        self.assertEqual(result["note"], self.FLAT)

    def test_set_material_under_a_workbench_colour_type_that_ignores_it_says_so(self):
        scene, fake = make_scene()
        scene.run("set_render", {"engine": "workbench"})
        fake.context.scene.display.shading.color_type = "RANDOM"
        scene.run("add_primitive", {"kind": "cube", "name": "Box"})
        result = scene.run("set_material", {"name": "Box", "color": "#ff0000"})
        self.assertIn("a random colour per object", result["note"])
        self.assertIn("do not show in its renders", result["note"])

    def test_a_workbench_render_carries_the_same_note_and_a_cycles_render_none(self):
        scene, fake = make_scene()
        with tempfile.TemporaryDirectory() as tmp:
            result = scene.run("render", {"output": os.path.join(tmp, "cycles.png")})
            self.assertNotIn("note", result)
            scene.run("set_render", {"engine": "workbench"})
            result = scene.run("render", {"output": os.path.join(tmp, "flat.png")})
            self.assertEqual(result["note"], self.FLAT)
            fake.context.scene.display.shading.color_type = "SINGLE"
            result = scene.run("render", {"output": os.path.join(tmp, "single.png")})
            self.assertIn("one colour for every object", result["note"])
        self.assertEqual(set(result) - {"note"}, {"path", "seconds", "bytes"},
                         "the note is added to the answer, not in place of any of it")

    def test_a_bpy_without_render_shading_settings_is_read_as_blenders_default(self):
        scene, fake = make_scene()
        scene.run("set_render", {"engine": "workbench"})
        del fake.context.scene.display
        self.assertEqual(scene._workbench_color_type(), "MATERIAL")


class TestWhatARenderDoesWithALight(unittest.TestCase):
    """#128, the light-shaped twin of #120: `set_light` on a Workbench scene answered
    `accepted: True, settled: True`, and the render was the same picture, because Workbench
    lights the scene itself and never reads a scene light — whatever its lighting mode.

    Two halves again. The pure decision — engine and Workbench lighting mode in, what the
    render will do with the light out — and the action that carries it: `set_light` still
    sets the light (a later EEVEE or Cycles render uses it) and answers with the note on a
    Workbench scene, without one on Cycles or EEVEE.
    """

    STUDIO = ("Workbench lights the scene itself, under its own studio light, and does not "
              "use scene lights, so this light changes nothing in its renders; "
              "`set_render engine=eevee` (or cycles) lights the scene with it")

    def test_cycles_and_eevee_use_scene_lights_and_there_is_nothing_to_add(self):
        self.assertIsNone(light_note("cycles", "STUDIO"))
        self.assertIsNone(light_note("eevee", "STUDIO"))
        # The lighting mode belongs to Workbench; under another engine it is not consulted.
        self.assertIsNone(light_note("cycles", "FLAT"))

    def test_workbench_under_its_studio_light_says_the_light_will_not_show(self):
        self.assertEqual(light_note("workbench", "STUDIO"), self.STUDIO)

    def test_workbench_under_a_matcap_or_flat_says_so_in_the_same_sentence(self):
        self.assertIn("with a MatCap, a baked image of a lit sphere",
                      light_note("workbench", "MATCAP"))
        self.assertIn("flat, with no lighting at all", light_note("workbench", "FLAT"))
        for light_mode in ("STUDIO", "MATCAP", "FLAT"):
            self.assertIn("does not use scene lights", light_note("workbench", light_mode),
                          light_mode)

    def test_a_lighting_mode_this_addon_has_not_heard_of_is_still_named_not_guessed_at(self):
        note = light_note("workbench", "LASER")
        self.assertIn("in its `LASER` lighting mode", note)
        self.assertIn("does not use scene lights", note)

    def test_set_light_on_a_cycles_scene_carries_no_note(self):
        scene, _ = make_scene()
        result = scene.run("set_light", {"kind": "sun", "energy": 3})
        self.assertNotIn("note", result)

    def test_set_light_on_a_workbench_scene_still_sets_the_light_and_says_it_will_not_show(self):
        scene, fake = make_scene()
        scene.run("set_render", {"engine": "workbench"})
        result = scene.run("set_light", {"kind": "sun", "energy": 3, "location": "0,0,10"})
        self.assertEqual(result["energy"], 3.0, "still accepted, still set")
        lights = [o for o in fake.context.scene.objects if o.type == "LIGHT"]
        self.assertEqual(len(lights), 1, "the light is in the scene for a later EEVEE render")
        self.assertEqual(lights[0].data.energy, 3.0)
        self.assertEqual(result["note"], self.STUDIO)
        self.assertEqual(set(result) - {"note"}, {"light", "kind", "energy", "location"},
                         "the note is added to the answer, not in place of any of it")

    def test_set_light_under_a_workbench_matcap_names_the_mode(self):
        scene, fake = make_scene()
        scene.run("set_render", {"engine": "workbench"})
        fake.context.scene.display.shading.light = "MATCAP"
        result = scene.run("set_light", {"kind": "point", "energy": 100})
        self.assertIn("with a MatCap", result["note"])

    def test_switching_to_eevee_after_the_light_drops_the_note(self):
        scene, _ = make_scene()
        scene.run("set_render", {"engine": "workbench"})
        self.assertIn("note", scene.run("set_light", {"kind": "sun", "energy": 3}))
        scene.run("set_render", {"engine": "eevee"})
        self.assertNotIn("note", scene.run("set_light", {"kind": "sun", "energy": 5}))

    def test_a_bpy_without_render_shading_settings_is_read_as_blenders_default(self):
        scene, fake = make_scene()
        scene.run("set_render", {"engine": "workbench"})
        del fake.context.scene.display
        self.assertEqual(scene._workbench_light_mode(), "STUDIO")
        self.assertEqual(scene.run("set_light", {"kind": "sun"})["note"], self.STUDIO)


if __name__ == "__main__":
    unittest.main()
