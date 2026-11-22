#!/usr/bin/env python3
"""A deterministic scripted test model. FOR TEST VMs ONLY. See README.md.

Speaks the OpenAI-compatible API (/v1/models, /v1/chat/completions), the Anthropic Messages API
(/v1/models, /v1/messages) and Ollama's native API (/api/tags, /api/chat), streamed and whole,
including tool calls. Answers come from scenario files, never from a model.
No key is checked or needed, nothing is fetched, and the server binds 127.0.0.1 unless told otherwise.
"""
import argparse
import json
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import sm_history
import sm_render as R
import sm_scenario

MAX_BODY = 8 * 1024 * 1024
START = time.time()


def _tag(rule, step):
    return "%s-%d" % (re.sub(r"[^A-Za-z0-9]+", "-", rule), step)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    scenarios = None
    model = R.MODEL
    delay_scale = 1.0
    log_file = None
    log_lock = threading.Lock()

    # The server's own access log is off: it would print the client's request line.
    def log_message(self, fmt, *args):
        pass

    def _log(self, api, stream, rule, step):
        line = "%s api=%s path=%s stream=%s rule=%s step=%s" % (
            time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), api, self.path.split("?")[0], str(stream).lower(), rule, step)
        with self.log_lock:
            print(line, file=sys.stderr, flush=True)
            if self.log_file:
                with open(self.log_file, "a", encoding="utf-8") as f:
                    f.write(line + "\n")

    def _json(self, code, obj):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _error(self, code, message):
        self._json(code, {"error": {"type": "scripted_model_error", "message": message}})

    def _path(self):
        p = self.path.split("?")[0].rstrip("/")
        return p[3:] if p.startswith("/v1") else p  # `/v1/x` and `/x` are the same route

    def do_GET(self):
        p = self._path()
        anthropic = "anthropic-version" in self.headers or "x-api-key" in self.headers
        if p == "/models":
            self._log("anthropic" if anthropic else "openai", False, "-", "-")
            self._json(200, R.models_anthropic(self.model) if anthropic else R.models_openai(self.model))
        elif self.path.split("?")[0].rstrip("/") == "/api/tags":
            self._log("ollama", False, "-", "-")
            self._json(200, R.models_ollama(self.model))
        elif self.path.split("?")[0].rstrip("/") == "/api/version":
            self._log("ollama", False, "-", "-")
            self._json(200, {"version": "0.0.0-scripted"})
        elif self.path.split("?")[0].rstrip("/") in ("", "/health"):
            self._json(200, {"ok": True, "scripted_model": True, "uptime_s": int(time.time() - START)})
        else:
            self._error(404, "no such route")

    def do_POST(self):
        full = self.path.split("?")[0].rstrip("/")
        p = self._path()
        if full == "/api/chat":
            api, stream_default, whole, stream = "ollama", True, R.ollama_whole, R.ollama_stream
        elif p == "/chat/completions":
            api, stream_default, whole, stream = "openai", False, R.openai_whole, R.openai_stream
        elif p == "/messages":
            api, stream_default, whole, stream = "anthropic", False, R.anthropic_whole, R.anthropic_stream
        else:
            return self._error(404, "no such route")
        try:
            n = int(self.headers.get("Content-Length") or 0)
            if n > MAX_BODY:
                return self._error(413, "request too large")
            body = json.loads(self.rfile.read(n) or b"{}")
            text, step = sm_history.read(body)
        except (ValueError, AttributeError):
            return self._error(400, "the request is not a chat request of this API")
        want_stream = bool(body.get("stream", stream_default))
        rule, turn = self.scenarios.pick(text, step)
        self._log(api, want_stream, rule, step)
        if turn["delay_s"] and self.delay_scale:
            time.sleep(turn["delay_s"] * self.delay_scale)
        model = str(body.get("model") or self.model)[:100]
        tag = _tag(rule, step)
        if not want_stream:
            return self._json(200, whole(turn, tag, model))
        self.send_response(200)
        self.send_header("Content-Type", "application/x-ndjson" if api == "ollama" else "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        try:
            for chunk in stream(turn, tag, model):
                self.wfile.write(chunk)
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


def make_server(scenarios, host="127.0.0.1", port=0, model=R.MODEL, delay_scale=1.0, log_file=None):
    handler = type("H", (Handler,), {"scenarios": scenarios, "model": model, "delay_scale": delay_scale, "log_file": log_file})
    return ThreadingHTTPServer((host, port), handler)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Scripted test model (test VMs only).")
    ap.add_argument("--host", default="127.0.0.1", help="default 127.0.0.1")
    ap.add_argument("--port", type=int, default=7471)
    ap.add_argument("--scenario", action="append", default=[], help="a scenario file or the name of a shipped one; repeatable. Default: every shipped scenario")
    ap.add_argument("--model", default=R.MODEL, help="the model name it lists")
    ap.add_argument("--delay-scale", type=float, default=1.0, help="multiplies each turn's delay_s; 0 removes them")
    ap.add_argument("--log", default=None, help="also append the request lines to this file")
    ap.add_argument("--deadline", type=float, default=0, help="exit after this many seconds (0: run until stopped)")
    a = ap.parse_args(argv)
    try:
        paths = [sm_scenario.resolve(s) for s in a.scenario] or sm_scenario.shipped_paths()
        scenarios = sm_scenario.Scenarios(paths)
    except (sm_scenario.ScenarioError, OSError) as e:
        print("scripted-model: %s" % e, file=sys.stderr)
        return 2
    srv = make_server(scenarios, a.host, a.port, a.model, a.delay_scale, a.log)
    if a.deadline > 0:
        threading.Timer(a.deadline, srv.shutdown).start()
    print("scripted-model listening on %s:%d (test VMs only)" % srv.server_address[:2], file=sys.stderr, flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
