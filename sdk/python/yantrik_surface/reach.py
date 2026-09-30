"""An agent's reach, and a mind's standing: the two rules a call carrying an agent token meets
before anything else about it is decided.

A port of `yantrik_ipc_transport::reach` — the same rules, in the same order, with the same
sentences to the punctuation. If the two ever disagree, the Rust file is right and this one is
wrong.

**Reach.** A role from the agent catalog (Reviewer, Coder, Planner …) carries a reach: the
surfaces it may act on and a grade ceiling narrower than the machine's. Every `app.act` that
carries such an agent's token is held to it, before the handler runs and before any grant is
spent: an act outside the surfaces, or above the ceiling, is refused with `REACH:`. The shell
keeps the reach; a surface asks it, over `app-shell.sock`, by the token's SHA-256, and believes
only a process that is the desktop's own shell (`gate.must_be_the_shell`). No answer is an error:
the call carrying the token is refused rather than let through unheld (#189). No token, or a token
with no reach, is not held: this rule only ever narrows.

**Standing.** A caller the kernel says is the mind account (#411) acts only as an agent the shell
has attached, with the token its harness was given, and a token the shell knows is live. Without
one it would act unheld by any reach, as nobody in particular. Everyone else is unaffected.

Until this module, a Python surface held neither: a role's agent acting on Blender over the
person's socket was not held to its reach at all.
"""

import hashlib

from . import gate, mind_door, wire

# Within every reach: asking the person for an act, following that request up, spending the grant
# it came to, writing down an act that ran unasked, and reading one's own session.
ALWAYS = (
    "shell.request_approval",
    "shell.approval_status",
    "shell.consume_approval",
    "shell.record_unasked_action",
    "shell.read_agent",
)

# The few acts a mind account may make without standing, each on its own surface alone.
STANDING_NOT_NEEDED = (("shell", "memory_validate"),)

# How long a surface waits for the shell to say what a token may reach, as `REACH_ROUNDTRIP`.
REACH_ROUNDTRIP = 2.0

NEXT = ("Say in your answer what else needs doing; the person, or whoever handed you this, can "
        "do it.")


class Unanswered(Exception):
    """The shell could not be asked, or its answer was not one it gives. Never "no reach"."""


# What Rust's `str::trim` takes off: Unicode `White_Space`, and nothing else. Python's `strip()`
# also takes the separators U+001C–U+001F, so names are trimmed with this set instead.
RUST_WHITESPACE = ("\t\n\x0b\x0c\r \x85\xa0        "
                   "        　")
_ASCII_LOWER = str.maketrans("ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz")


def trim(text):
    """`str::trim`."""
    return text.strip(RUST_WHITESPACE)


def same_ascii_case(a, b):
    """`eq_ignore_ascii_case`: only A–Z fold, so no other letter can lower into a name."""
    return a.translate(_ASCII_LOWER) == b.translate(_ASCII_LOWER)


def token_digest(token):
    """The SHA-256 of a token, as lowercase hex, trimmed as the dispatch trims it."""
    return hashlib.sha256(trim(token).encode("utf-8")).hexdigest()


def _ask_shell(token, what):
    """`reach_of` on the shell, by the token's digest; the reply object. The token itself is never
    sent to anything but the process it was issued by."""
    path = wire.default_socket_path(gate.SHELL)
    try:
        return wire.call_once(path, "app.act", {
            "action": "reach_of",
            "args": {"token_sha256": token_digest(token)},
        }, timeout=REACH_ROUNDTRIP, peer_rule=gate.must_be_the_shell)
    except wire.PeerRefused as e:
        raise Unanswered("the shell did not say %s (%s)" % (what, e)) from e
    except (OSError, ConnectionError, ValueError) as e:
        raise Unanswered("the shell did not say %s (%s)" % (what, e)) from e


def reach_in_reply(reply):
    """The reach in the shell's answer to `reach_of`, as the socket carries it:
    `{"result": {..., "result": {"reach": ...}}}`. None for a token with no role; `Unanswered`
    for any other shape — never "no reach"."""
    result = _act_result(reply)
    if not isinstance(result, dict) or "reach" not in result:
        raise Unanswered("the shell's answer about this agent token's reach is not one it gives")
    reach = result["reach"]
    if reach is None:
        return None
    if not _is_reach(reach):
        raise Unanswered("the shell's answer about this agent token's reach does not read")
    held = {k: reach[k] for k in ("agent", "role", "name", "surfaces", "ceiling")}
    # An agent answering a turn from the person's phone asks above this, whatever the mode
    # (`gate::Authority::asks_above`). Dropping it let such a turn act unasked on every Python
    # surface (security review, 29 Sep 2026). Anything but a level on the ladder reads as asking
    # about everything.
    if reach.get("asks_above") is not None:
        level = reach["asks_above"]
        held["asks_above"] = level if isinstance(level, str) and gate.grade(level) is not None else "safe"
    return held


def known_in_reply(reply):
    """Whether the shell's answer to `reach_of` says the token is live. Any other shape is an
    error, never "yes"."""
    result = _act_result(reply)
    known = result.get("known") if isinstance(result, dict) else None
    if not isinstance(known, bool):
        raise Unanswered("the shell's answer about this agent token does not say whether it is live")
    return known


def _act_result(reply):
    """What the shell's `reach_of` answered: the JSON-RPC reply's `result` is the act's answer
    (`{app, action_id, accepted, settled, result}`), and its `result` is `{reach, known}`. The
    Rust client hands its helpers the act's answer, so they read one level less. Nothing else is
    read as an answer."""
    if not isinstance(reply, dict):
        return None
    if reply.get("error") is not None:
        error = reply["error"]
        message = error.get("message") if isinstance(error, dict) else None
        raise Unanswered(message if isinstance(message, str) else "the shell refused the question")
    act = reply.get("result")
    return act.get("result") if isinstance(act, dict) else None


def _is_reach(value):
    return (isinstance(value, dict)
            and all(isinstance(value.get(k), str) for k in ("agent", "role", "name", "ceiling"))
            and isinstance(value.get("surfaces"), list)
            and all(isinstance(s, str) for s in value["surfaces"]))


def reach_of(token, ask=None):
    """The reach `token` carries right now, as the shell says it: a dict, or None for a token
    with no role. Raises `Unanswered` when the shell cannot say. `ask` stands in for the shell in
    tests."""
    reply = (ask or _ask_shell)(token, "what this agent token may reach")
    return reach_in_reply(reply)


def standing_of(token, ask=None):
    """Whether `token` belongs to an agent attached to the shell right now. Raises `Unanswered`
    when the shell cannot say."""
    reply = (ask or _ask_shell)(token, "whether this agent token is live")
    return known_in_reply(reply)


def covers(surfaces, app_id, action):
    """Does one of `surfaces` cover `app_id.action`? `*` covers every app."""
    for surface in surfaces:
        surface = trim(surface)
        if surface == "*":
            return True
        if "." not in surface:
            if same_ascii_case(surface, app_id):
                return True
            continue
        app, named = surface.split(".", 1)
        if not same_ascii_case(app, app_id):
            continue
        if named.endswith("*"):
            if action.startswith(named[:-1]):
                return True
        elif named == action:
            return True
    return False


def names_app(surfaces, app):
    """Does one of `surfaces` name this app — as `app`, `app.action` or `app.prefix*`?"""
    app = trim(app)
    return any(trim(s) == "*" or same_ascii_case(trim(s).split(".", 1)[0], app) for s in surfaces)


def opening(app_id, action, args):
    """The app an act opens, when it is `shell.open_app` and its `name` says which one."""
    if not same_ascii_case(app_id, "shell") or action != "open_app" or not isinstance(args, dict):
        return None
    name = args.get("name")
    if not isinstance(name, str) or not trim(name):
        return None
    return trim(name)


def surfaces_text(surfaces):
    """The surfaces as a sentence reads them: "editor, documents and notes"."""
    if len(surfaces) == 1 and trim(surfaces[0]) == "*":
        return "every app"
    if not surfaces:
        return "nothing on this desktop beyond asking the person and reading its own session"
    if len(surfaces) == 1:
        return surfaces[0]
    return "%s and %s" % (", ".join(surfaces[:-1]), surfaces[-1])


def within(reach, app_id, action, graded, args):
    """May the agent `reach` belongs to use `app_id.action` with these `args`, an act this surface
    grades `graded`? The refusal, or None. Pure: the reach was read beforehand."""
    what = "%s.%s" % (app_id, action)
    if what in ALWAYS:
        return None
    who = "`%s` is the %s, which may touch %s, at most `%s`" % (
        reach["agent"], reach["name"], surfaces_text(reach["surfaces"]), reach["ceiling"])
    if not covers(reach["surfaces"], app_id, action):
        name = opening(app_id, action, args)
        if name is not None:
            if names_app(reach["surfaces"], name):
                return None
            return ("REACH: %s opens `%s`, an app the %s's reach does not name, so it was not run. "
                    "%s. Opening an app its reach does name is within it, at any ceiling. %s"
                    % (what, name, reach["name"], who, NEXT))
        return "REACH: %s is outside the %s's reach, so it was not run. %s. %s" % (
            what, reach["name"], who, NEXT)
    ceiling = gate.grade(reach["ceiling"])
    if ceiling is None:
        return ("REACH: the %s's ceiling `%s` is not a level this OS defines (%s), so %s was not "
                "run. %s." % (reach["name"], reach["ceiling"], " < ".join(gate.LADDER), what, who))
    level = gate.grade(graded)
    if level is None:
        return ("REACH: %s is graded `%s`, which is not a level this OS defines (%s), so it was "
                "not run." % (what, graded, " < ".join(gate.LADDER)))
    if level > ceiling:
        return ("REACH: %s is graded `%s`, above the %s's `%s` ceiling, so it was not run, and "
                "nobody was asked. %s. %s" % (what, graded, reach["name"], reach["ceiling"], who, NEXT))
    return None


def read_reach(token, ask=None):
    """The reach a call's token carries: None with no token or no role; the refusal sentence,
    as a `wire.RpcError`, when the shell cannot say — the call is refused, not let through."""
    if token is None:
        return None
    try:
        return reach_of(token, ask)
    except Unanswered as why:
        raise wire.RpcError(wire.RPC_INVALID_PARAMS,
                            "REACH: %s, so no act carrying an agent token runs until it can be. "
                            "Nothing was run." % why) from None


def needs_standing(app_id, action):
    return (app_id, action) not in STANDING_NOT_NEEDED


def require_standing(app_id, action, token, peer, ask=None):
    """A caller the kernel says is the mind account acts only as a live attached agent. Raises the
    refusal as a `wire.RpcError`; returns None when the call may go on."""
    if not needs_standing(app_id, action):
        return None
    if peer is None or not mind_door.is_mind(peer.uid):
        return None
    token = trim(token or "")
    if not token:
        raise wire.RpcError(wire.RPC_INVALID_PARAMS,
                            "MIND: a mind acts on the desktop only as an attached agent, with the "
                            "token its harness was given; this call carried none. Nothing was run.")
    try:
        live = standing_of(token, ask)
    except Unanswered as why:
        raise wire.RpcError(wire.RPC_INVALID_PARAMS,
                            "MIND: %s, so no act from a mind runs until it can be. Nothing was run."
                            % why) from None
    if not live:
        raise wire.RpcError(wire.RPC_INVALID_PARAMS,
                            "MIND: this agent token is not one the shell has given a live agent. "
                            "Nothing was run.")
    return None
