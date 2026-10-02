# Mind status lines and dropped answers

## What already worked
The wire already carried `status` events (`Event::Status`), the Agents store kept the latest one in `agent.status` (replaced, never appended), and `describe shell` `agents[].status` already listed it. Unknown kinds were already ignored by the host. What was missing: the Chat v2 work card and strip never read `status`, and a `tool_start` left a stale status behind.

## Changes
1. **Status line.** `wire/lens_work.rs`: the working card's activity line is the running call, else the latest status, else the last finished call. `agents/store.rs`: `tool_start` clears the status (a pending question keeps its line). No timers; it arrives with the event.
2. **Drops are logged.** `host.rs`: `Flight` remembers why it was abandoned (stopped, interrupted, no listener) and logs one `tracing::info` per turn with harness, turn_id and why, from `chunk`, `event` and `complete`/`fail`. The text is never logged.
3. **Never silent.** The conversation history store lives in the Slint chat and has no write path from the host, so this takes the simpler honest option: the host keeps (capped at 16 KiB) what a harness says after the panel stopped listening, and when the turn ends well it calls `Host::with_late_answer`; the shell raises "<Mind> answered after you left: <first 80 chars>". It is not recorded in History. A turn the person stopped is not announced.
4. **Parity.** `describe shell` gains `conversation_status`, the active mind's latest status for a running turn (`agents[].status` already existed).

## Tests
- `yantrik-harness`: drop logged once with reason and no text; late answer handed on; stopped turn not announced.
- `yantrik-ui`: status replaces and tool_start overrides (store and card activity); late-answer notification text.

## Not done
The "answered after you left" text is a notification only, not a History entry. The "new chat" and "mind switch" reasons are not distinguishable by the host and read as "no listener".
