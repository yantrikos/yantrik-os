"""Answers that take time, and apps whose state lives on a thread of their own.

`settles="later"` is `Action::defers()`: the answer says `settled: false`. `Later(work)` is
`answer_later`: the app's thread is released at once, the work runs on the caller's thread, and
the answer carries its result with the view read again afterwards. A surface whose state
belongs to one thread overrides `run_on_app_thread` — Blender's addon is the worked example.
"""

import queue
import threading

import support
from yantrik_surface import Later, NotAnswered, Refusal, Surface, caller


class MainLoopSurface(Surface):
    """A surface whose state may only be touched on a 'main thread' of its own."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.jobs = queue.Queue()
        self.main = threading.Thread(target=self._loop, daemon=True)
        self.main.start()
        self.threads_seen = set()

    def _loop(self):
        while True:
            fn, box, done = self.jobs.get()
            try:
                box["value"] = fn()
            except BaseException as e:  # noqa: BLE001 - handed back to the submitter
                box["error"] = e
            done.set()

    def run_on_app_thread(self, fn, timeout):
        box, done = {}, threading.Event()
        self.jobs.put((fn, box, done))
        if not done.wait(timeout):
            raise NotAnswered()
        if "error" in box:
            raise box["error"]
        return box["value"]


class TestLater(support.MachineCase):
    def test_the_work_runs_off_the_app_thread_and_its_result_is_the_answer(self):
        s = MainLoopSurface("builder")
        log = []

        @s.view
        def state():
            s.threads_seen.add(threading.current_thread().name)
            return {"built": list(log)}

        @s.action("build", grade="standard", settles="later", expected_seconds=2)
        def build(target: str) -> dict:
            """Build a target."""
            on_main = threading.current_thread() is s.main

            def work():
                assert threading.current_thread() is not s.main
                log.append(target)
                return {"target": target, "ran_off_main": True, "handler_on_main": on_main}

            return Later(work)

        out = s.act({"action": "build", "args": {"target": "all"}})
        self.assertEqual(out["result"], {"target": "all", "ran_off_main": True,
                                         "handler_on_main": True})
        self.assertIs(out["settled"], False)
        # The view was read again after the work: the state beside the result is the state
        # the result came from.
        self.assertEqual(out["state"], {"built": ["all"]})
        self.assertEqual(s.threads_seen, {s.main.name}, "the view is only read on the app thread")

    def test_a_refusal_from_the_work_is_the_callers_refusal(self):
        s = Surface("builder")

        @s.action("build")
        def build() -> dict:
            """Build."""
            def work():
                raise Refusal("the compiler is not installed")
            return Later(work)

        self.assertEqual(self.refusal(lambda: s.act({"action": "build"})),
                         "the compiler is not installed")

    def test_the_app_is_free_while_the_work_runs(self):
        s = Surface("builder")
        started, release = threading.Event(), threading.Event()

        @s.action("build")
        def build() -> dict:
            """Build."""
            def work():
                started.set()
                release.wait(5)
                return {"done": True}
            return Later(work)

        answers = []
        worker = threading.Thread(target=lambda: answers.append(s.act({"action": "build"})))
        worker.start()
        self.assertTrue(started.wait(5))
        try:
            # Under the default lock a describe would wait behind a handler; it does not wait
            # behind work handed to Later.
            self.assertEqual(s.describe_json()["app"], "builder")
        finally:
            release.set()
            worker.join(5)
        self.assertEqual(answers[0]["result"], {"done": True})


class TestTheAppThread(support.MachineCase):
    def test_every_check_the_handler_and_the_view_run_in_one_turn(self):
        s = MainLoopSurface("main")
        where = []

        @s.view
        def state():
            where.append(threading.current_thread() is s.main)
            return {}

        @s.action("poke")
        def poke() -> dict:
            """Poke."""
            where.append(threading.current_thread() is s.main)
            return {"caller": caller() is None}

        current = s.describe_json()["revision"]
        out = s.act({"action": "poke", "expect_revision": current})
        self.assertTrue(out["accepted"])
        self.assertTrue(all(where), where)
        self.assertEqual(out["result"], {"caller": True}, "no socket, so no peer")

    def test_the_view_read_after_the_work_knows_who_the_caller_is(self):
        """A view that tells callers apart (the browser's) is read again after `Later`'s work, and
        must see the caller it is for — on the app's thread, not as nobody."""
        from yantrik_surface import PeerCred
        s = MainLoopSurface("main")
        seen = []

        @s.view
        def state():
            seen.append(caller())
            return {}

        @s.action("slow")
        def slow() -> dict:
            """Slow."""
            return Later(lambda: {"done": True})

        peer = PeerCred(1, 2, 3)
        s.act({"action": "slow"}, peer)
        self.assertTrue(seen and all(p == peer for p in seen), seen)

    def test_a_refusal_raised_on_the_app_thread_crosses_back(self):
        s = MainLoopSurface("main")

        @s.action("no")
        def no() -> dict:
            """Refuse."""
            raise Refusal("not today")

        self.assertEqual(self.refusal(lambda: s.act({"action": "no"})), "not today")
