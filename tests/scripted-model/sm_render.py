"""One scripted turn, in each API's own wire format: whole, and streamed.

OpenAI-compatible: /v1/chat/completions (`tool_calls`, SSE). Anthropic Messages: /v1/messages
(`tool_use`, SSE). Ollama native: /api/chat (`message.tool_calls`, NDJSON).
Each streamed form is a list of byte chunks, so the server can pace them.
"""
import json
import time

MODEL = "scripted-model"
CHUNK = 24


def pieces(text):
    return [text[i:i + CHUNK] for i in range(0, len(text), CHUNK)]


def _dump(o):
    return json.dumps(o, separators=(",", ":"))


def _calls(turn, tag, as_string):
    out = []
    for i, c in enumerate(turn["tool_calls"]):
        out.append({"id": "%s_%d" % (tag, i), "name": c["name"], "args": c["arguments"]})
    return out


# ── OpenAI-compatible ──

def openai_whole(turn, tag, model):
    msg = {"role": "assistant", "content": turn["text"] or None}
    calls = _calls(turn, "call_" + tag, True)
    if calls:
        msg["tool_calls"] = [{"id": c["id"], "type": "function", "function": {"name": c["name"], "arguments": _dump(c["args"])}} for c in calls]
    return {"id": "chatcmpl-" + tag, "object": "chat.completion", "created": int(time.time()), "model": model,
            "choices": [{"index": 0, "message": msg, "finish_reason": "tool_calls" if calls else "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}


def openai_stream(turn, tag, model):
    def chunk(delta, finish=None):
        return ("data: " + _dump({"id": "chatcmpl-" + tag, "object": "chat.completion.chunk", "created": int(time.time()),
                                  "model": model, "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}) + "\n\n").encode()
    out = [chunk({"role": "assistant", "content": ""})]
    for p in pieces(turn["text"]):
        out.append(chunk({"content": p}))
    calls = _calls(turn, "call_" + tag, True)
    for i, c in enumerate(calls):
        args = _dump(c["args"])
        half = max(1, len(args) // 2)
        out.append(chunk({"tool_calls": [{"index": i, "id": c["id"], "type": "function", "function": {"name": c["name"], "arguments": ""}}]}))
        for part in (args[:half], args[half:]):
            out.append(chunk({"tool_calls": [{"index": i, "function": {"arguments": part}}]}))
    out.append(chunk({}, "tool_calls" if calls else "stop"))
    out.append(("data: " + _dump({"id": "chatcmpl-" + tag, "object": "chat.completion.chunk", "model": model, "choices": [],
                                  "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}) + "\n\n").encode())
    out.append(b"data: [DONE]\n\n")
    return out


# ── Anthropic Messages ──

def anthropic_whole(turn, tag, model):
    content = [{"type": "text", "text": turn["text"]}] if turn["text"] else []
    calls = _calls(turn, "toolu_" + tag, False)
    content += [{"type": "tool_use", "id": c["id"], "name": c["name"], "input": c["args"]} for c in calls]
    return {"id": "msg_" + tag, "type": "message", "role": "assistant", "model": model, "content": content,
            "stop_reason": "tool_use" if calls else "end_turn", "stop_sequence": None,
            "usage": {"input_tokens": 1, "output_tokens": 1}}


def anthropic_stream(turn, tag, model):
    def ev(name, data):
        return ("event: %s\ndata: %s\n\n" % (name, _dump(data))).encode()
    out = [ev("message_start", {"type": "message_start", "message": {
        "id": "msg_" + tag, "type": "message", "role": "assistant", "model": model, "content": [],
        "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 1, "output_tokens": 1}}})]
    idx = 0
    if turn["text"]:
        out.append(ev("content_block_start", {"type": "content_block_start", "index": idx, "content_block": {"type": "text", "text": ""}}))
        for p in pieces(turn["text"]):
            out.append(ev("content_block_delta", {"type": "content_block_delta", "index": idx, "delta": {"type": "text_delta", "text": p}}))
        out.append(ev("content_block_stop", {"type": "content_block_stop", "index": idx}))
        idx += 1
    calls = _calls(turn, "toolu_" + tag, False)
    for c in calls:
        out.append(ev("content_block_start", {"type": "content_block_start", "index": idx,
                                              "content_block": {"type": "tool_use", "id": c["id"], "name": c["name"], "input": {}}}))
        args = _dump(c["args"])
        half = max(1, len(args) // 2)
        for part in (args[:half], args[half:]):
            out.append(ev("content_block_delta", {"type": "content_block_delta", "index": idx,
                                                  "delta": {"type": "input_json_delta", "partial_json": part}}))
        out.append(ev("content_block_stop", {"type": "content_block_stop", "index": idx}))
        idx += 1
    out.append(ev("message_delta", {"type": "message_delta", "delta": {"stop_reason": "tool_use" if calls else "end_turn", "stop_sequence": None},
                                    "usage": {"output_tokens": 1}}))
    out.append(ev("message_stop", {"type": "message_stop"}))
    return out


# ── Ollama native ──

def _ollama_msg(content, turn):
    msg = {"role": "assistant", "content": content}
    if turn["tool_calls"]:
        msg["tool_calls"] = [{"function": {"name": c["name"], "arguments": c["arguments"]}} for c in turn["tool_calls"]]
    return msg


def _stamp():
    return time.strftime("%Y-%m-%dT%H:%M:%S.000000Z", time.gmtime())


def ollama_whole(turn, tag, model):
    return {"model": model, "created_at": _stamp(), "message": _ollama_msg(turn["text"], turn), "done": True,
            "done_reason": "stop", "prompt_eval_count": 1, "eval_count": 1}


def ollama_stream(turn, tag, model):
    lines = [{"model": model, "created_at": _stamp(), "message": {"role": "assistant", "content": p}, "done": False}
             for p in pieces(turn["text"])]
    if turn["tool_calls"]:
        lines.append({"model": model, "created_at": _stamp(), "message": _ollama_msg("", turn), "done": False})
    lines.append({"model": model, "created_at": _stamp(), "message": {"role": "assistant", "content": ""}, "done": True,
                  "done_reason": "stop", "prompt_eval_count": 1, "eval_count": 1})
    return [(_dump(l) + "\n").encode() for l in lines]


# ── model lists ──

def models_openai(model):
    return {"object": "list", "data": [{"id": model, "object": "model", "created": 0, "owned_by": "scripted"}]}


def models_anthropic(model):
    return {"data": [{"type": "model", "id": model, "display_name": "Scripted test model", "created_at": "2026-01-01T00:00:00Z"}],
            "has_more": False, "first_id": model, "last_id": model}


def models_ollama(model):
    return {"models": [{"name": model, "model": model, "size": 0, "digest": "scripted", "details": {"family": "scripted"}}]}
