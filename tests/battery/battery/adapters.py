"""How each answering mind shows what it did: the Mind in the journal, Hermes in ~/.hermes/state.db.

An adapter answers four questions: is the mind idle, did a turn finish, what tools did it use (in order),
and what refusals did it meet."""
import json
import re
import sqlite3
import time

STEP = re.compile(r"\[agent\] step (\d+): (?:mcp\.yantrik-os\.)?([\w.\-]+) raw args (\{.*)")


class MindAdapter:
    id = "mind"
    answering_id = "mind"

    def __init__(self, machine):
        self.m = machine

    def journal(self, since_epoch=None, minutes=None):
        argv = ["journalctl", "-u", "yantrik-mind", "--no-pager", "--output=cat"]
        argv += ["--since", f"@{int(since_epoch)}"] if since_epoch else ["--since", f"-{minutes or 30} min"]
        return self.m.text(argv, timeout=60, person=False).splitlines()

    def idle(self):
        lines = self.journal(minutes=30)
        turns = [l for l in lines if "[harness] turn " in l and "turn done" not in l]
        done = [i for i, l in enumerate(lines) if "[agent] turn done" in l]
        started = [i for i, l in enumerate(lines) if "[harness] turn " in l and not re.search(r"turn \d+: /(new|reset|clear)\s*$", l)]
        running = bool(started) and (not done or started[-1] > done[-1])
        recent = self.journal(minutes=2)
        steps = [l for l in recent if "[agent] step" in l]
        return (not running) and not steps, f"turns={len(turns)} running={running} steps_last_2min={len(steps)}"

    def turn_finished(self, since_epoch):
        lines = self.journal(since_epoch)
        return any("[agent] turn done" in l for l in lines)

    def calls(self, since_epoch, until_epoch=None):
        """[(tool, raw arguments text)] for every tool call between the epochs."""
        out = []
        lines = self.journal(since_epoch)
        if until_epoch:
            cut = set(self.journal(until_epoch))
            lines = [l for l in lines if l not in cut]
        for l in lines:
            m = STEP.search(l)
            if m:
                out.append((m.group(2), m.group(3)))
        return out

    def trace(self, since_epoch):
        lines = self.journal(since_epoch)
        tools, refusals = [], []
        for l in lines:
            m = STEP.search(l)
            if m:
                tools.append(m.group(2) if m.group(2) not in ("os_act", "os_describe") else f"{m.group(2)}:{_arg(m.group(3), 'app')}.{_arg(m.group(3), 'action')}".rstrip("."))
            if "REFUSED" in l:
                refusals.append(l[:400])
        return {"tools": tools, "refusals": refusals, "lines": [l[:500] for l in lines if "[harness] turn" in l or "turn done" in l]}


def _arg(raw, key):
    m = re.search(rf'"{key}":"([^"]*)"', raw)
    return m.group(1) if m else ""


class HermesAdapter:
    id = "hermes"
    answering_id = "hermes"
    DB = ".hermes/state.db"

    def __init__(self, machine):
        self.m = machine

    def _db(self):
        return sqlite3.connect(f"file:{self.m.home}/{self.DB}?mode=ro", uri=True)

    def last_write(self):
        try:
            return self._db().execute("select max(timestamp) from messages").fetchone()[0] or 0
        except sqlite3.Error:
            return 0

    def running_delegations(self, since):
        try:
            return self._db().execute(
                "select count(*) from async_delegations where state not in ('error','done','completed','failed','cancelled') and dispatched_at>=?",
                (since - 5,)).fetchone()[0]
        except sqlite3.Error:
            return 0

    def idle(self):
        st = self.m.shell_state()
        quiet = time.time() - self.last_write()
        return (st["status"] == "idle" and quiet > 20), f"status={st['status']} quiet_for={int(quiet)}s"

    def turn_finished(self, since_epoch):
        st = self.m.shell_state()
        conv = [c for c in st["conversation"] if c.get("role") == "assistant"]
        finished = bool(conv) and not conv[-1].get("streaming") and st["status"] == "idle"
        return finished and self.running_delegations(since_epoch) == 0 and time.time() - self.last_write() > 30

    def asked_clarify(self, since_epoch):
        try:
            r = self._db().execute("select tool_calls from messages where timestamp>=? and role='assistant' and tool_calls like '%clarify%' limit 1",
                                   (since_epoch,)).fetchone()
            return bool(r)
        except sqlite3.Error:
            return False

    def calls(self, since_epoch, until_epoch=None):
        out = []
        try:
            for (tc,) in self._db().execute("select tool_calls from messages where timestamp>=? and timestamp<? and role='assistant' and tool_calls is not null",
                                            (since_epoch, until_epoch or 1e12)):
                try:
                    for call in json.loads(tc):
                        f = call.get("function", call)
                        a = f.get("arguments")
                        out.append((f.get("name"), a if isinstance(a, str) else json.dumps(a)))
                except ValueError:
                    out.append(("?", tc))
        except sqlite3.Error:
            pass
        return out

    def trace(self, since_epoch):
        tools, refusals = [], []
        try:
            for role, tn, tc, content in self._db().execute(
                    "select role, tool_name, tool_calls, content from messages where timestamp>=? order by id", (since_epoch,)):
                if role == "assistant" and tc:
                    try:
                        for call in json.loads(tc):
                            f = call.get("function", call)
                            args = f.get("arguments")
                            name = f.get("name")
                            if name == "tool_call":
                                try:
                                    inner = json.loads(args) if isinstance(args, str) else args
                                    c0 = (inner.get("calls") or [{}])[0]
                                    a = c0.get("arguments", {})
                                    name = f"{c0.get('name', '?').replace('mcp__yantrik_os__', '')}:{a.get('app', '')}.{a.get('action', '')}".rstrip(".")
                                except (ValueError, AttributeError):
                                    pass
                            tools.append(name)
                    except ValueError:
                        tools.append("?")
                elif role == "tool" and content and "REFUSED" in content:
                    refusals.append(content[content.index("REFUSED"):][:400])
        except sqlite3.Error:
            pass
        return {"tools": tools, "refusals": refusals, "lines": []}


def adapter_for(name, machine):
    return {"mind": MindAdapter, "hermes": HermesAdapter}[name](machine)
