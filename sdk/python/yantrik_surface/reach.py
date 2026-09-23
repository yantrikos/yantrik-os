"""An agent's reach: what a role from the agent catalog may touch, held on every door that carries
the agent's token.

A port of `yantrik_ipc_transport::reach` — same rules, same sentences to the punctuation. If the
two ever disagree, the Rust file is right and this one is wrong; `deploy/yantrik-os/reach-vectors.json`
is generated from it, and the tests beside this package replay it.

A role — Researcher, Planner, Coder, Reviewer … — carries a *reach*: the surfaces it may act on
and a grade ceiling narrower than the machine's. Every `app.act` that carries the agent's token is
held to it after the unknown action and before the arguments, the ceiling and any grant: an act
outside the surfaces, or above the ceiling, is refused with `REACH:`. It is a second rule beside
`gate`'s, and it only ever takes away.

**Where the reach is kept.** The shell, which starts every agent, keeps each token's standing —
held to a role's reach, a live agent with no role, or no live agent at all. A door asks it with
`agent.reach {token_sha256}` on `app-shell.sock`, by the token's SHA-256 and never the token, and
only once the process listening there is a `yantrik-ui` binary. It fails closed: a token no live
agent carries, and every token-carrying call while the shell does not answer, is refused. A call
with no token is the person's own and asks nothing.

**Opening the apps it names.** `shell.open_app name=notes` and `shell.show_app name=notes` are
within a reach that names `notes`, whatever the reach's ceiling — opening a window is not an act on
the app's data. Only the shell serves them, and only the shell knows which app a name opens, so a
surface built on this package never lets one through by the app it names (`within_call` with no
`opens`); the vectors replay the rule with the table they carry.

Functions that decide return the refusal sentence, or None when the call may go on.
"""

import hashlib
from collections import namedtuple

from . import gate, wire

# Within every reach: asking the person, the steps that follow from asking, and reading one's own
# session — without them a role could not ask to be allowed what its reach does let it do.
ALWAYS = (
    "shell.request_approval",
    "shell.approval_status",
    "shell.consume_approval",
    "shell.record_unasked_action",
    "shell.read_agent",
)
# The acts that put an app's window in front of the person, each naming the app as `name`.
OPENING = ("shell.open_app", "shell.show_app")
# The method a door asks the shell a token's standing with, on the shell's own socket.
ASK = "agent.reach"
# How long a door waits for the shell's answer, as `ASK_ROUNDTRIP` in the Rust module.
ASK_ROUNDTRIP = 5.0
# What the shell says a token is.
HELD, PLAIN, UNKNOWN = "held", "plain", "unknown"

Reach = namedtuple("Reach", "agent role name surfaces ceiling")
Reach.__doc__ = """One agent's reach, as the shell keeps it: the agent (`<mind>:<conversation>`),
the role's id and name, its surfaces (`app`, `app.action`, `app.prefix*`) and its ceiling."""

NO_LIVE_AGENT = ("REACH: the agent token this call carries names no live agent on this desktop — "
                 "its agent was stopped, or the shell has restarted since it was handed out — so "
                 "nothing carrying it runs. Nothing was run.")

_NEXT = ("Say in your answer what else needs doing; the person, or whoever handed you this, can "
         "do it.")


class Unanswered(Exception):
    """The shell did not answer what a token is; the message says why."""


def token_digest(token):
    """The SHA-256 of a token, as lowercase hex: what a door asks the shell about in its place."""
    return hashlib.sha256(token.strip().encode("utf-8")).hexdigest()


def reach_from_json(value):
    """A `Reach` from the object the shell sends, or ValueError: every field, of its type."""
    if not isinstance(value, dict):
        raise ValueError("not an object")
    fields = {}
    for key in ("agent", "role", "name", "ceiling"):
        if not isinstance(value.get(key), str):
            raise ValueError("`%s` is not text" % key)
        fields[key] = value[key]
    surfaces = value.get("surfaces")
    if not isinstance(surfaces, list) or not all(isinstance(s, str) for s in surfaces):
        raise ValueError("`surfaces` is not a list of text")
    return Reach(surfaces=list(surfaces), **fields)


def standing_from_json(answer):
    """Read the shell's answer to `agent.reach`: `(HELD, reach)`, `(PLAIN, None)` or
    `(UNKNOWN, None)`. Anything else raises ValueError with why — never a guess."""
    kind = answer.get("standing") if isinstance(answer, dict) else None
    if not isinstance(kind, str):
        raise ValueError("its answer named no standing")
    if kind == HELD:
        try:
            return HELD, reach_from_json(answer.get("reach"))
        except ValueError:
            raise ValueError("its answer held a reach this door cannot read") from None
    if kind in (PLAIN, UNKNOWN):
        return kind, None
    raise ValueError("its answer named a standing this door does not know: `%s`" % kind)


def answered_by_the_shell(peer):
    """Who may answer `agent.reach`: the process listening on `app-shell.sock` must be a
    `yantrik-ui` binary, as it must be to spend a grant. None when it is; otherwise the sentence,
    `reach::answered_by_the_shell`'s word for word."""
    if peer is None:
        return ("the kernel would not say which process is answering as the shell, so it was not "
                "asked")
    exe = gate.exe_of(peer.pid)
    if exe is None:
        return ("the process answering as the shell (pid %d) could not be identified from /proc, "
                "so it was not asked" % peer.pid)
    if gate.is_shell_binary(exe):
        return None
    return ("the process answering as the shell is %s (pid %d), not the desktop's own %s, so it "
            "was not asked" % (exe, peer.pid, gate.SHELL_BINARY))


def ask(path, digest, peer_rule=answered_by_the_shell, timeout=ASK_ROUNDTRIP):
    """Ask whatever answers at `path` for a digest's standing, once `peer_rule` says it is the
    shell. Raises `Unanswered` with why when it could not be asked, did not answer, or answered
    something this door does not read."""
    try:
        reply = wire.call_once(path, ASK, {"token_sha256": digest}, timeout=timeout,
                               peer_rule=peer_rule)
    except wire.PeerRefused as e:
        raise Unanswered(str(e)) from e
    except ConnectionError as e:
        if str(e) == "Connection closed before response":
            raise Unanswered(str(e)) from e
        raise Unanswered("Connection failed (%s): %s" % (path, e)) from e
    except OSError as e:
        raise Unanswered("Connection failed (%s): %s" % (path, e)) from e
    except ValueError as e:
        raise Unanswered("Response parse error: %s" % e) from e
    if isinstance(reply, dict) and reply.get("error") is not None:
        error = reply.get("error")
        message = error.get("message") if isinstance(error, dict) else None
        raise Unanswered(message if isinstance(message, str) else "the shell refused the question")
    try:
        return standing_from_json(reply.get("result") if isinstance(reply, dict) else None)
    except ValueError as e:
        raise Unanswered(str(e)) from None


def standing_of(token):
    """What `token` is right now, asked of the shell over its socket. Raises `Unanswered`."""
    return ask(wire.default_socket_path(gate.SHELL), token_digest(token))


def decided(standing, why=None):
    """What a call carrying a token of this standing is held to: `(reach, None)` for a role's
    agent, `(None, None)` for a live agent with no role, and `(None, refusal)` for a token no live
    agent carries — or, with `why`, when the shell did not answer. `reach::decided`."""
    if why is not None:
        return None, ("REACH: the shell, which keeps every agent's reach, did not answer (%s), so no "
                      "act carrying an agent token runs until it does. Nothing was run." % why)
    kind, held = standing
    if kind == HELD:
        return held, None
    if kind == PLAIN:
        return None, None
    return None, NO_LIVE_AGENT


def reach_of(token, standing=None):
    """`decided`, for a token: its standing asked of the shell (or of `standing`, a function of
    the token that raises `Unanswered`, which a test hands in as its shell)."""
    try:
        found = (standing or standing_of)(token)
    except Unanswered as e:
        return decided(None, str(e))
    return decided(found)


# ── holding an act to a reach ────────────────────────────────────────────────


def covers(surfaces, app_id, action):
    """Does one of `surfaces` cover `app_id.action`?"""
    for surface in surfaces:
        surface = surface.strip()
        if "." not in surface:
            if surface.lower() == app_id.lower():
                return True
            continue
        app, named = surface.split(".", 1)
        if app.lower() != app_id.lower():
            continue
        if named.endswith("*"):
            if action.startswith(named[:-1]):
                return True
        elif named == action:
            return True
    return False


def surfaces_text(surfaces):
    """The surfaces as a sentence reads them: "editor, documents and notes"."""
    if not surfaces:
        return "nothing on this desktop beyond asking the person and reading its own session"
    if len(surfaces) == 1:
        return surfaces[0]
    return "%s and %s" % (", ".join(surfaces[:-1]), surfaces[-1])


def apps_named(surfaces):
    """The apps a reach names, each once, in order: the app part of every surface but the
    desktop's own `shell`, which is not an app to open."""
    apps = []
    for surface in surfaces:
        app = surface.strip().split(".", 1)[0].strip().lower()
        if not app or app == "shell" or "*" in app or app in apps:
            continue
        apps.append(app)
    return apps


def _who(reach):
    apps = apps_named(reach.surfaces)
    opens = (", and may open %s (`shell.open_app name=<app>`)" % surfaces_text(apps)) if apps else ""
    return "`%s` is the %s, which may touch %s, at most `%s`%s" % (
        reach.agent, reach.name, surfaces_text(reach.surfaces), reach.ceiling, opens)


def within(reach, app_id, action, graded):
    """May the agent `reach` belongs to use `app_id.action`, which its surface grades `graded`?
    The refusal, or None. `reach::within`: without the arguments, so no opening act passes by
    the app it names."""
    what = "%s.%s" % (app_id, action)
    if what in ALWAYS:
        return None
    if not covers(reach.surfaces, app_id, action):
        return "REACH: %s is outside the %s's reach, so it was not run. %s. %s" % (
            what, reach.name, _who(reach), _NEXT)
    ceiling = gate.grade(reach.ceiling)
    if ceiling is None:
        return ("REACH: the %s's ceiling `%s` is not a level this OS defines (%s), so %s was not "
                "run. %s." % (reach.name, reach.ceiling, " < ".join(gate.LADDER), what, _who(reach)))
    level = gate.grade(graded)
    if level is None:
        return ("REACH: %s is graded `%s`, which is not a level this OS defines (%s), so it was "
                "not run." % (what, graded, " < ".join(gate.LADDER)))
    if level > ceiling:
        return ("REACH: %s is graded `%s`, above the %s's `%s` ceiling, so it was not run, and "
                "nobody was asked. %s. %s" % (what, graded, reach.name, reach.ceiling, _who(reach),
                                             _NEXT))
    return None


def within_call(reach, app_id, action, graded, args, opens=None):
    """`within`, for a call with its arguments as sent — what the dispatch holds a call to. An
    opening act naming an app the reach names is within it whatever the reach's ceiling, the app
    resolved by `opens` (a name → the app it opens, or None). This package serves no shell and
    knows no resolution, so with no `opens` no opening act passes by the app it names."""
    what = "%s.%s" % (app_id, action)
    name = args.get("name") if what in OPENING and isinstance(args, dict) else None
    name = name.strip() if isinstance(name, str) else ""
    if not name:
        return within(reach, app_id, action, graded)
    app = opens(name) if opens is not None else None
    if app is not None and any(named.lower() == app.lower() for named in apps_named(reach.surfaces)):
        return None
    # A reach that names the act itself holds it as any other act, ceiling and all.
    if covers(reach.surfaces, app_id, action):
        return within(reach, app_id, action, graded)
    return ("REACH: %s `%s` is outside the %s's reach, so it was not run: a role may open only the "
            "apps its reach names. %s. %s" % (what, name, reach.name, _who(reach), _NEXT))
