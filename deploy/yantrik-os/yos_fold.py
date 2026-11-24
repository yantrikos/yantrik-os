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
    "ai_accounts": ("detail", "private"),
    "approvals_off_for_test": ("means",),
    "minds": ("detail",),
}
# Keys whose prose is reference, not state: each string is clipped to CLIP_AT characters.
CLIP_KEYS = ("agents", "catalog", "harnesses", "recipes", "workroom", "mind_audit_recent", "proofs",
             "vault", "services", "problems", "dock", "mind_view", "installer", "files")
CLIP_AT = 80
# A long list of rows (agents, roles, recipes…) shows its first ROWS_SHOWN and a last row that counts
# the rest. Only the keys in CLIP_KEYS are ever cut this way; `pending_approvals` and the rest never.
ROWS_SHOWN = 6
# The shell's list of what `open_app` takes: `os_apps` presents it, so it is named, not printed.
LEFT_OUT = ("apps",)


def _shorten(value, drop, clip):
    if isinstance(value, dict):
        return {k: _shorten(v, drop, clip) for k, v in value.items() if k not in drop}
    if isinstance(value, list):
        rows = [_shorten(v, drop, clip) for v in value]
        if clip and len(rows) > ROWS_SHOWN + 1:
            rows = rows[:ROWS_SHOWN] + ["(%d more rows: detail=<this key> shows all)" % (len(rows) - ROWS_SHOWN)]
        return rows
    if clip and isinstance(value, str) and len(value) > CLIP_AT:
        return value[:CLIP_AT].rstrip() + "…"
    return value


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
        shortened = _shorten(v, DROP_FIELDS.get(k, ()), k in CLIP_KEYS) if (k in DROP_FIELDS or k in CLIP_KEYS) else v
        if json.dumps(shortened) != json.dumps(v):
            short.append(k)
        out[k] = shortened
    named = [k for k in LEFT_OUT if k in state]
    note = ""
    if short or named:
        note = ("\n(kept short: %s; the apps `open_app` takes are in os_apps. os_describe shell with "
                "detail=<key> prints one key in full)" % ", ".join(short + named))
    return out, note
