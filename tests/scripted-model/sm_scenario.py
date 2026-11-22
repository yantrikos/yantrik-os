"""Scenario files: rules matched in order against the last user message, each with a script of turns.

A scenario file is JSON:

    {"rules": [{"name": "greeting", "match": {"contains": "hello"} | {"regex": "^hel+o"},
                "script": [{"text": "..."},
                           {"text": "optional lead-in", "tool_calls": [{"name": "run_command", "arguments": {...}}],
                            "delay_s": 2},
                           {"text": "after the tool results"}]}],
     "default": {"text": "..."}}

`match` may hold both keys; both must hold. A turn is one reply of the model: optional text, then
optional tool calls. The script advances once per assistant message already in the conversation's
history (see sm_history), so the position lives in the request, not in the server.
"""
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
SHIPPED_DIR = os.path.join(HERE, "scenarios")
DEFAULT_SENTENCE = "This is the scripted test model's fixed answer."
FINISHED_SENTENCE = "The scripted turns for this conversation are finished."


class ScenarioError(ValueError):
    pass


class Rule:
    def __init__(self, name, contains, regex, script):
        self.name = name
        self.contains = contains
        self.regex = re.compile(regex) if regex is not None else None
        self.script = script

    def matches(self, text):
        if self.contains is not None and self.contains not in text:
            return False
        if self.regex is not None and not self.regex.search(text):
            return False
        return True


def _turn(raw, where):
    if not isinstance(raw, dict):
        raise ScenarioError("%s: a turn must be an object" % where)
    calls = raw.get("tool_calls") or []
    for call in calls:
        if not isinstance(call, dict) or not isinstance(call.get("name"), str):
            raise ScenarioError("%s: a tool call needs a name" % where)
        if not isinstance(call.get("arguments", {}), dict):
            raise ScenarioError("%s: tool call arguments must be an object" % where)
    text = raw.get("text", "")
    if not isinstance(text, str):
        raise ScenarioError("%s: text must be a string" % where)
    if not text and not calls:
        raise ScenarioError("%s: a turn needs text or tool_calls" % where)
    return {"text": text, "tool_calls": [{"name": c["name"], "arguments": c.get("arguments", {})} for c in calls],
            "delay_s": float(raw.get("delay_s", 0))}


def _rule(raw, index, source):
    where = "%s: rule %d" % (source, index)
    m = raw.get("match") or {}
    if "contains" not in m and "regex" not in m:
        raise ScenarioError("%s: match needs contains or regex" % where)
    try:
        script = [_turn(t, "%s turn %d" % (where, i)) for i, t in enumerate(raw.get("script") or [])]
        rule = Rule(raw.get("name") or "rule-%d" % index, m.get("contains"), m.get("regex"), script)
    except re.error as e:
        raise ScenarioError("%s: bad regex (%s)" % (where, e))
    if not script:
        raise ScenarioError("%s: empty script" % where)
    return rule


def resolve(ref):
    """A path, or the name of a shipped scenario (`markdown-all`)."""
    if os.path.isfile(ref):
        return ref
    shipped = os.path.join(SHIPPED_DIR, ref if ref.endswith(".json") else ref + ".json")
    if os.path.isfile(shipped):
        return shipped
    raise ScenarioError("no scenario file or shipped scenario called %r" % ref)


def shipped_paths():
    return [os.path.join(SHIPPED_DIR, f) for f in sorted(os.listdir(SHIPPED_DIR)) if f.endswith(".json")]


class Scenarios:
    def __init__(self, paths):
        self.rules = []
        self.default = {"text": DEFAULT_SENTENCE, "tool_calls": [], "delay_s": 0.0}
        for path in paths:
            with open(path, encoding="utf-8") as f:
                try:
                    data = json.load(f)
                except ValueError as e:
                    raise ScenarioError("%s: not JSON (%s)" % (path, e))
            for i, raw in enumerate(data.get("rules") or []):
                self.rules.append(_rule(raw, i, os.path.basename(path)))
            if "default" in data:
                self.default = _turn(data["default"], os.path.basename(path) + ": default")

    def pick(self, user_text, step):
        """(rule name, turn) for this message and position. Past the script's end: one closing sentence."""
        for rule in self.rules:
            if rule.matches(user_text):
                if step < len(rule.script):
                    return rule.name, rule.script[step]
                return rule.name, {"text": FINISHED_SENTENCE, "tool_calls": [], "delay_s": 0.0}
        if step == 0:
            return "default", self.default
        return "default", {"text": FINISHED_SENTENCE, "tool_calls": [], "delay_s": 0.0}
