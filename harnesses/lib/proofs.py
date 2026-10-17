#!/usr/bin/env python3
"""The three proofs that a harness is installed, seen rather than assumed: proofs.py HARNESS [--wait S] [--json]

    Can it answer?            a real call through the model gateway, when the install wired the
                              model (#675); "model: its own" when it did not, which is not a failure
    Can it remember?          a temporary note written through the harness's own memory path,
                              recalled, and deleted: "memory · YantrikDB · recalled"
    Can it reach the desktop? an attach seen on the harness socket: the desktop lists the harness
                              as attached (`yos describe shell`, harnesses[].state)

A harness is never reported installed until all three pass (design/onboarding-one-model-one-
memory-2026-10-10.md, "The flow" 2.4). Every installer in lib/install ends with this, and the
Settings → Minds row and the onboarding summary call the same thing:

    python3 /opt/yantrik/share/harnesses/lib/proofs.py hermes --json

which prints one JSON document and exits 0 only when every proof passed:

    {"harness": "hermes", "passed": true,
     "proofs": [{"id": "answer", "question": "Can it answer?", "ok": true,
                 "status": "connected", "says": "connected · deepseek-v4-pro",
                 "fix": null}, ...]}

`status` is the word a program matches on (connected, own, recalled, paired, failed); `says` is
the line a person reads; `fix`, on a failure, is the one press that puts it right: {"label",
"action"} where action is `install` (run the row's Install again), `configure` (the row's own
setup), `ai_accounts` (Settings → AI accounts, where the person chooses what is shared with an
account) or `restart` (with the `command` to run). Nothing here prints a secret: the probe note is a
fixed sentence with a random number, and the model probe's own output is reduced to one line.

Stdlib only, and run with the image's Python, so it works before any harness has its own.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

import hermes_runtime

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

ANSWER, REMEMBER, DESKTOP = "Can it answer?", "Can it remember?", "Can it reach the desktop?"

# Where the baked memory lives (build-debian-iso.sh), as common.sh has it.
MEMORY_DIR = Path(os.environ.get("YANTRIK_MEMORY_DIR", "/opt/yantrik/memory"))
YOS = os.environ.get("YOS_BIN", "/opt/yantrik/bin/yos")

# The harnesses that reach YantrikDB through yantrikdb-mcp, and the one that has a provider of
# its own inside the harness program (Hermes). The Mind keeps its own memory (yantrik-memory).
MCP_MEMORY = ("pi", "deepseek", "openclaw")


def state_dir(harness: str) -> Path:
    """Where the install recorded its steps (common.sh, harness_state): fixed under ~/.local/state,
    because the harness manifests name the same place with `~`."""
    return Path.home() / ".local" / "state" / "yantrik" / "harnesses" / harness


def memory_db(harness: str) -> Path:
    base = os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local" / "share")
    return Path(base) / "yantrik" / "memory" / ("%s.db" % harness)


def row(id_: str, question: str, ok: bool, status: str, says: str,
        fix: Optional[Dict[str, str]] = None) -> Dict[str, Any]:
    return {"id": id_, "question": question, "ok": ok, "status": status, "says": says,
            "fix": None if ok else fix}


def one_line(text: str, limit: int = 160) -> str:
    text = " ".join(str(text or "").split())
    return text if len(text) <= limit else text[: limit - 1] + "…"


def restart_fix(harness: str) -> Dict[str, str]:
    if harness == "hermes":
        command = "systemctl --user restart hermes-gateway"
    elif harness == "mind":
        command = "systemctl start yantrik-mind.service"
    else:
        command = "systemctl --user restart yantrik-%s.service" % harness
    return {"label": "Restart it", "action": "restart", "command": command}


INSTALL_FIX = {"label": "Install again", "action": "install"}


# ── Can it answer? ──────────────────────────────────────────────────────────────────────────────

# Where #675's handoff writes each harness's gateway token (crates/yantrik-ui/src/provider_handoff):
# the harness's own config, its own way. Read only to make the one call below; never printed.
GATEWAY_DEFAULT = "http://127.0.0.1:7460/v1"


def gateway_url(asked: Optional[str] = None) -> str:
    """The gateway's address. YANTRIK_GATEWAY_URL is taken only when it is plain http to a loopback
    address with no user info: the call carries the harness's bearer token, and a token sent
    anywhere else works from the gateway's own machine (yantrik-mind-72; the Mind's own rule for
    its gateway address, E.PROV2 is_loopback_http). Anything else is ignored."""
    import ipaddress
    import urllib.parse

    asked = os.environ.get("YANTRIK_GATEWAY_URL") if asked is None else asked
    if not asked:
        return GATEWAY_DEFAULT
    try:
        u = urllib.parse.urlsplit(asked)
        host = ipaddress.ip_address((u.hostname or "").strip("[]"))
        if u.scheme == "http" and u.username is None and u.password is None and host.is_loopback and u.port:
            return asked
    except ValueError:
        pass
    print("proofs: YANTRIK_GATEWAY_URL is not plain http to a loopback address; using %s" % GATEWAY_DEFAULT,
          file=sys.stderr)
    return GATEWAY_DEFAULT


# Says which press fixes a refused call: sharing is the person's choice in AI accounts, not the
# harness's wiring (yantrik-mind-72).
SHARING_FIX = {"label": "Allow sharing in AI accounts", "action": "ai_accounts"}
TOKEN_FILES = {
    "hermes": (".hermes/config.yaml",),
    "pi": (".pi/agent/models.json",),
    "openclaw": (".openclaw/openclaw.json",),
    "deepseek": (".config/yantrik/deepseek.json",),
}


def gateway_token(harness: str) -> str:
    """The `ygw-` token #675 wrote into the harness's own config, or "" when there is none."""
    import re

    for name in TOKEN_FILES.get(harness, ()):
        try:
            text = (Path.home() / name).read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        found = re.search(r"ygw-[0-9a-f]{64}", text)
        if found:
            return found.group(0)
    return ""


def gateway_call(token: str, timeout: float = 90.0) -> Dict[str, Any]:
    """One short, real chat call through the gateway as the harness: `model: picked`, so it is
    the model the person picked that answers. Returns {"model"} or {"error"}."""
    import urllib.error
    import urllib.request

    body = json.dumps({"model": "picked", "stream": False, "max_tokens": 8,
                       "messages": [{"role": "user", "content": "Reply with the single word OK."}]})
    req = urllib.request.Request(gateway_url().rstrip("/") + "/chat/completions", data=body.encode(),
                                 headers={"Authorization": "Bearer " + token,
                                          "Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as reply:
            answer = json.loads(reply.read(256 * 1024) or b"{}")
    except urllib.error.HTTPError as exc:
        try:
            err = json.loads(exc.read(64 * 1024) or b"{}").get("error") or {}
            message, code = err.get("message"), err.get("code")
        except (ValueError, AttributeError):
            message, code = None, None
        return {"error": message or "the gateway said %d" % exc.code, "code": code or ""}
    except (OSError, ValueError) as exc:
        return {"error": "the gateway did not answer (%s)" % one_line(exc, 80)}
    if not isinstance(answer, dict) or not answer.get("choices"):
        return {"error": "the gateway answered with no reply in it"}
    return {"model": str(answer.get("model") or "picked")}


def prove_answer(harness: str, run: Callable[..., subprocess.CompletedProcess] = subprocess.run,
                 call: Callable[[str], Dict[str, Any]] = gateway_call) -> Dict[str, Any]:
    """The model gateway, when the install wired it; otherwise the harness's own model, said so.

    The probe is one real call through the gateway (#675) with the gateway token the model step
    wrote into the harness's own config. An image whose model step ships a
    lib/install/model.sh with `yantrik_model_probe HARNESS` uses that instead.
    """
    try:
        wired = (state_dir(harness) / "model").read_text(encoding="utf-8").strip()
    except OSError:
        wired = ""
    if not wired:
        return row("answer", ANSWER, True, "own", "model: its own")
    fix = {"label": "Use Yantrik models", "action": "configure"}
    if wired != "picked":
        return row("answer", ANSWER, False, "failed",
                   "not connected: the model you picked was not wired in", fix)
    model_sh = HERE / "install" / "model.sh"
    if model_sh.is_file():
        try:
            done = run(["sh", "-c", '. "$1" && yantrik_model_probe "$2"', "sh", str(model_sh), harness],
                       capture_output=True, text=True, timeout=90, stdin=subprocess.DEVNULL)
        except (OSError, subprocess.TimeoutExpired) as exc:
            return row("answer", ANSWER, False, "failed", "not connected: %s" % one_line(exc), fix)
        if done.returncode != 0:
            said = (done.stderr or done.stdout or "").strip().splitlines()
            reason = one_line(said[-1]) if said else "the gateway did not answer"
            return row("answer", ANSWER, False, "failed", "not connected: %s" % reason, fix)
        model = one_line((done.stdout or "").strip().splitlines()[0] if done.stdout.strip() else "picked", 80)
        return row("answer", ANSWER, True, "connected", "connected · %s" % model)
    token = gateway_token(harness)
    if not token:
        return row("answer", ANSWER, False, "failed",
                   "not connected: its config holds no gateway token", fix)
    answer = call(token)
    if "error" in answer:
        # Refused because the person's picked account is not shared with: the press that fixes it
        # is in AI accounts, not the harness's wiring.
        refused = answer.get("code") == "private_context_not_shared"
        return row("answer", ANSWER, False, "failed", "not connected: %s" % one_line(answer["error"]),
                   SHARING_FIX if refused else fix)
    # One short real call (8 tokens) to the account the person picked; the row says so.
    return row("answer", ANSWER, True, "connected",
               "connected · %s · checked with one short call to your account" % one_line(answer["model"], 80))


# ── Can it remember? ────────────────────────────────────────────────────────────────────────────

def probe_note() -> str:
    return "Yantrik install check %s: this note is written, recalled and deleted at once." % uuid.uuid4().hex[:12]


def remember_through_mcp(harness: str, command: Optional[List[str]] = None,
                         env: Optional[Dict[str, str]] = None) -> Dict[str, Any]:
    """Write, recall and delete one note through yantrikdb-mcp with the harness's own settings,
    the same server and the same file the harness's memory tools use."""
    from yantrik_harness import McpTools, private_memory_dir

    command = command or [str(MEMORY_DIR / "bin" / "yantrikdb-mcp")]
    if not Path(command[0]).exists():
        return row("remember", REMEMBER, False, "failed",
                   "no memory: YantrikDB is not in this image (%s)" % command[0], INSTALL_FIX)
    if env is None:
        private_memory_dir(harness)
    env = dict(env or mcp_env(harness))
    tools = McpTools("Yantrik install check", "1", command=command, env=env, timeout=60,
                     log=lambda _m: None)
    note = probe_note()
    rid = ""
    try:
        text, error = tools.call("remember", {"text": note, "importance": 0.05, "source": harness,
                                              "metadata": {"provenance": harness, "probe": True}})
        if error:
            return row("remember", REMEMBER, False, "failed", "could not write: %s" % one_line(text),
                       INSTALL_FIX)
        try:
            rid = json.loads(text).get("rid") or ""
        except (ValueError, AttributeError):
            rid = ""
        found, error = tools.call("recall", {"query": note, "top_k": 5})
        recalled = (not error) and bool(rid) and rid in found
    finally:
        # The note is deleted however the recall went, a failed or interrupted one included.
        try:
            if rid:
                tools.call("forget", {"rid": rid})
        finally:
            tools.close()
    if not recalled:
        return row("remember", REMEMBER, False, "failed",
                   "wrote a note but could not recall it", INSTALL_FIX)
    return row("remember", REMEMBER, True, "recalled", "memory · YantrikDB · recalled")


def mcp_env(harness: str) -> Dict[str, str]:
    """What every yantrikdb-mcp started for a harness is given: its own file, the engine's bundled
    embedder (offline, nothing fetched), and the tools a mind needs rather than the full set."""
    return {"YANTRIKDB_DB_PATH": str(memory_db(harness)), "YANTRIKDB_EMBEDDER": "bundled"}


HERMES_PROBE = r"""
import json, os, sys
# Where the install put the provider (hermes_runtime.VENDOR), last, as the shim Hermes loads does.
sys.path.append(os.path.expanduser("~/.hermes/plugins/yantrikdb/vendor"))
env = os.path.expanduser(os.environ.get("HERMES_HOME", "~/.hermes")) + "/.env"
try:
    for line in open(env, encoding="utf-8"):
        key, sep, value = line.strip().partition("=")
        if sep and key.startswith("YANTRIKDB_"):
            os.environ[key] = value.strip().strip('"').strip("'")
except OSError:
    pass
from yantrikdb_hermes_plugin.client import YantrikDBConfig
from yantrikdb_hermes_plugin.embedded import make_backend
config = YantrikDBConfig.from_env()
client = make_backend(config)
note = sys.argv[1]
rid = client.remember(note, importance=0.05, metadata={"source": "hermes", "probe": True}).get("rid") or ""
try:
    found = json.dumps(client.recall(note, top_k=5), default=str)
finally:
    if rid:
        client.forget(rid)
print(json.dumps({"mode": config.mode, "recalled": bool(rid) and rid in found}))
"""


def hermes_python() -> Optional[str]:
    """The Python Hermes runs on now (hermes_runtime.py, which the install asks too)."""
    return hermes_runtime.python()


def remember_through_hermes(run: Callable[..., subprocess.CompletedProcess] = subprocess.run
                            ) -> Dict[str, Any]:
    """Hermes's own word first (`hermes memory status`), then a note through the provider's own
    backend, built from Hermes's own settings, in Hermes's own Python."""
    hermes = str(Path.home() / ".local" / "bin" / "hermes")
    try:
        status = run([hermes, "memory", "status"], capture_output=True, text=True, timeout=60,
                     stdin=subprocess.DEVNULL)
        said = (status.stdout or "") + (status.stderr or "")
    except (OSError, subprocess.TimeoutExpired) as exc:
        return row("remember", REMEMBER, False, "failed", "Hermes did not answer: %s" % one_line(exc),
                   INSTALL_FIX)
    lowered = said.lower()
    if "yantrikdb" not in lowered or "available" not in lowered or "not available" in lowered:
        return row("remember", REMEMBER, False, "failed",
                   "Hermes's memory is not YantrikDB, or not available", INSTALL_FIX)
    python = hermes_python()
    if not python:
        return row("remember", REMEMBER, False, "failed", "cannot find the Python Hermes runs on",
                   INSTALL_FIX)
    try:
        done = run([python, "-c", HERMES_PROBE, probe_note()], capture_output=True, text=True,
                   timeout=120, stdin=subprocess.DEVNULL)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return row("remember", REMEMBER, False, "failed", "could not write: %s" % one_line(exc),
                   INSTALL_FIX)
    try:
        answer = json.loads((done.stdout or "").strip().splitlines()[-1])
    except (ValueError, IndexError):
        answer = {}
    if done.returncode != 0 or not answer.get("recalled"):
        reason = one_line((done.stderr or "").strip().splitlines()[-1]) if (done.stderr or "").strip() \
            else "wrote a note but could not recall it"
        return row("remember", REMEMBER, False, "failed", reason, INSTALL_FIX)
    return row("remember", REMEMBER, True, "recalled", "memory · YantrikDB · recalled")


def harness_wiring(harness: str, run: Callable[..., subprocess.CompletedProcess] = subprocess.run
                   ) -> Optional[str]:
    """Why the harness itself would not use YantrikDB, or None. The note below goes through
    yantrikdb-mcp directly, so on its own it proves the memory works, not that this harness uses it:
    VM 520 showed OpenClaw with no yantrikdb MCP server while the note was recalled."""
    if harness == "openclaw":
        openclaw = str(Path.home() / ".local" / "bin" / "openclaw")
        try:
            done = run([openclaw, "mcp", "show", "yantrikdb", "--json"], capture_output=True, text=True,
                       timeout=60, stdin=subprocess.DEVNULL)
            server = json.loads(done.stdout or "{}") if done.returncode == 0 else {}
        except (OSError, ValueError, subprocess.TimeoutExpired):
            server = {}
        if not isinstance(server, dict) or not server.get("command"):
            return "OpenClaw has no YantrikDB MCP server"
        if (server.get("env") or {}).get("YANTRIKDB_DB_PATH") != str(memory_db(harness)):
            return "OpenClaw's YantrikDB MCP server points at another file"
        return None
    if harness in ("pi", "deepseek"):
        # On unless the harness's config says otherwise (yantrik_harness.memory_on); whether the
        # image carries yantrikdb-mcp is the note's own first question below.
        path = Path.home() / ".config" / "yantrik" / ("%s.json" % harness)
        try:
            setting = json.loads(path.read_text(encoding="utf-8")).get("memory")
        except (OSError, ValueError, AttributeError):
            setting = None
        if setting is False or (isinstance(setting, str) and setting.strip().lower() in ("off", "none", "false", "own")):
            return "YantrikDB is turned off in %s" % path.name
    return None


def prove_remember(harness: str) -> Dict[str, Any]:
    if harness == "hermes":
        return remember_through_hermes()
    if harness in MCP_MEMORY:
        reason = harness_wiring(harness)
        if reason:
            return row("remember", REMEMBER, False, "failed", "no memory: %s" % reason, INSTALL_FIX)
        return remember_through_mcp(harness)
    if harness == "mind":
        # The Mind's memory is the person's YantrikDB itself, kept by yantrik-memory under the
        # Mind's own account; this process may not open it, and must not.
        return row("remember", REMEMBER, True, "own", "memory · YantrikDB (the Mind's own)")
    return row("remember", REMEMBER, False, "failed", "no memory path for %s" % harness, INSTALL_FIX)


# ── Can it reach the desktop? ───────────────────────────────────────────────────────────────────

def desktop_state(harness: str, run: Callable[..., subprocess.CompletedProcess] = subprocess.run
                  ) -> str:
    """The desktop's own word for the harness: `attached`, `answering`, or whatever else it is.

    The desktop lists a harness as attached only once it has attached on the harness socket,
    whose peer credentials the host checked at accept; nothing a harness says makes it so."""
    try:
        done = run([YOS, "describe", "shell", "--json"], capture_output=True, text=True, timeout=20,
                   stdin=subprocess.DEVNULL)
        if done.returncode != 0:
            return "no desktop"
        view = json.loads(done.stdout or "{}")
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return "no desktop"
    state = (view.get("state") or {}) if isinstance(view, dict) else {}
    if state.get("locked"):
        # A locked desktop answers describe with nothing in it, harnesses included, so "not
        # listed" would send the person after the harness when it is the desktop that is waiting.
        return "locked"
    rows = state.get("harnesses") or []
    for entry in rows:
        if isinstance(entry, dict) and entry.get("id") == harness:
            return str(entry.get("state") or "")
    return "not listed"


def prove_desktop(harness: str, wait: float,
                  state: Callable[[str], str] = desktop_state,
                  sleep: Callable[[float], None] = time.sleep) -> Dict[str, Any]:
    end = time.monotonic() + max(wait, 0.0)
    seen = state(harness)
    while seen not in ("attached", "answering") and time.monotonic() < end:
        sleep(2.0)
        seen = state(harness)
    if seen in ("attached", "answering"):
        return row("desktop", DESKTOP, True, "paired", "paired")
    if seen == "locked":
        return row("desktop", DESKTOP, False, "failed",
                   "not checked: the desktop is locked; unlock it, then check again", restart_fix(harness))
    return row("desktop", DESKTOP, False, "failed",
               "not paired: the desktop has not seen it attach (%s)" % one_line(seen, 60),
               restart_fix(harness))


# ── All three ───────────────────────────────────────────────────────────────────────────────────

def prove(harness: str, wait: float = 0.0, only: Optional[List[str]] = None) -> Dict[str, Any]:
    """The function the Settings row and the onboarding summary call."""
    wanted = only or ["answer", "remember", "desktop"]
    proofs = []
    if "answer" in wanted:
        proofs.append(prove_answer(harness))
    if "remember" in wanted:
        proofs.append(prove_remember(harness))
    if "desktop" in wanted:
        proofs.append(prove_desktop(harness, wait))
    return {"harness": harness, "passed": all(p["ok"] for p in proofs), "proofs": proofs}


def render(result: Dict[str, Any]) -> str:
    lines = []
    for p in result["proofs"]:
        mark = "✓" if p["ok"] else "✗"
        line = "%s %s %s" % (mark, p["question"], p["says"])
        if p["fix"]:
            line += "  →  %s" % p["fix"]["label"]
            if p["fix"].get("command"):
                line += " (%s)" % p["fix"]["command"]
        lines.append(line)
    return "\n".join(lines)


def keep_and_find(harness: str, remember: str = "", recall: str = "") -> Dict[str, Any]:
    """For a check that a fact outlives the process that wrote it (the ISO boot test, the 520
    check): `remember` writes TEXT through the harness's memory server and keeps it; `recall`, in
    a later process, asks for TEXT and says whether it came back. MCP-memory harnesses only."""
    from yantrik_harness import McpTools

    if harness not in MCP_MEMORY:
        return {"harness": harness, "error": "--remember/--recall reach yantrikdb-mcp harnesses only"}
    from yantrik_harness import private_memory_dir
    private_memory_dir(harness)
    tools = McpTools("Yantrik memory check", "1", command=[str(MEMORY_DIR / "bin" / "yantrikdb-mcp")],
                     env=mcp_env(harness), timeout=60, log=lambda _m: None)
    try:
        out: Dict[str, Any] = {"harness": harness}
        if remember:
            text, error = tools.call("remember", {"text": remember, "source": harness,
                                                  "metadata": {"provenance": harness}})
            out["remembered"] = not error
        if recall:
            text, error = tools.call("recall", {"query": recall, "top_k": 5})
            out["recalled"] = (not error) and recall.lower() in text.lower()
        return out
    finally:
        tools.close()


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("harness")
    parser.add_argument("--wait", type=float, default=0.0,
                        help="seconds to wait for the attach (an install that just restarted it)")
    parser.add_argument("--only", default="", help="comma-separated: answer,remember,desktop")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--remember", default="", metavar="TEXT",
                        help="write TEXT to the harness's memory and keep it (a boot test's fact)")
    parser.add_argument("--recall", default="", metavar="TEXT",
                        help="say whether TEXT comes back from the harness's memory")
    args = parser.parse_args(argv)
    if args.remember or args.recall:
        found = keep_and_find(args.harness, args.remember, args.recall)
        print(json.dumps(found), flush=True)
        return 0 if found.get("remembered", True) and found.get("recalled", True) and "error" not in found else 1
    only = [w for w in args.only.split(",") if w] or None
    result = prove(args.harness, args.wait, only)
    print(json.dumps(result) if args.json else render(result), flush=True)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
