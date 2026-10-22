"""Which files an agent may have a surface read or write (#443): the rule every side shares.

A surface runs as the person, so anything the person can read it can read, and whatever it reads
into a document an agent can read back. The Rust apps hold every file an agent names to
`yantrik_ipc_contracts::home_paths`: in the person's home, outside its protected places (keys,
Yantrik's own data and state, the files that run as the person), and, for a write, nowhere hidden
where programs read their settings and startup. A Python surface is held to the same rule, asked
of the one place it lives: the shell's `files_stat` is that rule's `stat`, and the verdicts here are
`home_paths::may_read_file` and `may_write_file` read off its answer, as `verdict.rs` reads them.
So the protected list and the walk through links are never copied into Python to drift.

The person, calling with no agent token, is not asked. An agent is a call carrying an agent token,
or one from the mind's own account, as `agent_is_calling` says in the Rust runtime. A shell that
does not answer refuses an agent's file: it is never "allowed".

    from yantrik_surface import files

    @s.action("open")
    def open_document(path: str) -> dict:
        "Open a file."
        files.may_read_file(path)   # raises Refusal for an agent's file it may not read
        ...
"""

import os

from . import gate, mind_door, reach, wire
from .surface import Refusal, agent_token, caller

# How long the shell gets to answer one `files_stat`. It answers off its UI thread, from one
# `stat` walk; a shell that takes longer than this is not one an agent's file waits on.
STAT_ROUNDTRIP = 5.0

# `home_paths::HIDDEN_RULE`, word for word: the sentence is the rule's, whichever side refuses.
HIDDEN_RULE = ("an agent does not write into hidden folders or dotfiles in the home, where "
               "programs read their settings and startup")


def agent_is_calling():
    """Whether the call being handled is an agent's: it carried an agent token, or it came from
    the mind's own account. Read inside a handler, where the dispatch put both."""
    if agent_token():
        return True
    peer = caller()
    return peer is not None and mind_door.is_mind(peer.uid)


def shell_stat(asked):
    """The shell's `files_stat` of `asked`: `{exists: true|false|"unknown", kind, reason, ...}`.
    Raises `Refusal` when the shell does not say."""
    path = wire.default_socket_path(gate.SHELL)
    try:
        reply = wire.call_once(path, "app.act", {"action": "files_stat", "args": {"path": asked}},
                               timeout=STAT_ROUNDTRIP, peer_rule=gate.must_be_the_shell)
        answer = reach._act_result(reply)  # noqa: SLF001 - the one reader of an act's answer
    except (wire.PeerRefused, reach.Unanswered, OSError, ConnectionError, ValueError) as e:
        raise Refusal("the shell did not say whether an agent may use %s (%s); nothing was "
                      "read or written" % (asked, e)) from None
    if not isinstance(answer, dict) or "exists" not in answer:
        raise Refusal("the shell's answer about %s is not one it gives; nothing was read or "
                      "written" % asked)
    return answer


def _refusal(asked, answer):
    """`verdict.rs`'s refusal for a path `stat` would not answer for, ending with the reason."""
    reason = answer.get("reason")
    return ("an agent reads and writes files only in the person's home, outside its protected "
            "places; %s is %s" % (asked, reason if isinstance(reason, str) else
                                  "not a path it may use"))


def may_read_file(asked, stat=None):
    """Refuse (`Refusal`) an agent's read of anything but a file in the person's home, outside its
    protected places. The person is not asked. `stat` stands in for the shell in a test."""
    if not agent_is_calling():
        return
    asked = asked.strip() if isinstance(asked, str) else ""
    answer = (stat or shell_stat)(asked)
    exists, kind = answer.get("exists"), answer.get("kind")
    if exists is True and kind == "file":
        return
    if exists is True:
        raise Refusal("%s is not a file" % asked)
    if exists is False:
        raise Refusal("there is no file at %s" % asked)
    raise Refusal(_refusal(asked, answer))


def may_write_file(asked, stat=None):
    """Refuse (`Refusal`) an agent's write anywhere but a file in the person's home, outside its
    protected places and its hidden ones, in a folder already there, and not through a link. A
    file already there may be replaced, as `home_paths::may_write_file` allows. The person is not
    asked."""
    if not agent_is_calling():
        return
    ask = stat or shell_stat
    asked = asked.strip() if isinstance(asked, str) else ""
    answer = ask(asked)
    if not isinstance(answer.get("exists"), bool):
        raise Refusal(_refusal(asked, answer))
    home = os.path.expanduser("~")
    path = os.path.join(home, asked[2:]) if asked.startswith("~/") else asked
    if where_programs_look(path, home):
        raise Refusal("%s; %s is hidden_place" % (HIDDEN_RULE, asked))
    if answer["exists"]:
        if os.path.islink(path):
            raise Refusal("%s is a link; an agent writes a file, not through a link to one" % asked)
        if answer.get("kind") != "file":
            raise Refusal("%s is not a file" % asked)
        return
    folder = os.path.dirname(path)
    there = ask(folder)
    if there.get("exists") is True and there.get("kind") == "directory":
        return
    if there.get("exists") is True:
        raise Refusal("%s is not a folder" % folder)
    if there.get("exists") is False:
        raise Refusal("the folder %s does not exist; an agent's write makes no folders on the way"
                      % folder)
    raise Refusal(_refusal(folder, there))


def _hidden_below(path, home):
    """`writes.rs::hidden_below`: a part below `home` that starts with `.`, or `bin` first."""
    home = os.path.normpath(home)
    path = os.path.normpath(path)
    if path != home and not path.startswith(home.rstrip(os.sep) + os.sep):
        return False
    parts = [p.lower() for p in os.path.relpath(path, home).split(os.sep) if p not in ("", ".")]
    return bool(parts) and (parts[0] == "bin" or any(p.startswith(".") for p in parts))


def where_programs_look(path, home):
    """`home_paths::where_programs_look`: somewhere in `home` programs read their settings or
    startup from, as written and where it resolves (the deepest part that exists, links followed,
    the rest joined on: `os.path.realpath`)."""
    return (_hidden_below(path, home)
            or _hidden_below(os.path.realpath(path), os.path.realpath(home)))
