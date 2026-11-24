"""The backstop on what one tool result may carry back to a mind (#1388).

A mind's next request holds every result it has read. A free account's per-minute allowance is a
few thousand tokens, so one big read ended the turn with a 413 from the provider and no answer.
`cap_result` is applied once, to the text `serve_call` hands the mind, for every tool: not a
per-tool check. It is never applied to what the bridge parses for itself (`action_detail`,
`shell_state`), which read `yos` directly and do not pass through `serve_call`.
"""

# Why this size: a free account lists about 8,000 tokens a minute, a mind's own prompt takes some of
# it, and 24,000 bytes is about 6,000 tokens at the worst: one read may not use up the minute.
RESULT_CAP_BYTES = 24000


def how_to_ask_for_the_rest(tool, args):
    """The words that name the narrower read, from the call that was too big."""
    app = (args or {}).get("app")
    if tool == "os_describe" and app:
        if str(app).lower() in ("shell", "app-shell"):
            return "os_describe app=shell with detail=<one key of the state>, or actions=<family>_ for actions"
        return "os_describe app=%s with actions=<family>_ for one family of its actions" % app
    return "call %s again asking for less: a narrower app, key, range or count" % (tool or "the tool")


def cap_result(text, tool, args, limit=RESULT_CAP_BYTES):
    """`text` itself when it fits `limit` bytes; else its head cut on a line boundary, then one last
    line saying it was cut, by how much, and how to ask for the rest. The whole is within `limit`."""
    size = len(text.encode("utf-8"))
    if size <= limit:
        return text
    note = lambda kept: ("\n(cut: this answer was %d bytes and %d are shown, the rest left out so the next "
                         "request fits. To read more: %s.)" % (size, kept, how_to_ask_for_the_rest(tool, args)))
    room = limit - len(note(size).encode("utf-8"))
    head = text.encode("utf-8")[:max(room, 0)].decode("utf-8", "ignore")
    cut = head.rfind("\n")
    head = head[:cut] if cut > 0 else head
    return head + note(len(head.encode("utf-8")))


# os_describe's one way to ask for more of the shell's state: `detail`, the same word the fold's
# last line uses (`yos_fold`). Its schema and its argv live here so `yos-mcp` need not grow.
DETAIL_PROPERTY = {
    "type": "string",
    "description": "Optional, for app `shell`. The shell's state is shown short (its last line names "
                   "what was kept short): pass one of its keys here, e.g. `ai_accounts`, `apps` or "
                   "`catalog`, to read that key in full.",
}


def describe_argv(args):
    """`yos describe` for an `os_describe` call: `actions` is signatures alone, `detail` one key whole."""
    argv = ["describe", args["app"], "--fold"]
    if args.get("actions"):
        return argv + ["--no-state", "--actions", str(args["actions"])]
    return argv + (["--detail", str(args["detail"])] if args.get("detail") else [])
