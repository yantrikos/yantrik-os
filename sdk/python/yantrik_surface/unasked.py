"""An app's own gate records what it runs unasked, with the shell (#1353, round 4 of #1357).

`yantrik_ipc_transport::unasked`, rule for rule. The shell writes the audit line for every action
it answers `not_needed`; a program that reaches this surface's socket directly never meets that
path, so the gate that lets the call through hands the shell the whole call first, and the shell
writes the line. In either bypass, a call the `ask` mode would have refused and no grant stands
behind is recorded before it runs. If the shell cannot be reached, or does not say it wrote the
line, the call is refused: what cannot be recorded does not run.
"""

from . import gate, wire

ACTION = "record_gate_decision"
ROUNDTRIP = 2.0


def is_unasked(authority, app_id, action, graded, purpose):
    """Whether `authority` lets this call run unasked: in either bypass, with no grant, where the
    `ask` mode (no rule of any kind, same ceiling, same hold) would have refused it."""
    if authority.granted or authority.mode.name not in ("bypass", "bypass_all"):
        return False
    strict = gate.Authority(authority.ceiling, gate.Mode("ask", frozenset()), False,
                            getattr(authority, "asks_above", None))
    return gate.decide(strict, app_id, action, graded, purpose) is not None


def _written(reply):
    """The shell's answer: the act's answer carries `{"result": {"written": true}}`. Anything else
    is a refusal."""
    if not isinstance(reply, dict) or reply.get("error") is not None:
        return False
    act = reply.get("result")
    result = act.get("result") if isinstance(act, dict) else None
    return isinstance(result, dict) and result.get("written") is True


def ask_the_shell(app_id, action, graded, mode, args):
    """Have the shell write the line; True when it says it did."""
    import json
    path = wire.default_socket_path(gate.SHELL)
    try:
        reply = wire.call_once(path, "app.act", {
            "action": ACTION,
            "args": {"app": app_id, "action": action, "grade": graded, "mode": mode,
                     "args_json": json.dumps(args, ensure_ascii=False)},
        }, timeout=ROUNDTRIP, peer_rule=gate.must_be_the_shell)
    except (wire.PeerRefused, OSError, ConnectionError, ValueError):
        return False
    return _written(reply)


def decide_recorded(authority, app_id, action, graded, purpose, args, write=None):
    """`gate.decide`, and the record of the call when it runs unasked: the refusal, or None."""
    refusal = gate.decide(authority, app_id, action, graded, purpose)
    if refusal is not None or not is_unasked(authority, app_id, action, graded, purpose):
        return refusal
    if (write or WRITER)(app_id, action, graded, authority.mode.name, args):
        return None
    return ("NOT RUN: %s.%s would run without asking, and that has to be written down first, "
            "which could not be done (the shell did not say it wrote the line)." % (app_id, action))


# How the line is written: the shell, over its socket. The surface's own tests replace it.
WRITER = ask_the_shell
