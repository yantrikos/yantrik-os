"""Read a request of any of the three APIs into what the script needs: the last user message and how
many replies the model has already given to it. The step is derived from the history the client sends,
so two conversations at once, or a restarted server, never share a position."""


def _text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") in ("text", "input_text"))
    return ""


def _is_tool_result_only(msg):
    """Anthropic carries tool results in a user message of tool_result blocks."""
    c = msg.get("content")
    return isinstance(c, list) and c and all(isinstance(b, dict) and b.get("type") == "tool_result" for b in c)


def read(body):
    """(last user text, step, messages wanted by the API) from a request body."""
    msgs = body.get("messages")
    if not isinstance(msgs, list):
        raise ValueError("messages must be a list")
    last = -1
    for i, m in enumerate(msgs):
        if isinstance(m, dict) and m.get("role") == "user" and not _is_tool_result_only(m):
            last = i
    text = _text_of(msgs[last].get("content")) if last >= 0 else ""
    step = sum(1 for m in msgs[last + 1:] if isinstance(m, dict) and m.get("role") == "assistant")
    return text, step
