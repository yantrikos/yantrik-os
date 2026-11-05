"""How each answering mind shows what it did: the Mind in the journal, Hermes in ~/.hermes/state.db.

An adapter answers: is the mind idle, did a turn finish, what tools did it use (in order, with arguments and
results), and what refusals did it meet. Evidence is kept whole: nothing is filtered out of the lines kept for a
row (no `grep -v`), because a filter can hide the very refusal that explains a result."""
import json
import re
import sqlite3
import time

STEP = re.compile(r"\[agent\] step (\d+): (?:mcp\.yantrik-os\.)?([\w.\-]+) raw args (\{.*)")
RESULT = re.compile(r"\[agent\] step (\d+): (?:mcp\.yantrik-os\.)?([\w.\-]+) -> (.*)")
EVIDENCE_LINES = 300
LINE_CAP = 700


def _arg(raw, key):
    m = re.search(rf'"{key}":"([^"]*)"', raw)
    return m.group(1) if m else ""


class MindAdapter:
    id = "mind"
    answering_id = "mind"

    def __init__(self, machine):
        self.m = machine

    def journal(self, since_epoch=None, minutes=None):
        argv = ["journalctl", "-u", "yantrik-mind", "--no-pager", "--output=cat"]
        argv += ["--since", f"@{int(since_epoch)}"] if since_epoch else ["--since", f"-{minutes or 30} min"]
        return self.m.text(argv, timeout=60, person=False).splitlines()

    def readable(self):
        """(ok, why): the journal answers. A mind that is up writes something every few minutes ([dmn] lines)."""
        lines = self.journal(minutes=30)
        return (bool(lines), f"{len(lines)} journal lines in the last 30 minutes")

    def idle(self):
        lines = self.journal(minutes=120)
        done = [i for i, l in enumerate(lines) if "[agent] turn done" in l]
        started = [i for i, l in enumerate(lines)
                   if "[harness] turn " in l and not re.search(r"turn \d+: /(new|reset|clear)\s*$", l)]
        running = bool(started) and (not done or started[-1] > done[-1])
        steps = [l for l in self.journal(minutes=2) if "[agent] step" in l]
        return (not running) and not steps, f"running={running} steps_last_2min={len(steps)}"

    def turn_finished(self, since_epoch):
        return any("[agent] turn done" in l for l in self.journal(since_epoch))

    def calls(self, since_epoch, until_epoch=None):
        """[(tool, raw arguments, result)] between two epochs, in order. Lines are matched by position, not text."""
        lines = self.journal(since_epoch)
        if until_epoch:
            keep = len(lines) - len(self.journal(until_epoch))
            lines = lines[:keep]
        results = {}
        for l in lines:
            r = RESULT.search(l)
            if r:
                results[(r.group(1), r.group(2))] = r.group(3)
        out = []
        for l in lines:
            m = STEP.search(l)
            if m:
                out.append((m.group(2), m.group(3), results.get((m.group(1), m.group(2)))))
        return out

    def trace(self, since_epoch):
        lines = self.journal(since_epoch)
        tools, refusals = [], []
        for l in lines:
            m = STEP.search(l)
            if m:
                name = m.group(2)
                if name in ("os_act", "os_describe"):
                    name = f"{name}:{_arg(m.group(3), 'app')}.{_arg(m.group(3), 'action')}".rstrip(".")
                tools.append(name)
            if "REFUSED" in l:
                refusals.append(l[:LINE_CAP])
        return {"tools": tools, "refusals": refusals, "journal": [l[:LINE_CAP] for l in lines[:EVIDENCE_LINES]]}


class HermesAdapter:
    id = "hermes"
    answering_id = "hermes"
    DB = ".hermes/state.db"

    def __init__(self, machine):
        self.m = machine

    def _db(self):
        return sqlite3.connect(f"file:{self.m.home}/{self.DB}?mode=ro", uri=True)

    def readable(self):
        try:
            n = self._db().execute("select count(*) from messages").fetchone()[0]
            return n > 0, f"{n} messages in state.db"
        except sqlite3.Error as e:
            return False, f"state.db unreadable: {e}"

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

    def final_reply(self, since_epoch):
        """The last assistant message that made no tool call: Hermes's own answer, without its tool chatter."""
        try:
            r = self._db().execute("select content from messages where timestamp>=? and role='assistant' and (tool_calls is null or tool_calls='') order by id desc limit 1",
                                   (since_epoch,)).fetchone()
            return r[0] if r else None
        except sqlite3.Error:
            return None

    def asked_clarify(self, since_epoch):
        try:
            return bool(self._db().execute("select 1 from messages where timestamp>=? and role='assistant' and tool_calls like '%clarify%' limit 1",
                                           (since_epoch,)).fetchone())
        except sqlite3.Error:
            return False

    @staticmethod
    def _unwrap(call):
        """(name, arguments text). Hermes wraps most calls as tool_call {"calls": [{"name", "arguments"}]}: report the inner tool."""
        f = call.get("function", call)
        name, args = f.get("name"), f.get("arguments")
        text = args if isinstance(args, str) else json.dumps(args)
        if name == "tool_call":
            try:
                inner = json.loads(text)
                c0 = (inner.get("calls") or [{}])[0]
                a = c0.get("arguments", {})
                return (c0.get("name", "?").replace("mcp__yantrik_os__", "")
                        + (":" + str(a.get("app", "")) + "." + str(a.get("action", "")) if isinstance(a, dict) and a.get("app") else "")), text
            except (ValueError, AttributeError):
                pass
        return name, text

    def calls(self, since_epoch, until_epoch=None):
        out = []
        try:
            rows = self._db().execute("select id, tool_calls from messages where timestamp>=? and timestamp<? and role='assistant' and tool_calls is not null",
                                      (since_epoch, until_epoch or 1e12)).fetchall()
            tool_rows = self._db().execute("select tool_name, content from messages where timestamp>=? and timestamp<? and role='tool'",
                                           (since_epoch, until_epoch or 1e12)).fetchall()
        except sqlite3.Error:
            return out
        results = {}
        for tn, content in tool_rows:
            results.setdefault(tn, []).append(content)
        for _id, tc in rows:
            try:
                for call in json.loads(tc):
                    name, text = self._unwrap(call)
                    short = (name or "?").split(":")[0]
                    res = (results.get(short) or [None]).pop(0) if results.get(short) else None
                    out.append((name, text, res))
            except ValueError:
                out.append(("?", tc, None))
        return out

    def trace(self, since_epoch):
        tools, refusals, lines = [], [], []
        try:
            for role, tn, tc, content in self._db().execute(
                    "select role, tool_name, tool_calls, content from messages where timestamp>=? order by id", (since_epoch,)):
                lines.append(f"{role} {tn or ''} {(tc or content or '')[:LINE_CAP]}")
                if role == "assistant" and tc:
                    try:
                        tools += [self._unwrap(c)[0] for c in json.loads(tc)]
                    except ValueError:
                        tools.append("?")
                elif role == "tool" and content and "REFUSED" in content:
                    refusals.append(content[content.index("REFUSED"):][:LINE_CAP])
        except sqlite3.Error:
            pass
        return {"tools": tools, "refusals": refusals, "journal": lines[:EVIDENCE_LINES]}


def adapter_for(name, machine):
    return {"mind": MindAdapter, "hermes": HermesAdapter}[name](machine)
