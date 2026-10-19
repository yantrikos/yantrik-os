# The live stream cuts to what matters, and has it ready before it happens

18 Oct 2026. Pranab: "It's a living OS, let's put it to test. Anticipate which one is coming up and have it up in case it's needed." This is the stream half of contracts week 1 (`task-contracts-2026-10-17.md`). It builds on the showcase (#840, #853) and draws everything inside its confined browser.

## What the viewer gets

The stream cuts between screens like a director, on what happens rather than on a clock. Each screen is already staged, hidden, before its moment, so the cut is instant. A staged screen whose moment doesn't come is quietly dropped.

| Screen | Shows | Its moment |
|---|---|---|
| **Work** | Mind View's terminal beside the game being built (today's layout) | commands run during a turn |
| **Plan** | the mission brief and its checklist: what's done, what's next | a mission starts, or a checklist line is ticked |
| **Payoff** | the game full screen for a few seconds, playing itself, labelled with what just changed | the page check passes after the folder changed |
| **Honesty** | *"Said FINISHED. Nothing in the folder changed. Not accepted."*, then the Mind's own correction when it comes | the Director refuses a FINISHED (#839) |
| **Between missions** | the approved games in rotation and a "today so far" recap: games built, features added, claims refused | no mission is running |

## Anticipation: what the Director can see coming

The Director doesn't guess from pixels. It knows the mission's phase, it writes the next prompt itself, and it reads the Mind's tool calls and replies as they happen. Each signal predicts a likely next moment.

| Signal (the Director already sees it) | Likely next | Stage now |
|---|---|---|
| it sends a brief, or a turn that ticks a checklist line | Plan | the plan card with the new tick |
| the Mind writes to the project folder (an `agent_run` that changes a code file's hash) | Payoff, once the page check runs | a fresh copy of the game, loaded hidden |
| the Mind runs `check-page` | Payoff or a failure | both the payoff and a "check failed: …" variant |
| a reply contains FINISHED | Payoff if the folder changed, Honesty if not | both cards; the folder check picks one |
| the mission's last turn, or the timer's rest begins | Between missions | the rotation and the recap |
| an approval card rises | nothing: stay | (cuts are held until the card is answered) |

**Staging is double-buffered.** The frame page keeps the screen on air plus one hidden layer. Staging replaces the hidden layer, and a cut swaps the layers with a short cross-fade. Only one screen is staged at a time, the most likely one. When two are equally likely (FINISHED, check-page), both small cards are prepared and the decision picks one.

**Rules:**
- Hold each screen for at least 20 s, except Honesty, which cuts at once.
- Never cut while an approval card is waiting, and never cover one.
- A staged screen expires after 90 s unused.
- Only the Director decides a cut. The Mind can't ask for one, and no text it writes becomes a screen except through the existing caption cleaning (#840).
- Everything is drawn by the frame page inside the confined showcase browser: no new account, no new permission, nothing outside `/opt/yantrik-live/showcase`.

## How we know the anticipation works

The Director logs each prediction and each cut to `log.jsonl`:
- `staged` (screen, signal);
- `cut` (screen, event, how long after the event it was on air);
- `dropped` (screen, staged but unused).

The status line on the frame shows nothing of this; the numbers are for us. Over one evening of unattended missions:

- **Cut latency:** median under 1 s from event to screen (staged), against the unstaged path measured the same way.
- **Hit rate:** at least 60% of staged screens are used within 90 s. A low hit rate means the signals are wrong, not that staging is.
- **No false cuts:** no screen goes on air without its event. A Payoff shown for a check that failed counts as a failure of this whole design.
- **No flicker:** no screen shorter than 20 s apart from Honesty, and no cut while a card is waiting.
- **Watchability:** Pranab watches 15 minutes and says whether it's more interesting than today's single view. That's the real test.

## Acceptance check (before code)

1. With a mission running on 561, the stream shows Plan at the start, Work during commands, and Payoff within 1 s of a passing page check after a change.
2. A deliberately false FINISHED shows Honesty within 1 s, and the Mind's correction follows on the same card.
3. Between missions, the approved games rotate and the recap reads true against `log.jsonl`.
4. An approval card stops all cuts until it's answered.
5. `log.jsonl` has `staged`, `cut` and `dropped` entries, and a short script prints latency, hit rate and false cuts for the evening.
6. A probe from inside the browser's namespace still sees only the showcase files, as after #853.

## Later, beyond the stream

This is the same pattern the OS needs elsewhere: predict the next thing the person will need, prepare it hidden and cheap, and show it only when the moment comes. Examples are the app they always open after a meeting, or the document a reply will ask for. The stream is where we can measure it first, unattended, with honest numbers.
