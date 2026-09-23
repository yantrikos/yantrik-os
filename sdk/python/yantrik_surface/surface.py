"""A surface: what an app shows a mind, and what it lets one do, under the person's grades.

This file is a port of `yantrik-app-runtime::control`'s dispatch — the same order of checks,
the same sentences, the same error codes and envelopes — so a caller cannot tell a surface
built on this package from one built on the Rust runtime. The order, as `ControlRpc::dispatch`
and `Registry::act` run it:

  0. an empty `action`;
  1. `agent_token` lifted off the call and stripped out of `args`, and what it is asked of the
     shell (see `reach`): a token no live agent carries, or a shell that does not answer, ends
     the call here, whatever it asks for; a grant, if one rides along, spent through the shell
     once the reach, the arguments and the ceiling have passed (an unknown action is answered as
     unknown first, and nothing is spent on it);
  2. on the app's thread, in one turn: the unknown action; the calling agent's reach; the
     arguments (an object, none missing, none undeclared, each of its declared
     type — the `yantrik-surface` crate's checks and sentences); the ceiling and the mode (see
     `gate`); STALE, when
     `expect_revision` is not the revision the app is at; the handler, given the declared
     defaults for what was left out; and the view read back, so the answer carries the world
     the action left.

What "the app's thread" is belongs to the app. By default every describe and act runs on the
connection's thread under one lock — the app's serialization domain, which is what makes the
revision guard atomic. An app whose state lives on a thread of its own (a GUI main loop,
Blender's main thread) overrides `run_on_app_thread` to hop there and back.
"""

import inspect
import json
import math
import itertools
import os
import re
import signal
import sys
import threading
import types
import typing

from . import gate, reach, wire

PROTOCOL = 1
SETTLES = ("on return", "later")

# How long the app's thread gets before a caller is told it did not answer. The default
# runner is a lock, so this is how long a describe waits behind a long act; an app with a
# thread of its own sets its own per action.
DESCRIBE_TIMEOUT = 10.0
ACT_TIMEOUT = 30.0

_MISSING = inspect.Parameter.empty


class Refusal(Exception):
    """An action declining, in the app's own words.

    Raise it from a handler (or a view) with a sentence a person can read: what was asked,
    what is wrong with it, and what cannot be taken back. It is answered as -32602 with the
    sentence as it stands — the same answer a Rust handler's `Err(String)` gets.
    """


class NotAnswered(Exception):
    """Raised by a `run_on_app_thread` whose thread did not pick the work up in time.

    Answered as -32000 "app did not answer within Ns", the runtime's own words — a
    timeout is the app not answering, which is a different thing from the app refusing.
    """


class Later:
    """The rest of an answer, finished off the app's thread: `answer_later` in Python.

    Return `Later(work)` from a handler when the caller is owed the result of something slow
    (a command's exit code, a finished export) and the app's thread must not wait for it. The
    handler's turn ends at once; `work()` then runs on the caller's connection thread, and
    what it returns is the caller's `result` (a `Refusal` from it is the caller's refusal).
    The view is read again afterwards, so the state beside the result is the state the
    result came from. `work` must not touch anything only the app's thread may touch.
    """

    def __init__(self, work):
        if not callable(work):
            raise TypeError("Later needs a function to run")
        self.work = work


# ── who is calling, for the length of one dispatch ───────────────────────────

_context = threading.local()


def caller():
    """Who opened the socket the call being handled came in on — `PeerCred(pid, uid, gid)` as
    the kernel reported it — or None outside a dispatch or where the kernel would not say.
    A fact about the call, not a verdict: nothing in this package refuses on it."""
    return getattr(_context, "caller", None)


def agent_token():
    """The agent token the call being handled carried beside its `args`, or None. Carried,
    never checked: what it is worth is the handler's business."""
    return getattr(_context, "token", None)


class _Scope:
    """Installs a dispatch's caller and token on the thread that runs it, and puts back what
    was there — a guard, so a handler that raises cannot leave the next dispatch reading the
    previous caller."""

    def __init__(self, peer, token):
        self.peer = peer
        self.token = token

    def __enter__(self):
        self.saved = (getattr(_context, "caller", None), getattr(_context, "token", None))
        _context.caller, _context.token = self.peer, self.token
        return self

    def __exit__(self, *exc):
        _context.caller, _context.token = self.saved
        return False


# ── the vocabulary: parameters and actions ───────────────────────────────────


# The parameter types, as the Rust contracts list them (`PARAM_TYPES`), and how a refusal says
# them. The sentences are the `yantrik-surface` crate's (`args.rs`): a refusal names the kind of
# value that arrived and never the value itself — a number a caller sends may be a PIN, a year
# of birth or a dose, and a refusal is shown, logged and handed back to a model.
PARAM_TYPES = ("string", "number", "integer", "boolean", "array", "object")
_SINGULAR = {"string": "a string", "number": "a number", "integer": "an integer",
             "boolean": "a boolean", "object": "an object", "array": "an array"}
_PLURAL = {"string": "strings", "number": "numbers", "integer": "integers",
           "boolean": "booleans", "object": "objects", "array": "arrays"}


def is_type(value, kind):
    """Whether a parsed JSON value is of type `kind`, as the dispatch reads it.

    `integer` is a number written without a fraction or exponent, in the range serde_json holds
    as one (`3`, not `3.0`) — stricter than JSON Schema on purpose, because a handler reading it
    as a whole number finds nothing in `3.0`. `number` is any number; `true` is neither.
    """
    if kind == "string":
        return isinstance(value, str)
    if kind == "boolean":
        return isinstance(value, bool)
    if kind == "integer":
        return (isinstance(value, int) and not isinstance(value, bool)
                and wire._I64_MIN <= value <= wire._U64_MAX)
    if kind == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if kind == "array":
        return isinstance(value, list)
    if kind == "object":
        return isinstance(value, dict)
    return False


def arrived(wanted, value):
    """What arrived, as a refusal says it: its kind, never its value."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "a boolean"
    if isinstance(value, (int, float)):
        return "a number with a fraction" if wanted == "integer" else "a number"
    if isinstance(value, str):
        return "a string"
    if isinstance(value, list):
        return "an array"
    if isinstance(value, dict):
        return "an object"
    return "a %s" % type(value).__name__


# A string that is exactly an integer, or a decimal: the only strings a number is read from.
# ASCII digits only — `str.isdigit` and `int()` accept other scripts' digits, `_` and spaces, and
# serde_json does not.
_INTEGER = re.compile(r"-?(?:0|[1-9][0-9]*)", re.ASCII)
_DECIMAL = re.compile(r"-?(?:0|[1-9][0-9]*)\.[0-9]+", re.ASCII)


def coerced(param, value):
    """What `value` becomes for `param` when it is not already of its type and converts to it
    without loss: `(True, converted)`, or `(False, None)` — `coerced` in the `yantrik-surface`
    crate, rule for rule (`args.rs`, "What is converted, and what never is"):

    * text (and an enum): an integer is its decimal digits; a number with a fraction is not text;
    * integer: a string that is exactly an integer (`-?(0|[1-9][0-9]*)`) within what serde_json
      holds as one;
    * number: that, or a string that is exactly a decimal (that, a `.` and one digit or more), as
      the nearest float;
    * boolean: `"true"` and `"false"`;
    * nothing into or inside an array or an object.
    """
    kind = param.type
    if kind == "string":
        if isinstance(value, int) and not isinstance(value, bool) and \
                wire._I64_MIN <= value <= wire._U64_MAX:
            return True, str(value)
        return False, None
    if kind in ("integer", "number") and isinstance(value, str):
        if _INTEGER.fullmatch(value):
            number = int(value)
            if (value.startswith("-") and number >= wire._I64_MIN) or \
                    (not value.startswith("-") and number <= wire._U64_MAX):
                return True, number
            return False, None
        if kind == "number" and _DECIMAL.fullmatch(value):
            number = float(value)
            if math.isfinite(number):
                return True, number
        return False, None
    if kind == "boolean" and isinstance(value, str) and value in ("true", "false"):
        return True, value == "true"
    return False, None


class Param:
    """One argument of an action: `control_surface::Param` in Python, with the richer types the
    `yantrik-surface` crate adds, published the way it publishes them.

    `type` is one of `PARAM_TYPES`: string, number, integer, boolean, array, object. `enum`
    lists the only values a `string` accepts (Rust's `Param::one_of`); `items` is the type of
    every element of an array; `default` is published, means the argument may be left out, and
    is what the handler gets when it is (or when it arrives as `null`). `optional=True` makes an
    argument optional without a default to publish.
    """

    def __init__(self, name, type="string", description="", optional=False, *, default=_MISSING,
                 enum=None, items=None):
        if not isinstance(name, str) or not name.strip():
            raise ValueError("a parameter needs a name")
        if name == gate.AGENT_TOKEN:
            raise ValueError("`agent_token` travels beside `args` on app.act, never among them; "
                             "read it in a handler with yantrik_surface.agent_token()")
        if type not in PARAM_TYPES:
            raise ValueError("argument `%s` is declared as `%s`, which is not a type this OS "
                             "defines (%s)" % (name, type, ", ".join(PARAM_TYPES)))
        if enum is not None:
            enum = list(enum)
            if type != "string" or not all(isinstance(v, str) for v in enum):
                raise ValueError("argument `%s` lists values that are not all text; only a string "
                                 "can be one of a list" % name)
            if not enum:
                raise ValueError("argument `%s` lists no values, so nothing could be given" % name)
        if items is not None:
            if type != "array":
                raise ValueError("argument `%s` declares an item type but is declared `%s`, not "
                                 "an array" % (name, type))
            if items not in PARAM_TYPES:
                raise ValueError("argument `%s` holds `%s` items, which is not a type this OS "
                                 "defines (%s)" % (name, items, ", ".join(PARAM_TYPES)))
        self.name = name
        self.type = type
        self.description = description or ""
        self.enum = enum
        self.items = items
        self.default = _MISSING if default is _MISSING else wire.jsonable(
            default, "the default of `%s`" % name)
        self.optional = bool(optional) or default is not _MISSING
        if self.default is not _MISSING:
            wrong = self.mismatch(None, self.default)
            if wrong is not None:
                raise ValueError("the default is wrong: %s" % wrong)

    @property
    def required(self):
        return not self.optional

    @property
    def kind(self):
        """The Rust name for `type`."""
        return self.type

    def schema(self):
        """The published shape. Every parameter carries a description — the empty string when
        there is nothing to add, because an absent key has been read as a missing field."""
        out = {"type": self.type, "description": self.description}
        if self.enum is not None:
            out["enum"] = list(self.enum)
        if self.items is not None:
            out["items"] = {"type": self.items}
        if self.default is not _MISSING:
            out["default"] = self.default
        return out

    def wanted(self):
        """What this parameter takes, as a refusal says it."""
        if self.items is not None:
            return "an array of %s" % _PLURAL[self.items]
        return _SINGULAR[self.type]

    def accepts(self, action, value):
        """None if a caller's `value` is of this parameter's type or converts to it without loss
        (`coerced`); otherwise the refusal — `check_argument` in the crate. When it converts, the
        converted value is what is checked (an integer for an enum, against the list as its
        digits); when it does not, the refusal is the one for the value as it came."""
        refusal = self.mismatch(action, value)
        if refusal is None:
            return None
        converts, converted = coerced(self, value)
        return self.mismatch(action, converted) if converts else refusal

    def mismatch(self, action, value):
        """None if `value` is of this parameter's type, exactly; otherwise the refusal, in the
        crate's sentences (`check_value` in `yantrik-surface`). `action` None words it for a
        default, which is held to the exact type: a default is the author's, not a caller's."""
        whose = "argument `%s`" % self.name if action is None else "`%s` argument `%s`" % (
            action, self.name)
        if not is_type(value, self.type):
            return "%s must be %s, and %s arrived" % (
                whose, self.wanted(), arrived(self.type, value))
        if self.enum is not None and value not in self.enum:
            return "%s must be one of %s, and another string arrived" % (
                whose, ", ".join("`%s`" % v for v in self.enum))
        if self.items is not None:
            for i, item in enumerate(value):
                if not is_type(item, self.items):
                    return "%s must be %s, and `%s[%d]` is %s" % (
                        whose, self.wanted(), self.name, i, arrived(self.items, item))
        return None


class Action:
    """One thing an app can be asked to do: its name, purpose, grade and arguments.

    `description` is the purpose — the line a person reads on an approval card and a mind
    reads in `describe` — so it says what the action does, in the app's own words, including
    what cannot be taken back. `grade` is how much it can cost: safe < standard < sensitive <
    dangerous. `settles="later"` says the action only starts the work (the answer then says
    `settled: false`, and a caller watches for the result instead of mistaking the call for
    it); `expected_seconds` says how long it usually takes to settle, so a caller can size its
    wait. `timeout` is how long the app's thread gets before the caller hears it did not
    answer; it is not published.
    """

    def __init__(self, name, description, grade="standard", params=(), *, settles="on return",
                 expected_seconds=None, timeout=None):
        if not isinstance(name, str) or not name.strip() or name != name.strip():
            raise ValueError("an action needs a name without surrounding spaces")
        if not isinstance(description, str):
            raise ValueError("`%s` needs a description: the sentence a person reads on the "
                             "approval card" % name)
        if gate.grade(grade) is None:
            raise ValueError("`%s` is not a level this OS defines (%s); grade `%s` as one of them"
                             % (grade, " < ".join(gate.LADDER), name))
        if settles not in SETTLES:
            raise ValueError("`%s` settles %r; an action settles \"on return\" or \"later\""
                             % (name, settles))
        # Whole seconds, as the Rust `Action::expected_seconds(u32)` publishes them.
        if expected_seconds is not None and (
                isinstance(expected_seconds, bool) or not isinstance(expected_seconds, int)
                or not 0 <= expected_seconds < (1 << 32)):
            raise ValueError("`%s` expects %r seconds; say a whole number of seconds, or nothing"
                             % (name, expected_seconds))
        params = list(params)
        seen = set()
        for p in params:
            if not isinstance(p, Param):
                raise TypeError("`%s` has a parameter that is not a Param: %r" % (name, p))
            if p.name in seen:
                raise ValueError("`%s` declares `%s` twice" % (name, p.name))
            seen.add(p.name)
        self.name = name
        self.description = description
        self.permission = grade
        self.params = params
        self.settles = settles
        self.expected_seconds = expected_seconds
        self.timeout = timeout

    @property
    def grade(self):
        return self.permission

    @grade.setter
    def grade(self, value):
        self.permission = value

    @property
    def deferred(self):
        """The Rust name: whether the handler only starts the work."""
        return self.settles == "later"

    def schema(self, permission=None):
        """The action as JSON Schema, so a caller can hand it to a model unmodified —
        `Action::schema()` key for key, and `expected_seconds` when the action declared one."""
        out = {
            "name": self.name,
            "description": self.description,
            "permission": permission or self.permission,
            "settles": self.settles,
            "parameters": {
                "type": "object",
                "properties": {p.name: p.schema() for p in self.params},
                "required": [p.name for p in self.params if p.required],
            },
        }
        if self.expected_seconds is not None:
            out["expected_seconds"] = self.expected_seconds
        return out


# ── parameters from a function's signature ───────────────────────────────────


_SIMPLE = {str: "string", bool: "boolean", int: "integer", float: "number",
           list: "array", tuple: "array", dict: "object"}


def _unwrap_optional(annotation):
    """`X | None` and `Optional[X]` → X; any other union is not a parameter type."""
    origin = typing.get_origin(annotation)
    if origin is typing.Union or origin is types.UnionType:
        members = [a for a in typing.get_args(annotation) if a is not type(None)]
        if len(members) == 1:
            return members[0]
        raise TypeError("a union of %s" % ", ".join(getattr(m, "__name__", str(m))
                                                    for m in members))
    return annotation


def _type_of(annotation, default, where):
    """(type, enum, items, description) for one annotation."""
    description = ""
    if annotation is _MISSING:
        if default is _MISSING or default is None:
            return "string", None, None, description
        annotation = type(default)
    # `Annotated[Optional[T], "..."]` and `Optional[Annotated[T, "..."]]` alike.
    for _ in range(2):
        if typing.get_origin(annotation) is typing.Annotated:
            base, *extras = typing.get_args(annotation)
            texts = [e for e in extras if isinstance(e, str)]
            description = description or (texts[0] if texts else "")
            annotation = base
        try:
            annotation = _unwrap_optional(annotation)
        except TypeError as e:
            raise TypeError("%s is typed as %s; a parameter has one type" % (where, e)) from None
    origin = typing.get_origin(annotation)
    if origin is typing.Literal:
        return "string", list(typing.get_args(annotation)), None, description
    if annotation in _SIMPLE:
        return _SIMPLE[annotation], None, None, description
    if origin in (list, tuple, typing.List, typing.Tuple):
        args = [a for a in typing.get_args(annotation) if a is not Ellipsis]
        items = None
        if args and len(set(args)) == 1 and args[0] in _SIMPLE:
            items = _SIMPLE[args[0]]
        return "array", None, items, description
    if origin in (dict, typing.Dict):
        return "object", None, None, description
    raise TypeError("%s is typed as %r, which a caller cannot send; use str, int, float, bool, "
                    "Literal[...], list[...] or dict" % (where, annotation))


def action_from_function(fn, name=None, *, grade="standard", settles="on return",
                         expected_seconds=None, description=None, params=None, timeout=None):
    """An `Action` read off a Python function: its parameters from the signature — types from
    the hints (str, int, float, bool, Literal[...] as an enum, list[...], dict; `Optional` is
    the type inside it), defaults published, `Annotated[T, "..."]` or `params={name: "..."}`
    for each one's description — and its purpose from `description=` or the first paragraph
    of the docstring."""
    name = name or fn.__name__
    if description is None:
        doc = inspect.getdoc(fn) or ""
        description = " ".join(doc.split("\n\n", 1)[0].split())
    if not description:
        raise TypeError("`%s` has no description: give the function a docstring or pass "
                        "description= — it is the sentence a person reads on the approval card "
                        "and a mind reads in describe" % name)
    notes = dict(params or {})
    try:
        hints = typing.get_type_hints(fn, include_extras=True)
    except Exception:  # noqa: BLE001 - an unresolvable hint is read as written
        hints = dict(getattr(fn, "__annotations__", {}))
    declared = []
    for p in inspect.signature(fn).parameters.values():
        where = "`%s` argument `%s`" % (name, p.name)
        if p.kind in (p.VAR_POSITIONAL, p.VAR_KEYWORD):
            raise TypeError("%s collects *%s; an action's arguments are named one by one, so a "
                            "caller can be told what they are" % (where, p.name))
        if p.kind is p.POSITIONAL_ONLY:
            raise TypeError("%s is positional-only; arguments arrive by name" % where)
        kind, enum, items, doc = _type_of(hints.get(p.name, _MISSING), p.default, where)
        text = notes.pop(p.name, doc)
        default = p.default
        if default is None:
            declared.append(Param(p.name, kind, text, optional=True, enum=enum, items=items))
        else:
            declared.append(Param(p.name, kind, text, default=default, enum=enum, items=items))
    if notes:
        raise TypeError("`%s` describes %s, which it does not take"
                        % (name, ", ".join("`%s`" % n for n in sorted(notes))))
    return Action(name, description, grade, declared, settles=settles,
                  expected_seconds=expected_seconds, timeout=timeout)


# ── the surface ──────────────────────────────────────────────────────────────


class Surface:
    """An app's control surface: `app.describe` and `app.act` on `app-<id>.sock`.

        s = Surface("hello", summary=lambda: "3 items")

        @s.view
        def state():
            return {"items": items}

        @s.action("add", grade="standard")
        def add(text: str, count: int = 1) -> dict:
            "Add an item to the list."
            ...

        s.serve()

    `summary` is the one line a person could read (text, or a function returning it); the
    view is the structured state. `aliases` are other names the surface answers to, linked
    beside its socket. `settings_path`, `mode_path`, `spend_grant` and `reach_of` exist for
    tests: they pin the ceiling and mode files and stand in for the shell's grant store and for
    the shell's answer about an agent token (`reach.standing_of`).
    """

    describe_timeout = DESCRIBE_TIMEOUT
    act_timeout = ACT_TIMEOUT

    def __init__(self, app_id, summary=None, *, aliases=(), settings_path=None, mode_path=None,
                 spend_grant=None, socket_path=None, reach_of=None):
        if not isinstance(app_id, str) or not app_id or any(
                c.isspace() or c in "/\0" for c in app_id):
            raise ValueError("an app id is one word of text with no slash, like `hello` or "
                             "`download-manager`")
        self.app_id = app_id
        self.service_id = "app-%s" % app_id
        self.aliases = tuple(aliases)
        self.actions = []
        self.server = None
        self._summary = summary
        self._view = None
        self._handlers = {}
        self._overrides = {}
        self._settings_path = settings_path
        self._mode_path = mode_path
        self._spend_grant = spend_grant
        self._reach_of = reach_of
        self._socket_path = socket_path
        self._lock = threading.RLock()
        self._ids = itertools.count(1)
        self._ids_lock = threading.Lock()

    # ── declaring ────────────────────────────────────────────────────────────

    def view(self, fn):
        """Decorator: the function that returns the app's state, a JSON object."""
        self._view = fn
        return fn

    def action(self, name=None, *, grade="standard", settles="on return", expected_seconds=None,
               description=None, params=None, timeout=None):
        """Decorator: publish a function as an action. Its arguments come from the signature;
        it is called with them by name, and what it returns is the answer's `result`."""
        def register(fn):
            spec = action_from_function(
                fn, name if isinstance(name, str) else None, grade=grade, settles=settles,
                expected_seconds=expected_seconds, description=description, params=params,
                timeout=timeout)
            self.add_action(spec, lambda args: fn(**args))
            return fn
        if callable(name):
            return register(name)
        return register

    def add_action(self, spec, handler):
        """Publish `spec` with a handler called with the arguments as one dict — the shape of
        the Rust runtime's `App::action(spec, |args| ...)`."""
        if not isinstance(spec, Action):
            raise TypeError("add_action takes an Action")
        if spec.name in self._handlers:
            raise ValueError("`%s` is published twice" % spec.name)
        self.actions.append(spec)
        self._handlers[spec.name] = handler
        return spec

    def regrade(self, name, grade):
        """Re-declare the grade an action is published at, while the app runs — for an action
        whose cost depends on configuration (a prompt sent to a hosted service is `sensitive`,
        to one on the LAN `standard`). Returns the grade now published; raises `Refusal` and
        leaves the grade alone on a typo, so a typo never quietly un-grades an action."""
        if gate.grade(grade) is None:
            raise Refusal("`%s` is not a level this OS defines (%s), so `%s` kept the grade it had"
                          % (grade, " < ".join(gate.LADDER), name))
        if self._find(name) is None:
            raise Refusal("this app has no action `%s`; it offers: %s"
                          % (name, ", ".join(a.name for a in self.actions)))
        self._overrides[name] = grade
        return grade

    def published_grade(self, name):
        """The grade `name` is published at right now, or None for an action this app does
        not have — which is not the same as harmless."""
        spec = self._find(name)
        return None if spec is None else self._grade(spec)

    def configured_ceiling(self):
        """The machine's ceiling as this surface reads it, per call."""
        return gate.configured_ceiling(self._settings_path)

    def configured_mode(self, now=None):
        """The mode as this surface reads it, per call: a `Mode(name, session_rules)`."""
        return gate.configured_mode(self._mode_path, now)

    def _find(self, name):
        for spec in self.actions:
            if spec.name == name:
                return spec
        return None

    def _grade(self, spec):
        return self._overrides.get(spec.name, spec.permission)

    def _unknown(self, name):
        return "unknown action `%s`; this app offers: %s" % (
            name, ", ".join(a.name for a in self.actions))

    # ── the app's side, which an app may override ────────────────────────────

    def snapshot(self):
        """`(summary, state)` right now. Every describe, guard and answer reads through this,
        so a revision is never computed from a different read than the state beside it."""
        if self._view is None and self._summary is None:
            return "%s (no description published)" % self.app_id, {}
        state = self._view() if self._view is not None else {}
        summary = self._summary() if callable(self._summary) else self._summary
        return (self.app_id if summary is None else str(summary)), state

    def run_on_app_thread(self, fn, timeout):
        """Run `fn` in the app's serialization domain and return what it returned.

        The default is a lock around the connection's own thread: one dispatch at a time, so
        nothing moves the state between the revision guard and the handler. An app whose state
        belongs to one thread overrides this to hand `fn` there and wait; if the thread does
        not pick it up within `timeout` seconds, raise `NotAnswered`.
        """
        if not self._lock.acquire(timeout=timeout):
            raise NotAnswered()
        try:
            return fn()
        finally:
            self._lock.release()

    # ── the wire handler ─────────────────────────────────────────────────────

    def handle(self, method, params):
        return self.handle_from(method, params, None)

    def handle_from(self, method, params, peer):
        if method == "app.describe":
            return self.describe_json(peer)
        if method == "app.act":
            return self.act(params, peer)
        raise wire.RpcError(
            wire.RPC_METHOD_NOT_FOUND,
            "unknown method `%s`; this app serves app.describe, app.act" % method)

    def _turn(self, fn, timeout):
        try:
            return self.run_on_app_thread(fn, timeout)
        except NotAnswered:
            raise wire.RpcError(wire.RPC_TRANSPORT_ERROR,
                                "app did not answer within %ds" % int(timeout)) from None
        except Refusal as r:
            raise wire.RpcError(wire.RPC_INVALID_PARAMS, str(r)) from None

    def _read(self):
        summary, state = self.snapshot()
        state = wire.jsonable(state, "the state of %s" % self.app_id)
        summary = str(summary)
        return summary, state, wire.revision(summary, state)

    # ── describe ─────────────────────────────────────────────────────────────

    def describe_json(self, peer=None):
        """The reply to `app.describe`: `describe_json` in the Rust contracts, plus
        `protocol`, so a client can tell what it is talking to."""
        def read():
            with _Scope(peer, None):
                summary, state, revision = self._read()
                actions = [a.schema(self._grade(a)) for a in self.actions]
            return summary, state, revision, actions

        summary, state, revision, actions = self._turn(read, self.describe_timeout)
        return {
            "app": self.app_id,
            "protocol": PROTOCOL,
            "summary": summary,
            "state": state,
            "revision": revision,
            "actions": actions,
        }

    # ── act ──────────────────────────────────────────────────────────────────

    def _next_action_id(self):
        with self._ids_lock:
            return "%s#%d" % (self.service_id, next(self._ids))

    def act(self, params, peer=None):
        """The reply to `app.act`, or a `wire.RpcError` carrying the refusal."""
        params = params if isinstance(params, dict) else {}
        name = params.get("action")
        name = name.strip() if isinstance(name, str) else ""
        if not name:
            raise wire.RpcError(wire.RPC_INVALID_PARAMS, "act needs a non-empty `action`")
        # `args` left out is none given. Anything but an object is refused with the other
        # argument checks, before the gate, as the crate refuses it.
        args = params.get("args")
        if args is None:
            args = {}
        elif isinstance(args, dict):
            args = dict(args)
        # Lifted off before anything reads `args` — the grant below is bound to them.
        token = gate.agent_token_of(params, args)
        # A string, or no guard: what the transport reads with `as_str` (a client MUST send one).
        expect = params.get("expect_revision")
        expect = expect if isinstance(expect, str) else None
        grant = gate.grant_of(params)
        # What the token is, asked of the shell as the call is read (IO, beside the ceiling and
        # the mode): a role's reach to hold the call to, nothing for a live agent with no role,
        # or the refusal — a token no live agent carries, or a shell that did not answer — that
        # ends the call here, whatever it asks for. No token asks nothing.
        held = None
        if token is not None:
            held, refusal = reach.reach_of(token, self._reach_of)
            if refusal is not None:
                raise wire.RpcError(wire.RPC_INVALID_PARAMS, refusal)
        action_id = self._next_action_id()
        authority = gate.Authority(self.configured_ceiling(), self.configured_mode())

        spec = self._find(name)
        if grant:
            # Spent only once everything that could still refuse the call without asking anybody
            # has passed — the action exists, the agent's reach covers it, its arguments are
            # right, and the ceiling allows its grade (#154) — or a person's Allow is used up on
            # an act that never runs. Spent against the arguments as sent: what the card showed,
            # not what the handler will read.
            if spec is None:
                raise wire.RpcError(wire.RPC_INVALID_PARAMS, self._unknown(name))
            if held is not None:
                refusal = reach.within_call(held, self.app_id, name, self._grade(spec), args)
                if refusal is not None:
                    raise wire.RpcError(wire.RPC_INVALID_PARAMS, refusal)
            refusal = check_arguments(spec, args)
            if refusal is not None:
                raise wire.RpcError(wire.RPC_INVALID_PARAMS, refusal)
            refusal = authority.spend(grant, self.app_id, name, self._grade(spec), args,
                                      self._spend_grant)
            if refusal is not None:
                raise wire.RpcError(wire.RPC_INVALID_PARAMS, refusal)

        def turn():
            with _Scope(peer, token):
                return self._dispatch(name, args, expect, authority, held)

        timeout = (spec.timeout if spec is not None and spec.timeout else self.act_timeout)
        outcome = self._turn(turn, timeout)
        spec, result, later, summary, state, revision = outcome

        if later is not None:
            with _Scope(peer, token):
                try:
                    result = wire.jsonable(later.work(), "the answer of %s" % name)
                except Refusal as r:
                    raise wire.RpcError(wire.RPC_INVALID_PARAMS, str(r)) from None
            try:
                summary, state, revision = self._turn(self._read, timeout)
            except wire.RpcError:
                pass  # the result stands; the view is the one from when the handler ran

        return {
            "app": self.app_id,
            "action_id": action_id,
            "accepted": True,
            "settled": not spec.deferred,
            "result": result,
            "revision": revision,
            "summary": summary,
            "state": state,
        }

    def _dispatch(self, name, args, expect, authority, held=None):
        """One act, on the app's thread: every check, the handler, and the view read back."""
        spec = self._find(name)
        if spec is None:
            raise Refusal(self._unknown(name))

        # The calling agent's reach, on the grade this surface publishes now: a second rule
        # beside the gate's, which only ever takes away.
        if held is not None:
            refusal = reach.within_call(held, self.app_id, name, self._grade(spec), args)
            if refusal is not None:
                raise Refusal(refusal)

        # The arguments first: a malformed call is refused for what is wrong with it, before
        # anything about who may make it — as a grant is never spent on one.
        refusal = check_arguments(spec, args)
        if refusal is not None:
            raise Refusal(refusal)

        refusal = gate.decide(authority, self.app_id, name, self._grade(spec), spec.description)
        if refusal is not None:
            raise Refusal(refusal)

        if expect is not None:
            summary, _, current = self._read()
            if current != expect:
                raise Refusal("STALE: this app is at revision %s and you acted on %s. It now "
                              "reports: %s. Read it again before deciding."
                              % (current, expect, summary))

        # The handler reads what it declared: converted where the call converts, defaults in.
        result = self._handlers[name](as_declared(spec, args))
        later = result if isinstance(result, Later) else None
        if later is None:
            result = wire.jsonable(result, "the answer of %s" % name)
        summary, state, revision = self._read()
        return spec, (None if later else result), later, summary, state, revision

    # ── serving ──────────────────────────────────────────────────────────────

    def socket_path(self):
        """Where this surface binds: `app-<id>.sock` in the session's socket directory."""
        return self._socket_path or wire.default_socket_path(self.app_id)

    def serve_in_thread(self):
        """Bind the socket and answer on threads of its own; returns the `wire.Server`.

        Refuses with `wire.SocketBusy` (an `OSError`) when a live process already answers
        under this name, and replaces a dead socket left by a crash.
        """
        if self.server is not None:
            return self.server
        path = self.socket_path()
        directory = os.path.dirname(path)
        links = [os.path.join(directory, wire.socket_name("app-%s" % alias))
                 for alias in self.aliases if alias != self.app_id]
        server = wire.Server(path, self, links=links)
        server.start()
        self.server = server
        return server

    def serve(self):
        """Bind the socket and answer until interrupted (Ctrl-C or SIGTERM); unbind on the
        way out. The program's main loop: a socket that cannot be bound — another copy of
        this app already answers under its name, or the directory is not writable — ends the
        program with that sentence and exit status 1."""
        try:
            server = self.serve_in_thread()
        except OSError as e:
            print("[yantrik] %s is not serving: %s" % (self.app_id, e), file=sys.stderr)
            raise SystemExit(1) from None
        print("[yantrik] %s answering on %s (%d actions)"
              % (self.app_id, server.path, len(self.actions)), file=sys.stderr)
        stop = threading.Event()
        previous = None
        if threading.current_thread() is threading.main_thread():
            previous = signal.signal(signal.SIGTERM, lambda *_: stop.set())
        try:
            while not stop.wait(0.5):
                pass
        except KeyboardInterrupt:
            pass
        finally:
            if previous is not None:
                signal.signal(signal.SIGTERM, previous)
            self.stop()

    def stop(self):
        """Stop answering and take the socket (and its other names) away."""
        server, self.server = self.server, None
        if server is not None:
            server.stop()


def check_arguments(spec, args):
    """The checks every call's arguments meet before the handler runs — `check_arguments` in
    the `yantrik-surface` crate, in its order and its sentences: an object of named values
    (`null` is nothing given), every required argument present (the first missing, in
    declaration order), nothing the action does not take (the first in sorted order, as
    serde_json's map iterates), and each argument of its declared type or converting to it
    without loss (`coerced`; in declaration order; `null` for an optional one is the same as
    leaving it out). The refusal, or None. Checked before the gate and before any grant is spent."""
    name = spec.name
    if args is None:
        args = {}
    if not isinstance(args, dict):
        return "`%s` takes its arguments as an object of named values, and %s arrived" % (
            name, arrived("object", args))
    for p in spec.params:
        if p.required and p.name not in args:
            return "`%s` needs argument `%s`" % (name, p.name)
    known = {p.name for p in spec.params}
    for key in sorted(args):
        if key in known:
            continue
        if not spec.params:
            return "`%s` takes no arguments, but `%s` was given" % (name, key)
        return "`%s` has no argument `%s`; it takes: %s" % (
            name, key, ", ".join(p.name for p in spec.params))
    for p in spec.params:
        if p.name not in args:
            continue
        value = args[p.name]
        if value is None and not p.required:
            continue
        refusal = p.accepts(name, value)
        if refusal is not None:
            return refusal
    return None


def as_declared(spec, args):
    """The arguments as the handler receives them — `as_declared` in the crate: every argument
    converted to its declared type where it arrived as something that converts without loss, and
    every declared default filled in for one left out or sent as `null`. The call as sent is not
    changed: it is what a grant is bound to."""
    converted = dict(args or {})
    for p in spec.params:
        value = converted.get(p.name)
        if value is None or p.mismatch(None, value) is None:
            continue
        converts, value = coerced(p, value)
        if converts:
            converted[p.name] = value
    return with_defaults(spec, converted)


def with_defaults(spec, args):
    """The arguments as the handler receives them: what the caller sent, with every declared
    default filled in for an argument left out or sent as `null` — `with_defaults` in the crate."""
    filled = dict(args or {})
    for p in spec.params:
        if p.default is not _MISSING and filled.get(p.name) is None:
            filled[p.name] = p.default
    return filled
