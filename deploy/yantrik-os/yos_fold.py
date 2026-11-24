"""The shell's folded state: what a mind reads of `describe shell`, held to what it needs to act (#1388).

A mind's next request carries every describe it has read, and a free account's per-minute allowance
is small (Groq free lists 8,000 tokens in `tiers.rs`), so a describe that grew to 69 KB left the
mind's next call refused with a 413. `yos describe shell --fold` is the form a mind reads:

  * keys a mind rarely needs to act are SHORTENED: explanatory sentences dropped from named fields,
    long prose clipped, long lists cut to their first rows with the count of the rest. The key stays, with everything that says what is true.
  * `--detail <key>` (`os_describe` `detail`) prints one key in full, the one way to ask for more.
  * nothing the person relies on for safety is ever touched: only the keys named below are, and the
    mode, private state, pending approvals and questions, what is in front, and the windows are not.
"""
import json

# Fields that are a sentence about a value the row already carries: `detail` says again what
# `state` says, `private` / `means` explain `sharing` / `on`. The value stays; the sentence goes.
DROP_FIELDS = {
    "ai_accounts": ("detail", "private", "models"),
    "approvals_off_for_test": ("means",),
    "minds": ("detail",),
}
# Keys whose prose is reference, not state: each string is clipped to CLIP_AT characters.
CLIP_KEYS = ("agents", "catalog", "harnesses", "recipes", "workroom", "mind_audit_recent", "proofs",
             "vault", "services", "problems", "dock", "mind_view", "installer", "files", "notifications")
CLIP_AT = 80
# A long list of rows (agents, roles, recipes…) shows its first ROWS_SHOWN and a last row that counts
# the rest. Only the keys in CLIP_KEYS are ever cut this way; `pending_approvals` and the rest never.
ROWS_SHOWN = 6
# The lists that were the bulk of what the machine printed (#1388, measured on the test VM) are held
# tighter: a mind that wants a row asks for the key with `detail`. Fewer rows, shorter strings.
TIGHT_KEYS = ("agents", "catalog", "recipes", "harnesses", "problems", "notifications", "dock", "proofs",
              "mind_view", "mind_audit_recent", "services", "workroom")
TIGHT_ROWS, TIGHT_CLIP = 1, 24
NEVER_CLIPPED = ("id", "name")
# The state may take about 3,500 of the 10,000 bytes (#1396): the 62 one-line actions and the families
# take about 6,000. A key that holds a list or an object and is still over FOLD_KEY_AT bytes once
# shortened is FOLDED TO A COUNT ("agents": "40 rows", "files": "9 fields"): what a mind
# needs of it to act is that it is there and how big, and `detail=<key>` prints it whole. A first row
# or a clipped field said nothing of the rest and cost 250 to 600 bytes a key on the machine. Small keys
# and every scalar are untouched. The keys below are never folded to a count: what the person relies
# on for safety (the mode and its rules, what waits on them, what is in front, the clock, the vault, the
# switch that turns approvals off) and what a mind needs to pick an account or a mind.
FOLD_KEY_AT = 60
NEVER_COUNTED = ("summary", "version", "private", "locked", "screen", "screen_id", "clock", "windows", "in_front",
                 "desktop_in_front", "pending_approvals", "pending_questions", "mind_mode", "tool_permission",
                 "approvals_off_for_test", "ai_accounts", "minds", "vault")
# The shell's list of what `open_app` takes: `os_apps` presents it, so it is named, not printed.
LEFT_OUT = ("apps",)


def _shorten(value, drop, clip, shown=ROWS_SHOWN, at=CLIP_AT):
    if isinstance(value, dict):
        # `id` and `name` are what a mind passes back to act on a row: never clipped (#1396).
        return {k: v if k in NEVER_CLIPPED and isinstance(v, str) else _shorten(v, drop, clip, shown, at)
                for k, v in value.items() if k not in drop}
    if isinstance(value, list):
        rows = [_shorten(v, drop, clip, shown, at) for v in value]
        if clip and len(rows) > shown + 1:
            rows = rows[:shown] + ["(+%d rows: detail=<key>)" % (len(rows) - shown)]
        return rows
    if clip and isinstance(value, str) and len(value) > at:
        return value[:at].rstrip() + "…"
    return value


def _counted(key, value, whole):
    """`value` (`whole` before it was shortened) as a count of `whole` when it is a list or an object over FOLD_KEY_AT bytes and not a safety key."""
    if key in NEVER_COUNTED or not isinstance(value, (list, dict)) or len(json.dumps(value, ensure_ascii=False)) <= FOLD_KEY_AT:
        return value
    n = len(whole)
    return "%d %s" % (n, ("row" if n == 1 else "rows") if isinstance(value, list) else ("field" if n == 1 else "fields"))


def fold_shell(state, detail=None):
    """The shell's state as a mind reads it, and the line after it saying what was left short."""
    if detail:
        if detail not in state:
            return {}, "(no key `%s` in the shell's state; its keys: %s)" % (detail, ", ".join(sorted(state)))
        return {detail: state[detail]}, ""
    out, short = {}, []
    for k, v in state.items():
        if k in LEFT_OUT:
            continue
        tight = (TIGHT_ROWS, TIGHT_CLIP) if k in TIGHT_KEYS else (ROWS_SHOWN, CLIP_AT)
        shortened = _shorten(v, DROP_FIELDS.get(k, ()), k in CLIP_KEYS, *tight) if (k in DROP_FIELDS or k in CLIP_KEYS) else v
        counted = _counted(k, shortened, v)
        # A key folded to a count shows `N rows` / `N fields` in place; the note after names only the clipped ones.
        if counted is shortened and json.dumps(shortened) != json.dumps(v):
            short.append(k)
        shortened = counted
        out[k] = shortened
    named = [k for k in LEFT_OUT if k in state]
    note = ""
    if short or named:
        note = ("\n(kept short: %s; apps: os_apps. 'N rows' is a count; detail=<key> prints one key whole)"
                % ", ".join(short + named))
    return out, note


LINE_AT = 48  # a folded action is one line of about this many characters (#1388)


NAMED_AT = 3  # up to this many optional parameters are named (`open_lens(app?)`), past it they are counted
HINT_AT = 40  # only a count this big (the question actions' 60) says `(actions=<name>)` itself; the legend says it for the rest
NAMED_CHARS = 32  # and only while the names are this short together


def signature(action, hint=True):
    """An action's folded signature: the parameters it takes when they are few, else its required
    parameters and a count of the rest, `+N optional (actions=<name>)` (#1396).

    The four question actions take the whole card vocabulary (about 60 optional fields), so listing every
    parameter made each of their lines 600+ characters. Up to NAMED_AT optional parameters (and NAMED_CHARS
    characters of names) are named with a `?`, since `open_lens(app?)` says more than `open_lens(+1 optional)`
    and is shorter. A parameter the mind must give is always named; `actions=<name>` prints the action whole.
    `hint` False leaves `(actions=<name>)` off, for a family line that says it once for its members."""
    params = action.get("parameters") or {}
    props = params.get("properties") or {}
    wanted = set(params.get("required") or [])
    required = [n for n in props if n in wanted]
    optional = [n for n in props if n not in wanted]
    shown = list(required)
    if optional and len(optional) <= NAMED_AT and sum(len(n) + 3 for n in optional) <= NAMED_CHARS:
        shown += [n + "?" for n in optional]
    elif optional:
        shown.append("+%d optional%s" % (len(optional), " (actions=%s)" % action["name"] if hint and len(optional) > HINT_AT else ""))
    return "%s(%s)" % (action["name"], ", ".join(shown))


def common_settles(actions):
    """The `settles` value most actions share, said once in the legend; only an action that differs says its own."""
    seen = {}
    for a in actions:
        seen[a.get("settles", "?")] = seen.get(a.get("settles", "?"), 0) + 1
    return max(seen, key=seen.get) if seen else "?"


def legend(common):
    return "  (settles %s unless a line says otherwise; actions=<name> or <family>_ prints in full)" % common


def marks(action, common):
    """`[grade]`, or `[grade, settles <when>]` when this action does not settle as most do."""
    grade, settles = action.get("permission", "?"), action.get("settles", "?")
    return "[%s]" % grade if settles == common else "[%s, settles %s]" % (grade, settles)


def render_action_line(action, sentence, common="on return"):
    """An action not in a family, folded: signature, grading marks, the first sentence of what it does,
    all on one line of about LINE_AT characters. The marks always stay whole; the sentence is what is clipped.
    `actions=<name>` reads the whole description (`render_action`).
    `sentence` is the description's first, already through the caller's `one_line` (no line break of its own)."""
    head = "  act: %s  %s" % (signature(action), marks(action, common))
    room = LINE_AT - len(head) - 3
    if not sentence or room < 12:
        return head
    if len(sentence) > room:
        sentence = sentence[:room - 1].rstrip() + "…"
    return "%s — %s" % (head, sentence)


BARE_AT = 120  # a family line longer than this names its members without their arguments


def render_family(head, members, common, bare_at=None):
    """A family on one line (#1396): the grade once when all its members share it, else the members
    grouped by grade (`[safe] open_lens(app?), open_today(); [sensitive] open_capture(name)`). A line that
    runs past `bare_at` characters names its members without arguments (`files_go, files_enter`):
    `actions=<family>_` has the signatures."""
    def line(bare):
        def sig(m):
            return (m["name"] if bare else signature(m, hint=False)) + (
                "" if m.get("settles", "?") == common else " settles " + str(m.get("settles")))
        sigs = lambda ms: ", ".join(sig(m) for m in ms)
        grades = list(dict.fromkeys(m.get("permission", "?") for m in members))
        if len(grades) == 1 and len({m.get("settles", "?") for m in members}) == 1:
            return "  %s_* — %d actions, all %s — %s" % (head, len(members), marks(members[0], common), sigs(members))
        groups = "; ".join("[%s] %s" % (g, sigs([m for m in members if m.get("permission", "?") == g])) for g in grades)
        return "  %s_* (%d) — %s" % (head, len(members), groups)
    full = line(False)
    return full if len(full) <= (BARE_AT if bare_at is None else bare_at) else line(True)
