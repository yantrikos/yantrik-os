"""The desktop side of the Hermes bridge, without Hermes: `python3 -m unittest discover harnesses/hermes/tests`."""

import importlib.util
import json
import os
import socket
import sys
import tempfile
import threading
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location("desktop", Path(__file__).resolve().parents[1] / "desktop.py")
desktop = importlib.util.module_from_spec(_SPEC)
sys.modules["desktop"] = desktop  # dataclasses resolve annotations through sys.modules
_SPEC.loader.exec_module(desktop)


class LedgerTests(unittest.TestCase):
    def test_a_reply_goes_to_the_turn_it_answers_even_when_a_newer_one_is_open(self):
        ledger = desktop.Ledger()
        first = ledger.open("1", "desktop", "compare flights")
        ledger.open("2", "desktop", "/approve")
        self.assertIs(ledger.route("desktop", "1"), first)

    def test_without_a_reply_anchor_the_newest_turn_in_the_chat_is_the_one_on_screen(self):
        ledger = desktop.Ledger()
        ledger.open("1", "desktop", "first")
        second = ledger.open("2", "desktop", "second")
        ledger.open("3", "elsewhere", "other chat")
        self.assertIs(ledger.route("desktop", None), second)
        self.assertIs(ledger.route("desktop", "no-such-turn"), second)

    def test_nothing_open_means_nothing_to_answer(self):
        self.assertIsNone(desktop.Ledger().route("desktop", None))

    def test_messages_on_one_turn_are_separated_so_they_do_not_run_together(self):
        ledger = desktop.Ledger()
        turn = ledger.open("7", "desktop", "task")
        _, first = ledger.sent(turn, "Looking at the repo.")
        _, second = ledger.sent(turn, "Done: 3 files changed.")
        self.assertEqual(first, "Looking at the repo.")
        self.assertEqual(second, "\n\nDone: 3 files changed.")

    def test_an_edit_that_grows_a_message_streams_only_what_it_adds(self):
        ledger = desktop.Ledger()
        turn = ledger.open("7", "desktop", "task")
        message_id, _ = ledger.sent(turn, "💻 terminal: \"ls\"")
        got = ledger.edited(message_id, "💻 terminal: \"ls\"\n📖 read_file: \"README.md\"")
        self.assertEqual(got, (turn, "\n📖 read_file: \"README.md\""))
        self.assertEqual(ledger.edited(message_id, "💻 terminal: \"ls\"\n📖 read_file: \"README.md\""), (turn, ""))

    def test_a_rewritten_line_is_repeated_rather_than_lost(self):
        ledger = desktop.Ledger()
        turn = ledger.open("7", "desktop", "task")
        message_id, _ = ledger.sent(turn, "a\nb ⏳")
        _, delta = ledger.edited(message_id, "a\nb ✅\nc")
        self.assertEqual(delta, "\nb ✅\nc")

    def test_an_edit_to_a_closed_turn_goes_nowhere(self):
        ledger = desktop.Ledger()
        turn = ledger.open("7", "desktop", "task")
        message_id, _ = ledger.sent(turn, "working")
        self.assertIs(ledger.close("7"), turn)
        self.assertIsNone(ledger.edited(message_id, "working, still"))
        self.assertIsNone(ledger.edited("unknown", "x"))

    def test_a_turn_is_closed_exactly_once(self):
        ledger = desktop.Ledger()
        ledger.open("7", "desktop", "task")
        self.assertIsNotNone(ledger.close("7"))
        self.assertIsNone(ledger.close("7"))

    def test_saying_nothing_is_remembered_so_a_failure_can_still_be_reported(self):
        ledger = desktop.Ledger()
        turn = ledger.open("7", "desktop", "task")
        ledger.sent(turn, "")
        self.assertFalse(turn.said_anything)
        ledger.sent(turn, "hello")
        self.assertTrue(turn.said_anything)


class FakeDesktop:
    """A socket that answers like the shell: one JSON-RPC line in, one out, then close."""

    def __init__(self, path, answer):
        self.requests = []
        self._answer = answer
        self._server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._server.bind(path)
        self._server.listen(4)
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()

    def _serve(self):
        while True:
            try:
                conn, _ = self._server.accept()
            except OSError:
                return
            with conn:
                buf = b""
                while not buf.endswith(b"\n"):
                    piece = conn.recv(4096)
                    if not piece:
                        break
                    buf += piece
                request = json.loads(buf)
                self.requests.append(request)
                reply = self._answer(request)
                if reply is not None:
                    conn.sendall(json.dumps(reply).encode() + b"\n")

    def close(self):
        self._server.close()


@unittest.skipUnless(hasattr(socket, "AF_UNIX"), "unix sockets")
class SocketTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "harness.sock")

    def tearDown(self):
        self.dir.cleanup()

    def test_a_call_is_one_json_rpc_line_and_returns_the_result(self):
        fake = FakeDesktop(self.path, lambda r: {"jsonrpc": "2.0", "id": r["id"], "result": {"session": "s1"}})
        try:
            got = desktop.call(self.path, desktop.ATTACH, {"id": "hermes", "name": "Hermes Agent"})
        finally:
            fake.close()
        self.assertEqual(got, {"session": "s1"})
        self.assertEqual(fake.requests[0]["method"], "harness.attach")
        self.assertEqual(fake.requests[0]["params"]["id"], "hermes")

    def test_a_refusal_is_an_error_with_the_desktops_own_words(self):
        fake = FakeDesktop(
            self.path,
            lambda r: {"jsonrpc": "2.0", "id": r["id"], "error": {"code": -32000, "message": "no such session"}},
        )
        try:
            with self.assertRaisesRegex(desktop.HarnessError, "no such session"):
                desktop.call(self.path, desktop.POLL, {"session": "gone"})
        finally:
            fake.close()

    def test_a_desktop_that_hangs_up_without_answering_is_an_error_not_a_none(self):
        fake = FakeDesktop(self.path, lambda r: None)
        try:
            with self.assertRaisesRegex(desktop.HarnessError, "without answering"):
                desktop.call(self.path, desktop.POLL, {"session": "s1"}, timeout=2.0)
        finally:
            fake.close()

    def test_no_desktop_is_an_error(self):
        with self.assertRaises(desktop.HarnessError):
            desktop.call(os.path.join(self.dir.name, "missing.sock"), desktop.POLL, {})

    def test_the_socket_is_found_where_the_shell_binds_it(self):
        runtime = os.path.join(self.dir.name, "run")
        os.makedirs(os.path.join(runtime, "yantrik"))
        sock = os.path.join(runtime, "yantrik", "harness.sock")
        Path(sock).touch()
        saved = {k: os.environ.get(k) for k in ("XDG_RUNTIME_DIR", "YANTRIK_HARNESS_SOCKET")}
        try:
            os.environ.pop("YANTRIK_HARNESS_SOCKET", None)
            os.environ["XDG_RUNTIME_DIR"] = runtime
            self.assertEqual(desktop.socket_path(), sock)
            # Named outright, a missing socket is not replaced by one found elsewhere.
            os.environ["YANTRIK_HARNESS_SOCKET"] = os.path.join(self.dir.name, "nowhere.sock")
            self.assertIsNone(desktop.socket_path())
        finally:
            for key, value in saved.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value


if __name__ == "__main__":
    unittest.main()
