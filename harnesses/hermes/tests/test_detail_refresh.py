"""Hermes says its model again when the person picks another (#1114): once per change, per turn.

    python3 -m unittest discover -s harnesses/hermes/tests -p test_detail_refresh.py
"""

import asyncio
import http.server
import os
import threading
import unittest
from unittest import mock

import test_adapter_behaviour as base
from test_adapter_behaviour import setUpModule, tearDownModule  # noqa: F401  (the faked Hermes, set up here too)


class DetailRefreshTests(base.AdapterCase):
    async def test_a_changed_pick_is_sent_before_the_turn_and_an_unchanged_one_is_not(self):
        self.adapter._detail_said = "Hermes 1 · deepseek"
        with mock.patch.object(base.adapter_mod, "_detail", return_value="Hermes 1 · qwen3.8:27b"):
            await self.adapter._refresh_detail()
            await self.adapter._refresh_detail()
        sent = self.fake.of(base.desktop_mod.DETAIL)
        self.assertEqual(sent, [{"session": "s1", "detail": "Hermes 1 · qwen3.8:27b"}])
        self.assertEqual(self.fake.of(base.desktop_mod.ATTACH), [], "the line changes without attaching again")

    async def test_attaching_again_says_the_model_at_once_not_at_the_next_turn(self):
        """After a shell restart the attach line may lack the model (#1162): the refresh follows it."""
        said = iter(["Hermes 1", "Hermes 1 · qwen3.8:27b"])
        self.adapter._session = None
        with mock.patch.object(base.adapter_mod, "_detail", side_effect=lambda: next(said)), mock.patch.object(
            base.desktop_mod, "socket_path", return_value="/nowhere"
        ):
            self.assertTrue(await self.adapter._attach())
            await self.adapter._detail_task
        self.assertEqual(self.fake.of(base.desktop_mod.ATTACH)[0]["detail"], "Hermes 1")
        self.assertEqual(self.fake.of(base.desktop_mod.DETAIL), [{"session": "s1", "detail": "Hermes 1 · qwen3.8:27b"}])

    async def test_a_desktop_that_refuses_it_does_not_stop_the_turn(self):
        self.adapter._detail_said = "old"
        def refuse(*args, **kwargs):
            raise base.desktop_mod.HarnessError("no")

        base.desktop_mod.call = refuse  # AdapterCase.tearDown puts the fake back
        with mock.patch.object(base.adapter_mod, "_detail", return_value="new"):
            await self.adapter._refresh_detail()
        self.assertEqual(self.adapter._detail_said, "old", "tried again on the next turn")

    async def test_a_slow_refresh_does_not_hold_the_turn_up(self):
        release = asyncio.Event()
        started = []

        async def slow():
            started.append(True)
            await release.wait()

        self.adapter._refresh_detail = slow
        self.adapter._refresh_detail_soon()
        self.adapter._refresh_detail_soon()  # one at a time
        await asyncio.sleep(0)
        self.assertEqual(started, [True], "started, and the caller was not made to wait for it")
        self.assertFalse(self.adapter._detail_task.done())
        release.set()
        await self.adapter._detail_task


def _serve(test, handler):
    server = http.server.HTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    test.addCleanup(server.server_close)
    test.addCleanup(server.shutdown)
    return server


class _Quiet(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass


class NoProxyTests(unittest.TestCase):
    """M1: the gateway token goes to the loopback gateway directly, never to a proxy."""

    def test_the_token_never_reaches_a_proxy_named_by_the_environment(self):
        seen = []

        class Proxy(_Quiet):
            def do_GET(self):
                seen.append(self.headers.get("Authorization"))
                self.send_response(502)
                self.end_headers()

        proxy = _serve(self, Proxy)
        url = base.DetailTests.serve(self, "ollama-cloud/deepseek-v4.1-flash")
        address = "http://127.0.0.1:%d" % proxy.server_port
        env = {"http_proxy": address, "HTTP_PROXY": address, "https_proxy": address, "HTTPS_PROXY": address, "no_proxy": "", "NO_PROXY": ""}
        with mock.patch.dict(os.environ, env):
            model = base.gateway_model_mod().real_model({"base_url": url, "api_key": "ygw-t"})
        self.assertEqual(model, "deepseek-v4.1-flash", "answered by the gateway itself")
        self.assertEqual(seen, [], "the proxy saw nothing, so it saw no token")

    def test_a_redirect_is_not_followed_with_the_token(self):
        hits = []

        class Elsewhere(_Quiet):
            def do_GET(self):
                hits.append(self.headers.get("Authorization"))
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"{}")

        other = _serve(self, Elsewhere)
        target = "http://127.0.0.1:%d/v1/models" % other.server_port

        class Redirector(_Quiet):
            def do_GET(self):
                self.send_response(302)
                self.send_header("Location", target)
                self.end_headers()

        gateway = _serve(self, Redirector)
        url = "http://127.0.0.1:%d/v1" % gateway.server_port
        self.assertIsNone(base.gateway_model_mod().real_model({"base_url": url, "api_key": "ygw-t"}))
        self.assertEqual(hits, [])


if __name__ == "__main__":
    unittest.main()
