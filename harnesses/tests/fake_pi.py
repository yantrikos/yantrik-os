"""A `pi --mode rpc` that is not pi: the RPC lines, and nothing behind them.

Scenario comes from argv (everything after the flags pi itself would take, so the harness's own
argv building is exercised unchanged) or from $FAKE_PI_SCENARIO:

    text      a couple of text deltas, then agent_end + agent_settled
    tool      a tool execution around the text: start, two accumulated updates, end, and the
              assistant message's usage in message_end
    refused   a tool execution whose result is a REFUSED answer
    slow      answers only once $FAKE_PI_GATE exists, so two processes can be held mid-answer
    dialog    an extension_ui_request confirm, which must come back cancelled
    abort     never finishes on its own; answers `abort` with agent_end + agent_settled
    exit      dies in the middle of the turn
    silent    acknowledges the prompt and then says nothing at all, ever
    noend     text and agent_end, but never agent_settled — the grace path
    refuse    answers the prompt command with success:false

Launched as `[sys.executable, this file, scenario]`, so it needs no exec bit and no shebang.
"""

import json
import os
import sys
import threading
import time

SCENARIO = os.environ.get("FAKE_PI_SCENARIO") or (sys.argv[1] if len(sys.argv) > 1 else "text")
# What the harness passed us, written out so a test can assert on the command line it builds.
ARGV_DUMP = os.environ.get("FAKE_PI_ARGV_DUMP")
# Every command line the harness sends, for the dialog and abort assertions.
COMMANDS_DUMP = os.environ.get("FAKE_PI_COMMANDS_DUMP")
# One line per process started — its pid, the agent token in its environment and the directory
# it was started in — so a test can see one process per conversation, which agent each one is,
# and where in the filesystem it runs (#183).
ENV_DUMP = os.environ.get("FAKE_PI_ENV_DUMP")
# One line per prompt, with this process's pid, so a test can see which process answered.
PROMPTS_DUMP = os.environ.get("FAKE_PI_PROMPTS_DUMP")


def emit(event):
    sys.stdout.write(json.dumps(event) + "\n")
    sys.stdout.flush()


def record(path, value):
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(value + "\n")


def settle():
    emit({"type": "turn_end"})
    emit({"type": "agent_end", "messages": [], "willRetry": False})
    emit({"type": "agent_settled"})


def assistant_end(text):
    """What pi says when an assistant message is settled, usage and all."""
    emit({"type": "message_end", "message": {
        "role": "assistant", "model": "fake-model",
        "content": [{"type": "text", "text": text}],
        "usage": {"input": 100, "output": 20, "cacheRead": 5, "cacheWrite": 0,
                  "totalTokens": 125,
                  "cost": {"input": 0.001, "output": 0.002, "cacheRead": 0, "cacheWrite": 0,
                           "total": 0.003}}}})


def handle_prompt(command):
    record(PROMPTS_DUMP, json.dumps({"pid": os.getpid(), "message": command.get("message"),
                                     "at": time.time()}))
    emit({"type": "response", "command": "prompt", "id": command.get("id"),
          "success": SCENARIO != "refuse",
          **({"error": "no provider configured"} if SCENARIO == "refuse" else {})})
    if SCENARIO == "refuse":
        return
    emit({"type": "agent_start"})

    if SCENARIO == "exit":
        emit({"type": "message_update",
              "assistantMessageEvent": {"type": "text_delta", "delta": "starting"}})
        sys.stdout.flush()
        os._exit(7)

    if SCENARIO == "silent":
        return

    if SCENARIO == "tool":
        emit({"type": "message_update",
              "assistantMessageEvent": {"type": "thinking_delta", "delta": "hmm, notes"}})
        args = {"app": "calendar", "action": "add_event", "args": {"title": "dentist"}}
        emit({"type": "tool_execution_start", "toolCallId": "t1", "toolName": "os_act",
              "args": args})
        # Pi reports a running call's output accumulated, not as deltas.
        for so_far in ("checking the calendar\n", "checking the calendar\nadding dentist\n"):
            emit({"type": "tool_execution_update", "toolCallId": "t1", "toolName": "os_act",
                  "args": args, "partialResult": {"content": [{"type": "text", "text": so_far}],
                                                  "details": {}}})
        emit({"type": "tool_execution_end", "toolCallId": "t1", "toolName": "os_act",
              "result": {"content": [{"type": "text", "text":
                                      "checking the calendar\nadding dentist\ndone"}],
                         "details": {"exitCode": 0}},
              "isError": False})
        emit({"type": "message_update",
              "assistantMessageEvent": {"type": "text_delta", "delta": "Added it."}})
        assistant_end("Added it.")
        settle()
        return

    if SCENARIO == "refused":
        emit({"type": "tool_execution_start", "toolCallId": "t2", "toolName": "os_act",
              "args": {"app": "files", "action": "delete"}})
        emit({"type": "tool_execution_end", "toolCallId": "t2", "toolName": "os_act",
              "result": {"content": [{"type": "text", "text":
                                      "REFUSED: plan mode is on, so nothing was changed."}]},
              "isError": False})
        emit({"type": "message_update",
              "assistantMessageEvent": {"type": "text_delta", "delta": "That was refused."}})
        settle()
        return

    if SCENARIO == "slow":
        # Answers once $FAKE_PI_GATE exists (or after 20s), so a test can hold two processes
        # mid-answer at once and know they overlapped, however slow the machine is.
        gate = os.environ.get("FAKE_PI_GATE")
        deadline = time.monotonic() + 20
        while gate and not os.path.exists(gate) and time.monotonic() < deadline:
            time.sleep(0.02)
        emit({"type": "message_update",
              "assistantMessageEvent": {"type": "text_delta",
                                        "delta": "answered by %d" % os.getpid()}})
        settle()
        return

    if SCENARIO == "dialog":
        emit({"type": "extension_ui_request", "id": "ui-1", "method": "confirm",
              "params": {"message": "Delete every file in ~/work?"}})
        return  # finished only once the harness answers

    if SCENARIO == "abort":
        emit({"type": "message_update",
              "assistantMessageEvent": {"type": "text_delta", "delta": "working"}})
        return  # only `abort` ends this

    if SCENARIO == "noend":
        emit({"type": "message_update",
              "assistantMessageEvent": {"type": "text_delta", "delta": "Nearly done."}})
        emit({"type": "turn_end"})
        emit({"type": "agent_end", "messages": [], "willRetry": False})
        return  # deliberately no agent_settled

    emit({"type": "message_update",
          "assistantMessageEvent": {"type": "thinking_delta", "delta": "let me think"}})
    for piece in ("Two ", "windows."):
        emit({"type": "message_update",
              "assistantMessageEvent": {"type": "text_delta", "delta": piece}})
    settle()


def main():
    record(ARGV_DUMP, json.dumps(sys.argv[1:]))
    record(ENV_DUMP, json.dumps({"pid": os.getpid(),
                                 "token": os.environ.get("YANTRIK_AGENT_TOKEN"),
                                 "cwd": os.getcwd()}))
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            command = json.loads(line)
        except ValueError:
            continue
        record(COMMANDS_DUMP, line)
        kind = command.get("type")

        if kind == "prompt":
            threading.Thread(target=handle_prompt, args=(command,), daemon=True).start()
        elif kind == "abort":
            emit({"type": "response", "command": "abort", "id": command.get("id"), "success": True})
            emit({"type": "message_update",
                  "assistantMessageEvent": {"type": "text_delta", "delta": " — stopped."}})
            settle()
        elif kind == "new_session":
            emit({"type": "response", "command": "new_session", "id": command.get("id"),
                  "success": True})
        elif kind == "extension_ui_response":
            # The whole point of the dialog scenario: whatever the harness said, it is recorded
            # above and the turn ends here rather than hanging on an unanswered dialog.
            emit({"type": "message_update", "assistantMessageEvent":
                  {"type": "text_delta", "delta": "I will not do that then."}})
            settle()
        elif kind == "get_state":
            emit({"type": "response", "command": "get_state", "id": command.get("id"),
                  "success": True})


if __name__ == "__main__":
    if "--version" in sys.argv:
        print("pi 0.87.0 (fake)")
        sys.exit(0)
    main()
