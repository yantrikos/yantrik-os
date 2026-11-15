#!/usr/bin/env python3
"""Stub of the GitHub org runners API for the controller test. Fake secrets only.
The mode file decides what the runner and its job look like:
  fast      garbage JIT config: the runner exits at once
  slow      realistic JIT config, no job
  busy      realistic config, busy, the job is ci.yml's test from this repo (admit)
  busyiso   busy, but the job belongs to iso.yml (refuse)
  busyfork  busy, ci.yml test, but a fork's head repository (refuse)
  busynone  busy, but no job of this runner can be found (refuse after 2 looks)
  busymany  as busy, but 100 large runs are in progress and ours is the last (M1)
  apidown   the runner endpoint answers 500 and so do the run lists (GitHub cannot be asked)
  apidownjob the runner endpoint answers 500 but the run list shows a job of this runner that is iso.yml's
  fastiso   the runner exits at once (a job shorter than a poll) and the newest run is iso.yml's
The group file: absent = normal; "wide" = an existing group with visibility all;
"many" = an existing group listing two repositories.
"""
import base64, json
from http.server import BaseHTTPRequestHandler, HTTPServer

import os
T = os.environ.get("STUB_DIR", "/root/.yantrik-ci-test") + "/"
FAKE_TOKEN = open(T + "token").read().strip()
ORG = "/orgs/yantrikos/actions"
REPO = "/repos/yantrikos/yantrik-os/actions"


def b64(s):
    return base64.b64encode(s.encode()).decode()


FILES = {
    ".runner": b64(json.dumps({"agentId": 4242, "agentName": "stub", "poolId": 1, "poolName": "Default",
                               "serverUrl": "http://127.0.0.1:1", "gitHubUrl": "http://127.0.0.1:1/yantrikos",
                               "workFolder": "_work", "useV2Flow": False, "isHostedServer": False})),
    ".credentials": b64(json.dumps({"scheme": "OAuth", "data": {"clientId": "STUBSECRETMARK-client",
                                    "authorizationUrl": "http://127.0.0.1:1/auth", "requireFipsCryptography": "False"}})),
    ".credentials_rsaparams": b64(json.dumps({k: "AAAA" for k in "d dp dq inverseQ modulus p q".split()} | {"exponent": "AQAB"})),
}
REALISTIC = b64(json.dumps(FILES))
GARBAGE = b64("not a runner config STUBSECRETMARK")
GROUPS = []
STATE = {"name": ""}


def rd(name, default):
    try:
        return open(T + name).read().strip()
    except OSError:
        return default


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _go(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(n) or b"{}") if n else {}
        ok = self.headers.get("Authorization") == "Bearer " + FAKE_TOKEN
        path = self.path.split("?")[0]
        print("stub: %s %s auth=%s" % (self.command, path, "ok" if ok else "BAD"), flush=True)
        if not ok:
            return self._send(401, {"message": "Bad credentials"})
        c, p, mode = self.command, path, rd("mode", "fast")
        if c == "GET" and p == ORG + "/runner-groups":
            g = rd("group", "")
            extra = {"wide": [{"id": 5, "name": "yantrik-ci", "visibility": "all"}],
                     "many": [{"id": 6, "name": "yantrik-ci", "visibility": "selected"}]}.get(g, [])
            return self._send(200, {"runner_groups": [{"id": 1, "name": "Default", "visibility": "all"}] + extra + GROUPS})
        if c == "POST" and p == ORG + "/runner-groups":
            print("stub: create group %s" % json.dumps({k: body.get(k) for k in ("name", "visibility", "selected_repository_ids", "allows_public_repositories", "restricted_to_workflows")}), flush=True)
            g = {"id": 77, "name": body["name"], "visibility": body["visibility"]}
            GROUPS.append(g)
            return self._send(201, g)
        if c == "GET" and p.startswith(ORG + "/runner-groups/") and p.endswith("/repositories"):
            repos = [{"full_name": "yantrikos/yantrik-os"}]
            if p.split("/")[-2] == "6":
                repos.append({"full_name": "yantrikos/other"})
            return self._send(200, {"total_count": len(repos), "repositories": repos})
        if c == "GET" and p == "/repos/yantrikos/yantrik-os":
            return self._send(200, {"id": 123456})
        if c == "POST" and p == ORG + "/runners/generate-jitconfig":
            STATE["name"] = body.get("name")
            print("stub: jit name=%s group=%s labels=%s work=%s" % (body.get("name"), body.get("runner_group_id"), body.get("labels"), body.get("work_folder")), flush=True)
            real = mode not in ("fast", "fastiso")
            return self._send(201, {"runner": {"id": 4242, "name": body.get("name")},
                                    "encoded_jit_config": REALISTIC if real else GARBAGE})
        if c == "GET" and p == ORG + "/runners/4242":
            if mode in ("apidown", "apidownjob"):
                return self._send(500, {"message": "stub: down"})
            return self._send(200, {"id": 4242, "busy": mode.startswith("busy"), "status": "online"})
        if c == "GET" and p == ORG + "/runners":
            return self._send(200, {"runners": [{"id": 9001, "name": "yantrik-ci-old", "status": "offline"},
                                                {"id": 9002, "name": "someone-else", "status": "offline"}]})
        if c == "DELETE" and p.startswith(ORG + "/runners/"):
            self.send_response(204); self.end_headers(); return
        if c == "GET" and p == REPO + "/runs":
            if mode == "apidown":
                return self._send(500, {"message": "stub: down"})
            iso = mode in ("busyiso", "fastiso", "apidownjob")
            run = {"id": 555, "path": ".github/workflows/%s@refs/heads/main" % ("iso.yml" if iso else "ci.yml"),
                   "event": "push", "head_repository": {"full_name": "evil/yantrik-os" if mode == "busyfork" else "yantrikos/yantrik-os"}}
            runs = [run]
            if mode == "busymany":
                # 100 runs of a realistic size (about 14 KB each: 1.4 MB in all), ours last.
                filler = [{"id": 1000 + i, "path": ".github/workflows/ci.yml@refs/heads/x", "event": "push",
                           "head_repository": {"full_name": "yantrikos/yantrik-os"}, "pad": "x" * 14000} for i in range(99)]
                runs = filler + [run]
            return self._send(200, {"workflow_runs": runs})
        if c == "GET" and p.startswith(REPO + "/runs/") and p.endswith("/jobs"):
            rid = p.split("/")[-2]
            if mode == "apidown":
                return self._send(500, {"message": "stub: down"})
            jobs = [] if (mode == "busynone" or rid != "555") else [{"name": "test", "runner_name": STATE["name"]}]
            return self._send(200, {"jobs": jobs})
        self._send(404, {"message": "Not Found"})

    do_GET = do_POST = do_DELETE = _go

    def _send(self, code, obj):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


with open(T + "jit-marker", "w") as f:
    f.write(GARBAGE + chr(10) + REALISTIC)
HTTPServer(("127.0.0.1", 8099), H).serve_forever()
