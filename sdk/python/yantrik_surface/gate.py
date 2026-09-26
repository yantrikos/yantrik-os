"""The one rule every `app.act` meets: the machine's ceiling, the person's mode, and the grant
that stands in for their Allow.

A port of `yantrik_ipc_transport::gate` — same order, same defaults, same sentences to the
punctuation. If the two ever disagree, the Rust file is right and this one is wrong; the tests
beside this package quote the Rust source and replay the policy vectors generated from it.

The order, as the Rust dispatch runs it:

  1. **The ceiling** (`tool_permission` in `~/.config/yantrik/settings.yaml`), on the grade
     alone. Nothing reaches past it: not a mode, not a grant. `CEILING:`.
  2. **The grant**, if the call carries one, spent through the shell's `consume_approval` —
     and only once the ceiling has passed, so a person's Allow is never used up on an act the
     ceiling then refuses (#154).
  3. **The mode** (`mind-mode.json`, beside the settings): above what it runs unasked, with no
     grant spent and no session rule for the action, the call is refused with `GRANT:` and
     told how to get one. Every mode runs `standard` on a socket (`SOCKET_FLOOR`), because the
     desktop's own processes make standard calls and cannot yet be told from a mind (#43). So
     is an action whose own published description says it cannot be undone, in every mode but
     bypass and whatever its grade above `safe`; a session rule never covers one, and in plan
     mode no session rule covers anything (docs/surface-protocol.md, section 7).

A grant is spent only through the desktop's own shell: before anything is written to
`app-shell.sock`, the process listening on it must be a `yantrik-ui` binary
(`must_be_the_shell`, the transport's `owner::must_be_the_shell`).

`describe` never comes here. Reading an app is free.

Functions that decide return the refusal sentence, or None when the call may go on.
"""

import json
import os
import sys
import time
from typing import NamedTuple

from . import wire

LADDER = ("safe", "standard", "sensitive", "dangerous")
DEFAULT_CEILING = "sensitive"
MODE_FILE = "mind-mode.json"
# What each mode runs without a grant: the top of the ladder it allows, strictest mode first.
MODES = {"plan": "safe", "ask": "standard", "auto": "sensitive", "bypass": "dangerous"}
SOCKET_FLOOR = "standard"
DEFAULT_MODE = "ask"
# One hop to the shell and back, as `GRANT_ROUNDTRIP` in the Rust gate.
GRANT_ROUNDTRIP = 5.0
# The shell's surface, where grants are kept and spent.
SHELL = "shell"
# The key an agent token travels under: beside `args` on `app.act`, never inside them.
AGENT_TOKEN = "agent_token"


def grade(permission):
    """Where a grade sits on `LADDER`, or None if it is not a level this OS defines."""
    try:
        return LADDER.index(permission)
    except ValueError:
        return None


# ── what an action says cannot be undone ─────────────────────────────────────

# The wording that makes an action's own description a promise that it cannot be taken back:
# the seven phrases, in order, of the gate's `UNRECOVERABLE_PHRASES` (which the shell's card and
# the MCP bridge read too). `decide` reads the action's published description with it.
UNRECOVERABLE_PHRASES = (
    "not recoverable",
    "cannot be undone",
    "can't be undone",
    "irreversible",
    "permanently",
    "permanent",
    "no undo",
)


def unrecoverable(purpose):
    """Does an action's own description say it cannot be taken back? Lowercased, substrings."""
    lower = (purpose or "").lower()
    return any(phrase in lower for phrase in UNRECOVERABLE_PHRASES)


# ── the ceiling ──────────────────────────────────────────────────────────────


def settings_path():
    """The shell's settings file, from `$HOME` as the Rust side reads it (then `USERPROFILE`,
    then the working directory)."""
    home = os.environ.get("HOME") or os.environ.get("USERPROFILE") or "."
    return os.path.join(home, ".config", "yantrik", "settings.yaml")


def mode_path():
    """Where the shell publishes the mode: beside the settings file."""
    return os.path.join(os.path.dirname(settings_path()), MODE_FILE)


def ceiling_from(text):
    """`tool_permission` out of settings text, as `ceiling_from` reads it: the first line whose
    key is `tool_permission` decides, quotes stripped; a value off the ladder is the default."""
    for line in text.split("\n"):
        key, colon, value = line.partition(":")
        if not colon or key.strip() != "tool_permission":
            continue
        value = value.strip().strip('"').strip("'")
        return value if grade(value) is not None else DEFAULT_CEILING
    return DEFAULT_CEILING


def configured_ceiling(path=None):
    """The machine's ceiling for programmatic callers, read per call: a person tightens it
    while apps are running, and a boundary that only noticed at launch would be one the
    Settings screen lies about. Missing or unreadable is the default, never looser."""
    try:
        with open(path or settings_path(), "r", encoding="utf-8") as f:
            text = f.read()
    except (OSError, ValueError):
        return DEFAULT_CEILING
    return ceiling_from(text)


# ── the mode ─────────────────────────────────────────────────────────────────


class Mode(NamedTuple):
    """The mode as the shell last published it: its name, and the `(app, action)` pairs a
    person allowed for the rest of the session. Compares equal to a plain `(name, rules)`."""

    name: str
    session_rules: frozenset = frozenset()

    def allows(self):
        """The highest grade this mode runs unasked, as a position on the ladder. A name that
        is not a mode reads as `ask`, never as something looser."""
        top = MODES.get(self.name)
        return grade(top) if top is not None else grade("standard")

    def covers(self, app, action):
        """Whether a session rule is the person's standing answer for `app.action`."""
        return (app, action) in self.session_rules


def _as_u64(value):
    if isinstance(value, int) and not isinstance(value, bool) and 0 <= value < (1 << 64):
        return value
    return None


def _proc_stat(pid):
    """`pid`'s state character (field 3 of `/proc/<pid>/stat`) and start time (field 22), or
    None when there is no such process or no `/proc` to ask."""
    try:
        with open("/proc/%d/stat" % pid, "rb") as f:
            stat = f.read().decode("utf-8", "replace")
    except (OSError, ValueError):
        return None
    # Field 2, the command name, may hold spaces and parentheses, so the fields are counted
    # from the LAST `)`. The token after that is field 3, the state, and starttime is field 22.
    fields = stat.rpartition(")")[2].split()
    if len(fields) < 20 or not fields[0]:
        return None
    try:
        return fields[0][0], int(fields[19])
    except ValueError:
        return None


def proc_start_ticks(pid):
    """`pid`'s start time — field 22 of `/proc/<pid>/stat`, clock ticks since boot — or None
    when there is no such process or no `/proc` to ask.

    A pid on its own says nothing: the kernel reuses them, and a recycled pid would resurrect
    a dead shell's mode. A pid and the start time it was recorded with name one process,
    because whatever reuses the pid does not also reuse the boot tick it started at.
    """
    seen = _proc_stat(pid)
    return None if seen is None else seen[1]


def boot_id():
    """The boot this machine is in — `/proc/sys/kernel/random/boot_id` — or None when there is
    no `/proc` to ask. The kernel picks a fresh random id on every boot, so an identity
    recorded under a different one names a machine that has since restarted (#333)."""
    try:
        with open("/proc/sys/kernel/random/boot_id", "rb") as f:
            text = f.read().decode("utf-8", "replace")
    except (OSError, ValueError):
        return None
    return text.strip() or None


def _names_a_live_shell(doc):
    """Whether the shell that wrote `doc` is the process still running under that pid, in this
    boot — `gate::names_a_live_shell`, whose comment carries the reasoning. A file that names
    no shell reads as it always did; a file that names one is trusted only while it runs, and
    less than the whole identity — a pid with no start time, or a file from before the boot id
    existed with no boot to tie the pair to — fails closed like a dead one."""
    pid = _as_u64(doc.get("shell_pid"))
    start = _as_u64(doc.get("shell_start_ticks"))
    boot = doc.get("boot_id") if isinstance(doc.get("boot_id"), str) else None
    if pid is None or start is None or boot is None:
        return pid is None and start is None and boot is None
    if pid > 0xFFFFFFFF:
        return False
    this_boot = boot_id()
    if this_boot is None or boot.strip() != this_boot:
        return False
    seen = _proc_stat(pid)
    if seen is None:
        return False
    state, started = seen
    # A zombie has exited but not been reaped: it keeps its pid and its start time in /proc,
    # and the shell behind them is gone all the same. `X` is the kernel's own "dead".
    return started == start and state not in ("Z", "X")


def mode_from(text, now):
    """Read the mode out of what the shell wrote, the way `gate::mode_from` does.

    Anything unreadable is `ask`. A bypass whose deadline has passed reads as the mode before
    it (or `ask`), so a shell that died mid-bypass does not leave this app trusting it past the
    minute the person was promised; a bypass with no deadline is trusted while the shell that
    wrote the file is running — the file names it and the boot it wrote in, and a name that is
    not running, or a boot that has ended, reads as `ask`, session rules and all (#154, #333).
    """
    try:
        doc = json.loads(text)
    except ValueError:
        return Mode(DEFAULT_MODE, frozenset())
    if not isinstance(doc, dict):
        doc = {}
    if not _names_a_live_shell(doc):
        return Mode(DEFAULT_MODE, frozenset())
    name = doc.get("mode") if isinstance(doc.get("mode"), str) else ""
    if name not in MODES:
        name = DEFAULT_MODE
    if name == "bypass":
        until = _as_u64(doc.get("bypass_expires_unix"))
        if until is not None and now >= until:
            previous = doc.get("previous")
            previous = previous if isinstance(previous, str) else DEFAULT_MODE
            name = previous if previous in MODES and previous != "bypass" else DEFAULT_MODE
    rules = set()
    listed = doc.get("session_rules")
    for rule in listed if isinstance(listed, list) else ():
        if isinstance(rule, dict) and isinstance(rule.get("app"), str) \
                and isinstance(rule.get("action"), str):
            rules.add((rule["app"], rule["action"]))
    return Mode(name, frozenset(rules))


def configured_mode(path=None, now=None):
    """The mode right now, from the file the shell writes; `ask` when there is none."""
    try:
        with open(path or mode_path(), "r", encoding="utf-8") as f:
            text = f.read()
    except (OSError, ValueError):
        return Mode(DEFAULT_MODE, frozenset())
    return mode_from(text, int(time.time()) if now is None else now)


# ── what rides beside `args` ─────────────────────────────────────────────────


def grant_of(params):
    """The grant a call carries: the `request_id` the shell answered `request_approval` with,
    once a person pressed Allow. Text, trimmed; an empty one is none, and so is a non-string."""
    value = params.get("grant") if isinstance(params, dict) else None
    if isinstance(value, str) and value.strip():
        return value.strip()
    return None


def agent_token_of(params, args):
    """The agent token a call carries beside `args` — and any copy inside `args` taken out.

    Call it before anything reads `args`: a grant is bound to the arguments, the approval card
    draws them and the audit log keeps them, so a token among them is a token anyone reading
    the screen can replay. The copy inside is removed and NOT used.
    """
    if isinstance(args, dict) and AGENT_TOKEN in args:
        del args[AGENT_TOKEN]
        print("[yantrik] an agent token arrived inside `args`; it was removed and not used. It "
              "travels beside `args` on app.act, never among them", file=sys.stderr)
    value = params.get(AGENT_TOKEN) if isinstance(params, dict) else None
    if isinstance(value, str) and value.strip():
        return value.strip()
    return None


# ── who answers as the shell ─────────────────────────────────────────────────

# The shell's program name. The rule is the file name, not the directory, so the installed shell
# and a developer's `target/release/yantrik-ui` both pass and nothing else does.
SHELL_BINARY = "yantrik-ui"
# What Linux appends to `/proc/<pid>/exe` when the file a process was started from has since been
# replaced: an update replaces the shell's binary under a running shell, which is still the shell.
_DELETED = " (deleted)"


def is_shell_binary(exe):
    """Whether `exe` — `/proc/<pid>/exe` resolved — is a `yantrik-ui` binary."""
    if not isinstance(exe, str):
        return False
    if exe.endswith(_DELETED):
        exe = exe[:-len(_DELETED)]
    return exe.startswith("/") and os.path.basename(exe) == SHELL_BINARY


def exe_of(pid):
    """The program behind a pid, as `/proc` says it, or None when it cannot be read."""
    if not isinstance(pid, int) or pid <= 0:
        return None
    try:
        return os.readlink("/proc/%d/exe" % pid)
    except OSError:
        return None


def must_be_the_shell(peer):
    """The rule for `app-shell`: the process answering on it must be a `yantrik-ui` binary.
    None when it is; otherwise the sentence, which ends in a full stop because it is dropped into
    the middle of the gate's own refusal — `owner::must_be_the_shell`, word for word."""
    if peer is None:
        return ("the kernel would not say which process is answering as the shell, so it could "
                "not be checked and the grant was not offered to it.")
    exe = exe_of(peer.pid)
    if exe is None:
        return ("the process answering as the shell (pid %d) could not be identified from /proc, "
                "so the grant was not offered to it." % peer.pid)
    if is_shell_binary(exe):
        return None
    return ("the process answering as the shell is %s (pid %d), not the desktop's own %s, so the "
            "grant was not offered to it." % (exe, peer.pid, SHELL_BINARY))


# ── spending a grant ─────────────────────────────────────────────────────────


class GrantRefused(Exception):
    """The shell would not spend a grant; the message is the shell's own sentence."""


def spend_through_shell(grant, app, action, args):
    """Burn `grant` for exactly `app.action(args)` through the shell's `consume_approval`.

    Only through the shell: before the grant is written to `app-shell.sock`, the process
    listening on it must pass `must_be_the_shell`. The check of the grant is the shell's —
    granted, unspent, unexpired, bound to this app, this action and these arguments — and a
    refusal carries the shell's sentence. A shell that cannot be reached is a refusal too, worded
    as the Rust client words it.
    """
    path = wire.default_socket_path(SHELL)
    try:
        reply = wire.call_once(path, "app.act", {
            "action": "consume_approval",
            "args": {"request_id": grant, "app": app, "action": action, "args_json": args},
        }, timeout=GRANT_ROUNDTRIP, peer_rule=must_be_the_shell)
    except wire.PeerRefused as e:
        raise GrantRefused(str(e)) from e
    except ConnectionError as e:
        if str(e) == "Connection closed before response":
            raise GrantRefused(str(e)) from e
        raise GrantRefused("Connection failed (%s): %s" % (path, e)) from e
    except OSError as e:
        raise GrantRefused("Connection failed (%s): %s" % (path, e)) from e
    except ValueError as e:
        raise GrantRefused("Response parse error: %s" % e) from e
    if isinstance(reply, dict) and reply.get("error") is not None:
        error = reply.get("error")
        message = error.get("message") if isinstance(error, dict) else None
        raise GrantRefused(message if isinstance(message, str) else "the shell refused it.")


# ── the decision ─────────────────────────────────────────────────────────────


class Authority:
    """What is known about one call before its action runs: the ceiling and the mode as the
    files say them, and whether a grant was attached and spent."""

    def __init__(self, ceiling=DEFAULT_CEILING, mode=None, granted=False):
        self.ceiling = ceiling
        self.mode = mode if mode is not None else Mode(DEFAULT_MODE, frozenset())
        if isinstance(self.mode, str):
            self.mode = Mode(self.mode, frozenset())
        self.granted = granted

    @classmethod
    def now(cls, settings=None, mode=None):
        """The ceiling and the mode as the files say them now, and no grant yet."""
        return cls(configured_ceiling(settings), configured_mode(mode), False)

    def spend(self, grant, app_id, action, graded, args, spender=None):
        """Spend `grant` for exactly `app_id.action(args)`, graded `graded` — but only if the
        ceiling lets that grade be used at all. Returns the refusal, or None once spent.

        Any grant attached is spent once the ceiling passes, whether or not the mode would
        have asked: a replayed, swapped or invented grant ends the call here, in the shell's
        words, rather than being ignored.
        """
        _, refusal = within_ceiling(self.ceiling, app_id, action, graded)
        if refusal is not None:
            return refusal
        try:
            (spender or spend_through_shell)(grant, app_id, action, args)
        except GrantRefused as why:
            return ("GRANT: `%s` does not authorise %s.%s — %s Nothing was run; a grant covers "
                    "one action, once, with the arguments the person was shown."
                    % (grant, app_id, action, why))
        self.granted = True
        return None


def permits(cap, graded):
    """Whether a caller capped at `cap` may use an action graded `graded` — the ceiling's
    comparison alone. None when `graded` is not a level this OS defines; a cap off the ladder
    reads as the default ceiling."""
    level = grade(graded)
    if level is None:
        return None
    top = grade(cap)
    return level <= (top if top is not None else grade(DEFAULT_CEILING))


def within_ceiling(ceiling, app_id, action, graded):
    """`(level, None)`, or `(None, the ceiling's refusal)`.

    An unrecognised ceiling falls back to the default rather than failing open; an
    unrecognised grade is refused rather than waved through — a typo in a grade must fail
    closed, or the typo silently becomes an exemption.
    """
    within = permits(ceiling, graded)
    if within is None:
        return None, ("CEILING: %s.%s is graded `%s`, which is not a level this OS defines "
                      "(%s), so it was not run." % (app_id, action, graded, " < ".join(LADDER)))
    if not within:
        return None, ("CEILING: %s.%s is graded `%s`, above this machine's `%s` ceiling "
                      "(`tool_permission` in ~/.config/yantrik/settings.yaml), so it was not "
                      "run. An action at that grade needs a person to authorise it directly — "
                      "raise the ceiling in Settings if that is the intent."
                      % (app_id, action, graded, ceiling))
    return grade(graded), None


def decide(authority, app_id, action, graded, purpose=""):
    """May `app_id.action`, graded `graded` and published with the description `purpose`, run
    under `authority`? The refusal, or None — `gate::decide`, rule for rule.

    The ceiling, then the mode — on the grade and the action's own description alone, before
    the arguments, the revision guard or the handler. A grant answers every question after the
    ceiling. Bypass runs everything under the ceiling; every other mode runs what its column
    says (never less than the socket floor) and asks about anything whose description says it
    cannot be undone (a `safe` read excepted). A session rule covers its action — except one
    that cannot be undone, and except in plan mode, which raises no card and so has no standing
    answers. Pure: nothing is read and nothing is spent here.
    """
    level, refusal = within_ceiling(authority.ceiling, app_id, action, graded)
    if refusal is not None:
        return refusal
    if authority.granted:
        return None
    mode = authority.mode
    everything = len(LADDER) - 1
    irreversible = level > 0 and unrecoverable(purpose)
    asks = mode.allows() < everything and (
        irreversible or level > max(mode.allows(), grade(SOCKET_FLOOR)))
    if not asks:
        return None
    plan = mode.allows() == 0
    if not plan and not irreversible and mode.covers(app_id, action):
        return None
    return grant_refusal(app_id, action, graded, mode, irreversible)


def permit(authority, app_id, action, graded, purpose, args, grant=None, spender=None):
    """The whole rule for one call, for a caller that holds the grade where it holds the call:
    the ceiling, then the grant (spent only past the ceiling), then the mode."""
    if grant:
        refusal = authority.spend(grant, app_id, action, graded, args, spender)
        if refusal is not None:
            return refusal
    return decide(authority, app_id, action, graded, purpose)


_HOW = ("Ask the shell for approval first (`request_approval` with this app, action and these "
        "exact arguments, poll `approval_status`, then send the granted request_id as `grant` on "
        "app.act — `yos act` does all of that for you), or have the person at the machine press "
        "Allow when the card appears.")
_PLAN = ("Say what you would do and let the person decide; they switch the mode from the chip "
         "in the status bar.")
_FINAL_WORD = "its own description says it cannot be undone"


def grant_refusal(app, action, graded, mode, irreversible=False):
    """The refusal for a call the mode will not run without a grant — `grant_refusal` in the
    Rust gate, its four sentences to the punctuation: plan or not, and whether the reason is the
    grade or the action's own word that it cannot be undone. `mode` is a `Mode` or a name."""
    mode = mode if isinstance(mode, Mode) else Mode(mode, frozenset())
    if mode.name == "plan" and not irreversible:
        return ("GRANT: %s.%s is graded `%s` and this machine is in plan mode, which raises no "
                "card for anything above `%s` — so it was not run. %s"
                % (app, action, graded, SOCKET_FLOOR, _PLAN))
    if mode.name == "plan":
        return ("GRANT: %s.%s is graded `%s` and %s, and this machine is in plan mode, which "
                "raises no card for that — so it was not run. %s"
                % (app, action, graded, _FINAL_WORD, _PLAN))
    if not irreversible:
        allowed = LADDER[max(mode.allows(), grade(SOCKET_FLOOR))]
        return ("GRANT: %s.%s is graded `%s` and this machine is in %s mode, which runs nothing "
                "above `%s` without asking — so it was not run. %s"
                % (app, action, graded, mode.name, allowed, _HOW))
    return ("GRANT: %s.%s is graded `%s` and %s, and this machine is in %s mode, which asks "
            "before anything that cannot be undone — so it was not run. %s"
            % (app, action, graded, _FINAL_WORD, mode.name, _HOW))
