"""A capturing OpenAI-compatible upstream for measuring Pi (#1402): it refuses (413) a request over
`limit` tokens (the body at 4 bytes a token), stops an answer at the request's `max_tokens` with
`finish_reason: "length"` (a stub that answered whatever it was asked hid a Pi that asked for ONE
token), and answers a user message containing USE_TOOL with a call of `memory_recall`."""

import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ANSWER = "Noted. " + "This is a short answer about the topic. " * 8


class Upstream:
    def __init__(self, limit=8000):
        self.limit = limit
        self.log = []
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                raw = self.rfile.read(int(self.headers.get("content-length", 0)))
                outer.answer(self, json.loads(raw), -(-len(raw) // 4))

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = "http://127.0.0.1:%d/v1" % self.server.server_address[1]

    def stop(self):
        self.server.shutdown()
        self.server.server_close()

    def answer(self, h, body, tokens):
        messages = body.get("messages", [])
        text = json.dumps(messages).lower()
        summary = "context summarization assistant" in text or "<conversation>" in text
        asked = body.get("max_tokens") or body.get("max_completion_tokens")
        row = {"tokens": tokens, "max_tokens": asked, "summary": summary}
        self.log.append(row)
        if tokens > self.limit:
            row["status"] = 413
            out = json.dumps({"error": {"message": "Limit %d, Requested %d" % (self.limit, tokens), "code": "rate_limit_exceeded"}}).encode()
            h.send_response(413)
            h.send_header("content-type", "application/json")
            h.send_header("content-length", str(len(out)))
            h.end_headers()
            h.wfile.write(out)
            return
        row["status"] = 200
        last_user = next((m for m in reversed(messages) if m.get("role") == "user"), {})
        want_tool = (not summary and "USE_TOOL" in json.dumps(last_user) and messages[-1].get("role") == "user"
                     and any(t.get("function", {}).get("name") == "memory_recall" for t in body.get("tools") or []))
        reply = ANSWER
        finish = "stop"
        if asked is not None and len(reply) // 4 > asked:
            reply, finish = reply[: asked * 4], "length"
        row["finish"] = "tool_calls" if want_tool else finish
        delta = ({"role": "assistant", "content": None, "tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "memory_recall", "arguments": json.dumps({"query": "x"})}}]}
                 if want_tool else {"role": "assistant", "content": reply})
        h.send_response(200)
        h.send_header("content-type", "text/event-stream")
        h.end_headers()
        usage = {"prompt_tokens": tokens, "completion_tokens": max(1, len(reply) // 4), "total_tokens": tokens + len(reply) // 4}
        for chunk in ({"choices": [{"index": 0, "delta": delta, "finish_reason": None}]},
                      {"choices": [{"index": 0, "delta": {}, "finish_reason": row["finish"]}], "usage": usage}):
            h.wfile.write(("data: " + json.dumps(dict(chunk, id="x", object="chat.completion.chunk", model=body.get("model"))) + "\n\n").encode())
        h.wfile.write(b"data: [DONE]\n\n")
