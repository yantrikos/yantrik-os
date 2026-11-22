"""Drives the scripted model with a real HTTP client (urllib) over a real socket, on every API."""
import json
import os
import sys
import tempfile
import threading
import unittest
import urllib.request
import urllib.error

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import scripted_model as sm  # noqa: E402
import sm_scenario  # noqa: E402

CALLS = {"rules": [{"name": "calls", "match": {"contains": "do calls"}, "script": [
    {"text": "Looking.", "tool_calls": [{"name": "run_command", "arguments": {"command": "echo 1"}}]},
    {"tool_calls": [{"name": "run_command", "arguments": {"command": "echo 2"}}, {"name": "other", "arguments": {"x": 1}}]},
    {"text": "All done."}]},
    {"name": "rx", "match": {"regex": "^re+gex$"}, "script": [{"text": "regex hit"}]}]}


class Base(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        d = tempfile.mkdtemp()
        cls.logpath = os.path.join(d, "log")
        p = os.path.join(d, "s.json")
        json.dump(CALLS, open(p, "w"))
        cls.srv = sm.make_server(sm_scenario.Scenarios([p]), delay_scale=0, log_file=cls.logpath)
        cls.base = "http://127.0.0.1:%d" % cls.srv.server_address[1]
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()
        cls.srv.server_close()

    def req(self, path, body=None, headers=None):
        r = urllib.request.Request(self.base + path, data=None if body is None else json.dumps(body).encode(),
                                   headers=dict({"Content-Type": "application/json"}, **(headers or {})))
        with urllib.request.urlopen(r, timeout=10) as resp:
            return resp.status, resp.read().decode()

    def jreq(self, path, body=None, headers=None):
        return json.loads(self.req(path, body, headers)[1])


def sse(raw):
    return [json.loads(l[6:]) for l in raw.splitlines() if l.startswith("data: ") and l != "data: [DONE]"]


class OpenAI(Base):
    def chat(self, msgs, stream):
        return self.req("/v1/chat/completions", {"model": "m", "messages": msgs, "stream": stream})[1]

    def test_models(self):
        self.assertEqual(self.jreq("/v1/models")["data"][0]["id"], "scripted-model")

    def test_default_whole_and_stream(self):
        m = [{"role": "user", "content": "anything"}]
        whole = json.loads(self.chat(m, False))
        self.assertEqual(whole["choices"][0]["message"]["content"], sm_scenario.DEFAULT_SENTENCE)
        raw = self.chat(m, True)
        self.assertTrue(raw.rstrip().endswith("data: [DONE]"))
        text = "".join(c["choices"][0]["delta"].get("content") or "" for c in sse(raw) if c["choices"])
        self.assertEqual(text, sm_scenario.DEFAULT_SENTENCE)

    def test_tool_calls_round_trip_whole(self):
        msgs = [{"role": "user", "content": "please do calls"}]
        a = json.loads(self.chat(msgs, False))["choices"][0]
        self.assertEqual(a["finish_reason"], "tool_calls")
        call = a["message"]["tool_calls"][0]
        self.assertEqual(call["function"]["name"], "run_command")
        self.assertEqual(json.loads(call["function"]["arguments"]), {"command": "echo 1"})
        msgs += [a["message"], {"role": "tool", "tool_call_id": call["id"], "content": "1"}]
        b = json.loads(self.chat(msgs, False))["choices"][0]
        self.assertEqual([c["function"]["name"] for c in b["message"]["tool_calls"]], ["run_command", "other"])
        msgs += [b["message"]] + [{"role": "tool", "tool_call_id": c["id"], "content": "ok"} for c in b["message"]["tool_calls"]]
        c = json.loads(self.chat(msgs, False))["choices"][0]
        self.assertEqual((c["finish_reason"], c["message"]["content"]), ("stop", "All done."))

    def test_tool_calls_streamed(self):
        chunks = sse(self.chat([{"role": "user", "content": "do calls"}], True))
        name, args, finish = "", "", None
        for ch in chunks:
            for choice in ch["choices"]:
                finish = choice["finish_reason"] or finish
                for tc in choice["delta"].get("tool_calls") or []:
                    name += tc["function"].get("name") or ""
                    args += tc["function"].get("arguments") or ""
        self.assertEqual((name, json.loads(args), finish), ("run_command", {"command": "echo 1"}, "tool_calls"))

    def test_regex_rule(self):
        out = json.loads(self.chat([{"role": "user", "content": "reeegex"}], False))
        self.assertEqual(out["choices"][0]["message"]["content"], "regex hit")


class Anthropic(Base):
    H = {"x-api-key": "none", "anthropic-version": "2023-06-01"}

    def msg(self, msgs, stream):
        return self.req("/v1/messages", {"model": "m", "max_tokens": 10, "messages": msgs, "stream": stream}, self.H)[1]

    def test_models(self):
        out = self.jreq("/v1/models", headers=self.H)
        self.assertEqual(out["data"][0]["type"], "model")

    def test_whole_tool_use_round_trip(self):
        msgs = [{"role": "user", "content": [{"type": "text", "text": "do calls"}]}]
        a = json.loads(self.msg(msgs, False))
        self.assertEqual(a["stop_reason"], "tool_use")
        use = [b for b in a["content"] if b["type"] == "tool_use"][0]
        self.assertEqual((use["name"], use["input"]), ("run_command", {"command": "echo 1"}))
        self.assertEqual(a["content"][0]["text"], "Looking.")
        msgs += [{"role": "assistant", "content": a["content"]},
                 {"role": "user", "content": [{"type": "tool_result", "tool_use_id": use["id"], "content": "1"}]}]
        b = json.loads(self.msg(msgs, False))
        self.assertEqual([x["name"] for x in b["content"]], ["run_command", "other"])
        msgs += [{"role": "assistant", "content": b["content"]},
                 {"role": "user", "content": [{"type": "tool_result", "tool_use_id": x["id"], "content": "ok"} for x in b["content"]]}]
        c = json.loads(self.msg(msgs, False))
        self.assertEqual((c["stop_reason"], c["content"][0]["text"]), ("end_turn", "All done."))

    def test_stream_events(self):
        raw = self.msg([{"role": "user", "content": "do calls"}], True)
        events = [l[7:] for l in raw.splitlines() if l.startswith("event: ")]
        self.assertEqual(events[0], "message_start")
        self.assertEqual(events[-1], "message_stop")
        data = sse(raw)
        text = "".join(d["delta"]["text"] for d in data if d["type"] == "content_block_delta" and d["delta"]["type"] == "text_delta")
        self.assertEqual(text, "Looking.")
        args = "".join(d["delta"]["partial_json"] for d in data if d["type"] == "content_block_delta" and d["delta"]["type"] == "input_json_delta")
        self.assertEqual(json.loads(args), {"command": "echo 1"})
        self.assertEqual([d for d in data if d["type"] == "message_delta"][0]["delta"]["stop_reason"], "tool_use")


class Ollama(Base):
    def chat(self, msgs, stream):
        return self.req("/api/chat", {"model": "m", "messages": msgs, "stream": stream})[1]

    def test_tags(self):
        self.assertEqual(self.jreq("/api/tags")["models"][0]["name"], "scripted-model")

    def test_whole_tool_calls_round_trip(self):
        msgs = [{"role": "user", "content": "do calls"}]
        a = json.loads(self.chat(msgs, False))
        self.assertTrue(a["done"])
        self.assertEqual(a["message"]["tool_calls"][0]["function"], {"name": "run_command", "arguments": {"command": "echo 1"}})
        msgs += [a["message"], {"role": "tool", "content": "1"}]
        b = json.loads(self.chat(msgs, False))
        self.assertEqual(len(b["message"]["tool_calls"]), 2)
        msgs += [b["message"], {"role": "tool", "content": "ok"}, {"role": "tool", "content": "ok"}]
        self.assertEqual(json.loads(self.chat(msgs, False))["message"]["content"], "All done.")

    def test_streams_by_default_as_ndjson(self):
        raw = self.req("/api/chat", {"model": "m", "messages": [{"role": "user", "content": "do calls"}]})[1]
        lines = [json.loads(l) for l in raw.splitlines()]
        self.assertTrue(lines[-1]["done"])
        self.assertEqual("".join(l["message"]["content"] for l in lines), "Looking.")
        self.assertTrue(any(l["message"].get("tool_calls") for l in lines))
        self.assertFalse(any(l["done"] for l in lines[:-1]))


class Stepping(Base):
    def chat(self, msgs):
        return json.loads(self.req("/v1/chat/completions", {"messages": msgs})[1])["choices"][0]["message"]

    def test_two_conversations_do_not_interfere(self):
        a = [{"role": "user", "content": "do calls"}]
        b = [{"role": "user", "content": "do calls"}]
        a1 = self.chat(a)
        a += [a1, {"role": "tool", "tool_call_id": "x", "content": "1"}]
        self.assertEqual(self.chat(b)["tool_calls"][0]["function"]["arguments"], '{"command":"echo 1"}')  # b is still at step 0
        a2 = self.chat(a)
        self.assertEqual(len(a2["tool_calls"]), 2)  # a moved on to step 1
        self.assertEqual(self.chat(b)["tool_calls"][0]["function"]["arguments"], '{"command":"echo 1"}')

    def test_two_conversations_at_once_from_threads(self):
        results = {}

        def run(name):
            msgs = [{"role": "user", "content": "do calls"}]
            for _ in range(2):
                m = self.chat(msgs)
                msgs += [m, {"role": "tool", "tool_call_id": "x", "content": "ok"}]
            results[name] = self.chat(msgs)["content"]
        ts = [threading.Thread(target=run, args=(n,)) for n in "ab"]
        [t.start() for t in ts]
        [t.join() for t in ts]
        self.assertEqual(results, {"a": "All done.", "b": "All done."})

    def test_a_new_user_message_restarts_the_script(self):
        msgs = [{"role": "user", "content": "do calls"}]
        m = self.chat(msgs)
        msgs += [m, {"role": "tool", "tool_call_id": "x", "content": "1"}, {"role": "assistant", "content": "ok"},
                 {"role": "user", "content": "do calls again"}]
        self.assertEqual(self.chat(msgs)["content"], "Looking.")

    def test_log_line_has_no_request_content(self):
        secret = "zz-secret-prompt-text-qq"
        self.chat([{"role": "user", "content": secret}])
        log = open(self.logpath).read()
        self.assertNotIn(secret, log)
        last = log.strip().splitlines()[-1]
        for field in ("api=openai", "path=/v1/chat/completions", "stream=false", "rule=default", "step=0"):
            self.assertIn(field, last)

    def test_bad_request_is_a_400_without_echo(self):
        with self.assertRaises(urllib.error.HTTPError) as cm:
            self.req("/v1/chat/completions", {"nope": "zz-echo-check"})
        self.assertEqual(cm.exception.code, 400)
        self.assertNotIn("zz-echo-check", cm.exception.read().decode())

    def test_binds_loopback_by_default(self):
        self.assertEqual(self.srv.server_address[0], "127.0.0.1")


class Shipped(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.sc = sm_scenario.Scenarios(sm_scenario.shipped_paths())

    def script(self, name):
        return [r for r in self.sc.rules if r.name == name][0].script

    def test_commands_20(self):
        s = self.script("commands-20")
        calls = [c for t in s for c in t["tool_calls"]]
        self.assertEqual(len(calls), 20)
        self.assertTrue(all(c["name"] == "run_command" and "command" in c["arguments"] for c in calls))
        self.assertTrue(s[-1]["text"] and not s[-1]["tool_calls"])

    def test_slow_task_paces_its_calls(self):
        s = self.script("slow-task")
        self.assertGreaterEqual(sum(1 for t in s if t["tool_calls"]), 5)
        self.assertTrue(all(t["delay_s"] >= 3 for t in s))

    def test_markdown_all_holds_every_element(self):
        text = self.script("markdown-all")[0]["text"]
        for needle in ("# ", "## ", "### ", "*emphasis*", "**strong**", "~~", "`inline code`", "```python", "```rust",
                       "   - nested", "     1. deep", "- [x]", "- [ ]", "| Name |", "|:----:|", "> ", "](http://", "](https://",
                       "](javascript:", "\n---\n"):
            self.assertIn(needle, text)
        self.assertGreater(len(text), 1500)

    def test_every_shipped_file_loads_and_has_a_rule_that_matches(self):
        for name in ("markdown-all", "commands-20", "slow-task"):
            self.assertEqual(self.sc.pick("run " + name, 0)[0], name)

    def test_no_file_is_over_600_lines(self):
        d = os.path.dirname(os.path.abspath(__file__))
        for f in os.listdir(d):
            if f.endswith((".py", ".json", ".md")):
                self.assertLessEqual(len(open(os.path.join(d, f)).read().splitlines()), 600, f)


if __name__ == "__main__":
    unittest.main()
