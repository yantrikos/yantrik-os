"""The generic half, against a desktop that remembers: `python3 -m unittest discover harnesses/tests`.

Every test here is about the one rule the desktop actually enforces — a turn that was handed over
is owed exactly one `complete` or `fail` — and about the two things a harness gets wrong when it
is written again from scratch: it stops breathing while it works, and it swallows the message that
arrives while it is working.
"""

import os
import tempfile
import threading
import time
import unittest

import support
from support import FakeDesktop, recording_turn, said, wait_for

import yantrik_harness
from yantrik_harness import Handler, Harness, tool_trail


class Echo(Handler):
    def answer(self, turn):
        turn.emit("You said: ")
        turn.emit(turn.text)


class Silent(Handler):
    def answer(self, turn):
        return


class Raising(Handler):
    def answer(self, turn):
        raise ValueError("my model is not loaded")


class Slow(Handler):
    def __init__(self, seconds=1.0):
        self.seconds = seconds
        self.started = threading.Event()
        self.release = threading.Event()

    def answer(self, turn):
        self.started.set()
        self.release.wait(self.seconds)
        turn.emit("done")


class Waiting(Handler):
    """Emits nothing and waits to be stopped."""

    def __init__(self):
        self.started = threading.Event()
        self.cancelled_from = []

    def answer(self, turn):
        self.started.set()
        turn.cancelled.wait(20)

    def cancel(self, turn):
        self.cancelled_from.append(turn.turn_id)


class Resettable(Handler):
    def __init__(self):
        self.resets = 0

    def answer(self, turn):
        turn.emit("ok")

    def reset(self):
        self.resets += 1


@unittest.skipUnless(support.HAS_UNIX_SOCKETS, "the harness socket is a unix socket")
class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.desktop = FakeDesktop()
        self.addCleanup(self.desktop.stop)

    def start(self, handler, **kwargs):
        kwargs.setdefault("heartbeat_seconds", 30.0)
        kwargs.setdefault("poll_interval", 0.02)
        kwargs.setdefault("retry_seconds", 0.2)
        harness = Harness("test", "Test", handler, detail="a fake", tools=True,
                          address=self.desktop.path, log=lambda message: None, **kwargs)
        thread = threading.Thread(target=harness.run, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 3)
        self.addCleanup(harness.stop)
        self.assertTrue(wait_for(lambda: bool(self.desktop.attachments)), "never attached")
        return harness

    # ── the happy path ──────────────────────────────────────────────────

    def test_a_turn_goes_out_and_comes_back_in_pieces(self):
        self.start(Echo())
        announced = self.desktop.attachments[0]
        self.assertEqual(announced["id"], "test")
        self.assertEqual(announced["detail"], "a fake")
        self.assertTrue(announced["tools"])
        self.assertFalse(announced["memory"])

        turn = self.desktop.ask("what is open?")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(self.desktop.text(turn), "You said: what is open?")

    def test_the_context_the_desktop_offers_reaches_the_handler(self):
        seen = []

        class Peek(Handler):
            def answer(self, turn):
                seen.append(turn.context)
                turn.emit("ok")

        self.start(Peek())
        turn = self.desktop.ask("where am I?", context='{"machine": {"timezone": "Asia/Kolkata"}}')
        self.desktop.wait_closed(turn)
        self.assertIn("Asia/Kolkata", seen[0])

    # ── closed exactly once ─────────────────────────────────────────────

    def test_a_handler_that_raises_fails_the_turn_once_with_a_readable_sentence(self):
        self.start(Raising())
        turn = self.desktop.ask("hi")
        closed = self.desktop.wait_closed(turn)
        self.assertEqual(closed[1], "fail")
        self.assertEqual(closed[2], "my model is not loaded.")
        time.sleep(0.2)
        self.assertEqual(len(self.desktop.closes_for(turn)), 1, "closed twice")

    def test_a_handler_that_says_nothing_says_so_rather_than_leaving_an_empty_bubble(self):
        self.start(Silent())
        turn = self.desktop.ask("hi")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(self.desktop.text(turn), "(no answer)")
        self.assertEqual(len(self.desktop.closes_for(turn)), 1)

    def test_stop_cancels_the_running_turn_and_both_turns_are_closed_once(self):
        handler = Waiting()
        self.start(handler)
        working = self.desktop.ask("a long job")
        self.assertTrue(handler.started.wait(3), "the turn never started")
        stopping = self.desktop.ask("/stop")

        self.assertEqual(self.desktop.wait_closed(stopping)[1], "complete")
        self.assertIn("stopping", self.desktop.text(stopping))
        self.assertEqual(self.desktop.wait_closed(working)[1], "complete")
        self.assertEqual(self.desktop.text(working), "(stopped)")
        self.assertEqual(handler.cancelled_from, [working])
        time.sleep(0.2)
        self.assertEqual(len(self.desktop.closes_for(working)), 1)
        self.assertEqual(len(self.desktop.closes_for(stopping)), 1)

    def test_new_is_answered_here_and_reaches_the_handler(self):
        handler = Resettable()
        self.start(handler)
        turn = self.desktop.ask("/new")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(handler.resets, 1)
        self.assertIn("new conversation", self.desktop.text(turn))

    # ── staying alive ───────────────────────────────────────────────────

    def test_a_slow_turn_keeps_breathing(self):
        # Without this the desktop reaps the harness after 90 seconds and tells the person it
        # stopped responding while it is working.
        handler = Slow(seconds=1.0)
        self.start(handler, heartbeat_seconds=0.2)
        turn = self.desktop.ask("something slow")
        self.assertTrue(wait_for(lambda: self.desktop.heartbeats(turn) >= 2, timeout=3),
                        "no heartbeat during a slow turn")
        handler.release.set()
        self.desktop.wait_closed(turn)
        self.assertEqual(self.desktop.text(turn), "done")

    def test_a_dead_session_is_recovered_by_attaching_again(self):
        self.start(Echo())
        self.desktop.invalidate()
        self.assertTrue(wait_for(lambda: len(self.desktop.attachments) >= 2, timeout=5),
                        "never re-attached")
        turn = self.desktop.ask("still there?")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(self.desktop.text(turn), "You said: still there?")

    # ── a message that arrives while you are working ────────────────────

    def test_a_second_turn_mid_turn_is_answered_rather_than_queued(self):
        handler = Slow(seconds=5.0)
        self.start(handler)
        first = self.desktop.ask("the long one")
        self.assertTrue(handler.started.wait(3))
        second = self.desktop.ask("and this?")

        self.assertEqual(self.desktop.wait_closed(second)[1], "complete")
        self.assertIn("still working", self.desktop.text(second))
        self.assertIn("/stop", self.desktop.text(second))
        handler.release.set()
        self.desktop.wait_closed(first)

    def test_a_concurrent_handler_runs_both_at_once(self):
        class Both(Handler):
            concurrent = True

            def __init__(self):
                self.running = 0
                self.most = 0
                self.lock = threading.Lock()

            def answer(self, turn):
                with self.lock:
                    self.running += 1
                    self.most = max(self.most, self.running)
                time.sleep(0.3)
                with self.lock:
                    self.running -= 1
                turn.emit("ok")

        handler = Both()
        self.start(handler)
        first = self.desktop.ask("one")
        second = self.desktop.ask("two")
        self.desktop.wait_closed(first)
        self.desktop.wait_closed(second)
        self.assertEqual(handler.most, 2)
        self.assertEqual(self.desktop.text(second), "ok")


class TrailTests(unittest.TestCase):
    def test_a_tool_call_is_one_line_naming_what_it_touched(self):
        self.assertEqual(tool_trail("os_act", {"app": "calendar", "action": "add_event"}),
                         "⚙️ os_act calendar.add_event")
        self.assertEqual(tool_trail("os_describe", {"app": "notes"}), "⚙️ os_describe notes")
        self.assertEqual(tool_trail("os_apps", {}), "⚙️ os_apps")
        # A model that copies `new_note()` out of os_describe sends the punctuation too.
        self.assertEqual(tool_trail("os_act", {"app": "notes", "action": "new_note()"}),
                         "⚙️ os_act notes.new_note")

    def test_the_arguments_ride_on_the_line_so_the_panel_can_show_them(self):
        # #125: the panel showed `os_act studio.generate` and nothing of what was generated,
        # next to an approval card that listed every argument of the same call.
        line = tool_trail("os_act", {"app": "studio", "action": "generate",
                                     "args": {"prompt": "a red kite", "count": 1}})
        self.assertEqual(line, '⚙️ os_act studio.generate {"args":{"prompt":"a red kite","count":1}}')
        # What the label already says is not said twice.
        self.assertNotIn('"app"', line)

    def test_the_trail_is_one_line_whatever_the_arguments_hold(self):
        line = tool_trail("os_act", {"app": "notes", "action": "new_note",
                                     "args": {"body": "first line\nsecond line", "when": object()}})
        self.assertEqual(len(line.splitlines()), 1)
        self.assertIn("first line\\nsecond line", line)

    def test_the_trail_sits_on_its_own_line_with_a_blank_one_after_it(self):
        turn, recorder = recording_turn("hi")
        turn.emit("Looking at the calendar.")
        turn.tool("os_act", {"app": "calendar", "action": "add_event"})
        turn.emit("Done.")
        self.assertEqual(
            said(recorder),
            "Looking at the calendar.\n⚙️ os_act calendar.add_event\n\nDone.")

    def test_a_trail_first_does_not_start_with_a_stray_newline(self):
        turn, recorder = recording_turn("hi")
        turn.tool("os_apps", {})
        self.assertEqual(said(recorder), "⚙️ os_apps\n\n")


class EventTests(unittest.TestCase):
    """What a turn says it is doing, beside the text."""

    def test_a_tool_call_is_a_card_and_still_a_trail_line(self):
        turn, recorder = recording_turn("add dentist")
        turn.tool_start("t1", "os_act", args={"app": "calendar", "action": "add_event"})
        turn.tool_output("t1", "added\n")
        turn.tool_end("t1", True, "added the event\nsecond line", exit_code=0)
        self.assertEqual(recorder.kinds(), ["tool_start", "tool_output", "tool_end"])
        start, output, end = recorder.events
        self.assertEqual((start["call"], start["name"], start["target"]),
                         ("t1", "os_act", "calendar.add_event"))
        self.assertEqual(start["args"], {"app": "calendar", "action": "add_event"})
        self.assertEqual((output["stream"], output["delta"]), ("stdout", "added\n"))
        self.assertEqual((end["ok"], end["summary"], end["exit_code"]),
                         (True, "added the event", 0))
        # Every reader of the text — and every panel that draws no cards — still sees the call.
        self.assertEqual(said(recorder), tool_trail("os_act", {"app": "calendar",
                                                               "action": "add_event"}) + "\n\n")

    def test_the_trail_line_can_be_left_out_for_a_call_that_touched_nothing(self):
        turn, recorder = recording_turn("hi")
        turn.tool_start("t1", "os_act", args={"arguments": "{not json"}, trail=False)
        self.assertEqual(said(recorder), "")
        self.assertEqual(recorder.kinds(), ["tool_start"])

    def test_long_output_is_sent_in_pieces_each_under_the_desktops_limit(self):
        import json
        turn, recorder = recording_turn("hi")
        # ASCII, and characters that the wire escapes to twelve bytes each.
        for text in ("x" * 200_000, "\U0001F600" * 30_000):
            recorder.events.clear()
            turn.tool_output("t1", text)
            self.assertGreater(len(recorder.events), 1)
            self.assertEqual("".join(e["delta"] for e in recorder.events), text)
            for event in recorder.events:
                self.assertLess(len(json.dumps(event)), yantrik_harness.MAX_EVENT_BYTES)

    def test_huge_arguments_are_cut_rather_than_the_call_refused(self):
        import json
        turn, recorder = recording_turn("hi")
        turn.tool_start("t1", "os_act", args={"app": "notes", "body": "y" * 100_000}, trail=False)
        event = recorder.events[0]
        self.assertLess(len(json.dumps(event)), yantrik_harness.MAX_EVENT_BYTES)
        self.assertTrue(event["args"]["truncated"])
        self.assertEqual(event["target"], "notes")

    def test_thinking_status_and_usage(self):
        turn, recorder = recording_turn("hi")
        turn.thinking("let me look")
        turn.status("waiting for your approval")
        turn.usage(model="qwen", output_tokens=412)
        self.assertEqual(recorder.events, [
            {"kind": "thinking", "delta": "let me look"},
            {"kind": "status", "text": "waiting for your approval"},
            # What the harness does not know is left out, not sent as zero.
            {"kind": "usage", "model": "qwen", "output_tokens": 412},
        ])

    def test_a_target_is_what_was_touched_when_that_can_be_said(self):
        self.assertEqual(yantrik_harness.tool_target("os_act", {"app": "notes", "action": "new_note()"}),
                         "notes.new_note")
        self.assertEqual(yantrik_harness.tool_target("read", {"path": "~/Pictures"}), "~/Pictures")
        self.assertEqual(yantrik_harness.tool_target("bash", {"command": "ls -la\npwd"}), "ls -la")
        self.assertEqual(yantrik_harness.tool_target("os_apps", {}), "")

    def test_a_turn_knows_its_conversation_and_its_agent(self):
        turn, _ = recording_turn("hi", conversation="c-7f3a91", agent_token="ab" * 16)
        self.assertEqual((turn.conversation, turn.agent_token), ("c-7f3a91", "ab" * 16))
        plain, _ = recording_turn("hi")
        self.assertEqual((plain.conversation, plain.agent_token), ("main", ""))


class Minds(Handler):
    """One `Mind` per conversation, recorded, for the PerConversation tests."""

    def __init__(self):
        self.made = []          # (conversation, token)
        self.closed = []        # conversations whose mind was closed
        self.lock = threading.Lock()

    def make(self, conversation, token):
        outer = self

        class Mind(Handler):
            def __init__(self):
                self.conversation = conversation
                self.resets = 0
                self.running = threading.Event()
                self.release = threading.Event()

            def answer(self, turn):
                self.running.set()
                if turn.text == "slow":
                    self.release.wait(20)
                turn.emit("%s heard %s" % (conversation, turn.text))

            def reset(self):
                self.resets += 1

            def close(self):
                with outer.lock:
                    outer.closed.append(conversation)

        with self.lock:
            self.made.append((conversation, token))
        return Mind()


@unittest.skipUnless(support.HAS_UNIX_SOCKETS, "the harness socket is a unix socket")
class ConversationTests(unittest.TestCase):
    def setUp(self):
        self.desktop = FakeDesktop()
        self.addCleanup(self.desktop.stop)
        self.logged = []

    def start(self, handler, desktop=None):
        desktop = desktop or self.desktop
        harness = Harness("test", "Test", handler, address=desktop.path,
                          log=self.logged.append, heartbeat_seconds=30.0,
                          poll_interval=0.02, retry_seconds=0.2)
        thread = threading.Thread(target=harness.run, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 3)
        self.addCleanup(harness.stop)
        self.assertTrue(wait_for(lambda: bool(desktop.attachments)), "never attached")
        return harness

    def test_a_harness_says_whether_it_holds_many_conversations(self):
        self.start(yantrik_harness.PerConversation(Minds().make))
        self.assertTrue(self.desktop.attachments[0]["conversations"])
        other = FakeDesktop()
        self.addCleanup(other.stop)
        self.start(Echo(), desktop=other)
        self.assertFalse(other.attachments[0]["conversations"])

    def test_each_conversation_gets_its_own_mind_made_with_its_agents_token(self):
        minds = Minds()
        self.start(yantrik_harness.PerConversation(minds.make))
        a = self.desktop.ask("hi", conversation="c-aaaaaa", agent_token="a" * 32)
        b = self.desktop.ask("hi", conversation="c-bbbbbb", agent_token="b" * 32)
        # The second message to c-aaaaaa goes only once the first has closed: sent while it is
        # still running, it is rightly answered "still working", which is a different test.
        self.desktop.wait_closed(a)
        again = self.desktop.ask("again", conversation="c-aaaaaa", agent_token="a" * 32)
        for turn in (b, again):
            self.desktop.wait_closed(turn)
        self.assertEqual(self.desktop.text(a), "c-aaaaaa heard hi")
        self.assertEqual(self.desktop.text(b), "c-bbbbbb heard hi")
        self.assertEqual(self.desktop.text(again), "c-aaaaaa heard again")
        self.assertEqual(sorted(minds.made), [("c-aaaaaa", "a" * 32), ("c-bbbbbb", "b" * 32)])

    def test_different_conversations_run_at_once_and_the_same_one_does_not(self):
        minds = Minds()
        handler = yantrik_harness.PerConversation(minds.make)
        self.start(handler)
        slow = self.desktop.ask("slow", conversation="c-aaaaaa")
        self.assertTrue(wait_for(lambda: handler.mind("c-aaaaaa") is not None
                                 and handler.mind("c-aaaaaa").running.is_set()))
        # Another conversation is answered while the first is still working...
        other = self.desktop.ask("quick", conversation="c-bbbbbb")
        self.assertEqual(self.desktop.wait_closed(other)[1], "complete")
        self.assertEqual(self.desktop.text(other), "c-bbbbbb heard quick")
        # ...and a second message to the busy one is told so, not interleaved into it.
        same = self.desktop.ask("and this?", conversation="c-aaaaaa")
        self.desktop.wait_closed(same)
        self.assertIn("still working", self.desktop.text(same))
        self.assertEqual(self.desktop.closes_for(slow), [])
        handler.mind("c-aaaaaa").release.set()
        self.assertEqual(self.desktop.wait_closed(slow)[1], "complete")

    def test_stop_in_one_conversation_leaves_the_others_running(self):
        handler = Waiting()

        class Two(Handler):
            conversations = True

            def answer(self, turn):
                handler.answer(turn)

            def cancel(self, turn):
                handler.cancel(turn)

        self.start(Two())
        one = self.desktop.ask("job", conversation="c-111111")
        self.assertTrue(handler.started.wait(3))
        two = self.desktop.ask("job", conversation="c-222222")
        self.assertTrue(wait_for(lambda: len(self.desktop.open_turns) == 2))
        stop = self.desktop.ask("/stop", conversation="c-111111")
        self.desktop.wait_closed(stop)
        self.desktop.wait_closed(one)
        self.assertEqual(handler.cancelled_from, [one])
        self.assertEqual(self.desktop.closes_for(two), [], "stopped a conversation nobody stopped")

    def test_new_forgets_only_its_own_conversation(self):
        minds = Minds()
        handler = yantrik_harness.PerConversation(minds.make)
        self.start(handler)
        for conversation in ("c-aaaaaa", "c-bbbbbb"):
            self.desktop.wait_closed(self.desktop.ask("hi", conversation=conversation))
        self.desktop.wait_closed(self.desktop.ask("/new", conversation="c-aaaaaa"))
        self.assertEqual(handler.mind("c-aaaaaa").resets, 1)
        self.assertEqual(handler.mind("c-bbbbbb").resets, 0)

    def test_a_cancelled_turn_is_stopped_and_still_closed_once(self):
        handler = Waiting()
        self.start(handler)
        turn = self.desktop.ask("a long job")
        self.assertTrue(handler.started.wait(3))
        self.desktop.stop_agent(turn_id=turn)
        self.assertTrue(wait_for(lambda: handler.cancelled_from == [turn]), "the mind was not told")
        self.desktop.wait_closed(turn)
        time.sleep(0.2)
        self.assertEqual(len(self.desktop.closes_for(turn)), 1)

    def test_an_ended_conversation_lets_its_mind_go(self):
        minds = Minds()
        handler = yantrik_harness.PerConversation(minds.make)
        self.start(handler)
        self.desktop.wait_closed(self.desktop.ask("hi", conversation="c-aaaaaa"))
        self.desktop.wait_closed(self.desktop.ask("hi", conversation="c-bbbbbb"))
        self.desktop.stop_agent(conversation="c-aaaaaa")
        self.assertTrue(wait_for(lambda: minds.closed == ["c-aaaaaa"]), minds.closed)
        self.assertEqual(handler.held(), ["c-bbbbbb"])

    def test_a_conversation_started_over_under_a_new_token_gets_a_new_mind(self):
        # The desktop stopped `main` and the Lens spoke to it again: the same conversation name,
        # a new agent. A process made under the old token must not answer for the new one.
        minds = Minds()
        self.start(yantrik_harness.PerConversation(minds.make))
        self.desktop.wait_closed(self.desktop.ask("hi", conversation="main", agent_token="a" * 32))
        self.desktop.wait_closed(self.desktop.ask("hi", conversation="main", agent_token="b" * 32))
        self.assertEqual(minds.made, [("main", "a" * 32), ("main", "b" * 32)])
        self.assertTrue(wait_for(lambda: minds.closed == ["main"]))

    def test_attaching_again_ends_the_conversations_of_the_session_before(self):
        minds = Minds()
        handler = yantrik_harness.PerConversation(minds.make)
        self.start(handler)
        self.desktop.wait_closed(self.desktop.ask("hi", conversation="c-aaaaaa"))
        self.desktop.invalidate()
        self.assertTrue(wait_for(lambda: len(self.desktop.attachments) >= 2, timeout=5))
        self.assertTrue(wait_for(lambda: minds.closed == ["c-aaaaaa"]), minds.closed)
        self.assertEqual(handler.held(), [])

    def test_past_its_own_limit_a_new_conversation_is_told_why(self):
        minds = Minds()
        self.start(yantrik_harness.PerConversation(minds.make, limit=1))
        self.desktop.wait_closed(self.desktop.ask("hi", conversation="c-aaaaaa"))
        closed = self.desktop.wait_closed(self.desktop.ask("hi", conversation="c-bbbbbb"))
        self.assertEqual(closed[1], "fail")
        self.assertIn("already holding 1 conversations", closed[2])

    def test_events_reach_the_desktop_for_their_turn(self):
        class Tooling(Handler):
            def answer(self, turn):
                turn.emit("Looking.\n")
                turn.tool_start("t1", "os_apps")
                turn.tool_output("t1", "notes, calendar")
                turn.tool_end("t1", True, "2 apps")
                turn.usage(model="m", input_tokens=10)
                turn.emit("Two apps.")

        self.start(Tooling())
        turn = self.desktop.ask("what is open?")
        self.assertEqual(self.desktop.wait_closed(turn)[1], "complete")
        self.assertEqual(self.desktop.kinds(turn), ["tool_start", "tool_output", "tool_end", "usage"])
        self.assertIn("⚙️ os_apps", self.desktop.text(turn))
        self.assertTrue(self.desktop.text(turn).endswith("Two apps."))

    def test_against_a_desktop_from_before_events_the_turn_is_unchanged(self):
        old = FakeDesktop(events=False)
        self.addCleanup(old.stop)

        class Tooling(Handler):
            def answer(self, turn):
                for n in range(3):
                    turn.tool_start("t%d" % n, "os_apps")
                    turn.tool_end("t%d" % n, True)
                turn.emit("done")

        self.start(Tooling(), desktop=old)
        turn = old.ask("hi")
        self.assertEqual(old.wait_closed(turn)[1], "complete")
        self.assertTrue(old.text(turn).endswith("done"))
        self.assertEqual(old.text(turn).count("⚙️ os_apps"), 3)
        # Said once, not once per event.
        self.assertEqual(sum("does not take harness.event" in m for m in self.logged), 1)


class NoteTests(unittest.TestCase):
    """What the desktop has to tell an agent since its last turn, from the turn's context."""

    def test_notes_are_read_from_the_context_and_nothing_else_is(self):
        import json
        context = json.dumps({"machine": {"timezone": "UTC"},
                              "notes": ["Your command `make` finished: exit code 0.", "  "]})
        turn, _ = recording_turn("what now?", context)
        self.assertEqual(turn.notes, ["Your command `make` finished: exit code 0."])
        self.assertEqual(
            turn.notes_before(turn.text),
            "[From the desktop, since your last turn:\n- Your command `make` finished: exit "
            "code 0.]\n\nwhat now?")
        # No notes, a context that is not an object, and no context at all: the text alone.
        for context in (json.dumps({"machine": {}}), "plain words", None, json.dumps(["x"])):
            turn, _ = recording_turn("hi", context)
            self.assertEqual((turn.notes, turn.notes_before("hi")), ([], "hi"), context)

    def test_a_note_of_several_lines_stays_inside_its_bullet(self):
        import json
        turn, _ = recording_turn("go", json.dumps({"notes": ["ended.\nIts last lines:\nok"]}))
        self.assertIn("- ended.\n  Its last lines:\n  ok]", turn.notes_before("go"))

    def test_a_command_tool_is_given_as_long_as_the_command_it_waits_for(self):
        # run_command waits `wait_seconds` for the command on top of everything an os_act can
        # wait for; a client allowing only the flat budget would cut it off.
        self.assertEqual(yantrik_harness.mcp_timeout("os_act", {"app": "x"}), 300.0)
        self.assertEqual(yantrik_harness.mcp_timeout("run_command", {"command": "ls"}), 420.0)
        self.assertEqual(yantrik_harness.mcp_timeout("run_command", {"wait_seconds": 600}), 900.0)
        self.assertEqual(yantrik_harness.mcp_timeout("command_status", {"wait_seconds": 5000}), 900.0)
        self.assertEqual(yantrik_harness.mcp_timeout("command_status", {"wait_seconds": "x"}), 420.0)
        self.assertEqual(yantrik_harness.mcp_timeout("command_kill", {"job": "j"}), 300.0)
        # hand_off waits only when told to, for the role's answer.
        self.assertEqual(yantrik_harness.mcp_timeout("hand_off", {"role": "reviewer", "task": "x"}), 300.0)
        self.assertEqual(yantrik_harness.mcp_timeout("hand_off", {"wait_seconds": 240}), 540.0)


class SocketDiscoveryTests(unittest.TestCase):
    def test_an_explicit_socket_is_honoured_and_nothing_else_is_looked_at(self):
        # Pointing a harness at one desktop must never silently fall through to another.
        old = os.environ.get("YANTRIK_HARNESS_SOCKET")
        os.environ["YANTRIK_HARNESS_SOCKET"] = "/nowhere/harness.sock"
        try:
            self.assertIsNone(yantrik_harness.socket_path())
        finally:
            if old is None:
                os.environ.pop("YANTRIK_HARNESS_SOCKET", None)
            else:
                os.environ["YANTRIK_HARNESS_SOCKET"] = old


class MindDirectoryTests(unittest.TestCase):
    """Where an agent process is started (#183): the desktop's own data directory, never $HOME.

    A coding agent reads the instruction files of its working directory and its parents, so a
    directory that is not the desktop's own — the harness service's $HOME above all — lets a
    person's own ~/CLAUDE.md steer the mind.
    """

    def setUp(self):
        self.data = tempfile.mkdtemp(prefix="mind-dir-")
        self.previous = os.environ.get("XDG_DATA_HOME")
        os.environ["XDG_DATA_HOME"] = self.data
        self.addCleanup(self.restore)

    def restore(self):
        if self.previous is None:
            os.environ.pop("XDG_DATA_HOME", None)
        else:
            os.environ["XDG_DATA_HOME"] = self.previous

    def test_a_conversation_gets_its_own_made_directory_under_the_data_home(self):
        where = yantrik_harness.mind_directory("pi", "c-abc123")
        self.assertEqual(where, os.path.join(self.data, "yantrik", "minds", "pi", "c-abc123"))
        self.assertTrue(os.path.isdir(where))
        self.assertNotEqual(where, os.path.expanduser("~"))

    def test_an_id_from_the_wire_cannot_name_a_directory_outside_the_tree(self):
        # The conversation id arrives over the harness socket; one carrying a path must stay a
        # single name inside minds/<harness>, whatever it spells.
        for sneaky in ("../../etc", "..", ".", "/home/me"):
            where = yantrik_harness.mind_directory("pi", sneaky)
            self.assertEqual(os.path.dirname(where),
                             os.path.join(self.data, "yantrik", "minds", "pi"))
            self.assertTrue(os.path.isdir(where))

    def test_a_harness_with_one_conversation_still_gets_a_directory_of_its_own(self):
        where = yantrik_harness.mind_directory("openclaw")
        self.assertEqual(where,
                         os.path.join(self.data, "yantrik", "minds", "openclaw", "main"))
        self.assertTrue(os.path.isdir(where))


if __name__ == "__main__":
    unittest.main()
