# The initiative inbox: Stage A, the OS half

16 October 2026. Agreed between the OS workspace and yantrik-mind-3e, on the plan Pranab chose
the same day: grounded initiative on top of a reliable obligation channel, at most two
attention-seeking moments a day, the rest in an inbox, and the controls from the first live
delivery.

## Why

Stage 0 check 3 found that a notice a mind sends while no turn is running reaches nobody.
`Event::Notice` is `{ text }` and nothing else. With a turn in flight it becomes the agent's
status line, which the next status overwrites. With no turn in flight the host logs `notice with
no turn in flight; kept`, answers `{"kept": true}` and stores nothing. A reminder the person
asked for, or the result of a job they delegated, is lost exactly when they are not looking.

## What this is, in one paragraph

A mind hands the OS a **notice**: a short card with an id, a family, a reason, and at most one
harmless button. The OS writes it to the one notification store this machine already has, and
answers `stored` only once it is on disk. The OS, not the mind, decides whether the card
interrupts (a toast) or waits in the inbox. That decision weighs a daily budget shared by every
mind, the person's quiet settings, and what the screen is doing. The person's response (useful,
not now, never this kind) goes back to the mind that sent it on its next poll, and the OS also
applies it on its own.

## Decisions

1. **One store.** Notices live in the notifications service's store
   (`~/.local/share/yantrik/notifications.json`, `yantrik-ipc-contracts::notifications`). It
   already has bounded fields, `replaces_id`, actions, and a `sender` taken from `SO_PEERCRED`.
   A notice is a `Notification` with a new optional `notice` field. The inbox is a filtered view
   (records with `notice` set), not a second store.
2. **Scoped to who sent it.** Dedupe, supersede and feedback are all keyed by the **harness the
   host established**, never by anything the request says. One mind cannot replace, hide or read
   feedback on another's notice.
3. **The OS spends the attention budget.** The budget is global across harnesses, because the OS
   is one entity with many minds. A mind may ask for `interrupt`; the OS grants it or files the
   card in the inbox.
4. **Text is data, buttons are navigation.** Every field is clamped and drawn as plain text.
   Clamping is recorded in the stored record. A button can only open something; it can never
   act. An action of an unknown kind is dropped and the card still lands.
5. **Ack after fsync.** `stored: true` means the record is on disk. It says nothing about the
   person having seen it; that comes back later as feedback.
6. **#806 folds in.** A plain `{text}` notice with no turn in flight is stored as a mind notice
   with an OS-made id. #806 is closed when this lands.

## The wire

### Attach advertises the feature

The `harness.attach` reply gains:

```json
{ "session": "s4-…", "as": "mind",
  "features": ["notice_inbox_v1"],
  "initiative": { "quiet": false, "off": ["discovery"], "inbox_only": ["calendar"] } }
```

- `features` is a list of strings. A harness sends notices only when it sees
  `notice_inbox_v1`. An older desktop sends no `features` at all, and that means no.
- `initiative` is the person's current settings (see *Controls*). Every `harness.poll` answer
  carries it too, so a change reaches a mind within one poll.

### A notice: `harness.notice`

A new method beside `harness.event`, with no `turn_id`, because a notice is not part of a turn:

```json
{ "session": "s4-…",
  "notice": {
    "notice_id": "rem-2026-10-16-0001",
    "family": "reminder",
    "title": "Stretch",
    "body": "You asked me at 14:02 to remind you to stretch.",
    "why": "You asked for this reminder.",
    "evidence": ["task:41"],
    "attention": "interrupt",
    "useful_until": "2026-10-16T19:30:00Z",
    "supersedes": null,
    "action": { "label": "Open note", "kind": "open_note", "ref": "n-7f3a" }
  } }
```

| Field | Required | Bound | Meaning |
|---|---|---|---|
| `notice_id` | yes | 1–96 chars of `[A-Za-z0-9._:-]` | The mind's stable dedupe key. |
| `family` | yes | one of `reminder`, `delegated`, `machine`, `calendar`, `discovery`, `start_of_day` | Decides budget and controls. An unknown family is refused, because the OS could not apply the person's per-family settings to it. |
| `title` | yes | one line, 80 chars | |
| `body` | no | 45 words and 400 chars | |
| `why` | yes | one line, 140 chars | Shown as "Why you got this". |
| `evidence` | no | up to 5 refs, 300 chars each | Shown for drill-down only, never followed automatically. |
| `attention` | no, default `inbox` | `interrupt` or `inbox` | What the mind asks for. The OS decides. |
| `useful_until` | no | RFC 3339 | After it, the card leaves the inbox; the record stays. |
| `supersedes` | no | a `notice_id` | Replaces that card, only if this same harness sent it. |
| `action` | no | one `{label ≤ 32, kind, ref ≤ 300}` | See *Buttons*. |

**Answers.** Every answer is a result, never a transport error, so a mind's outbox can tell
"retry" from "stop":

- `{"stored": true, "notice_id": "…"}`: on disk, after fsync.
- `{"stored": true, "notice_id": "…", "duplicate": true}`: already held from this harness. The
  stored card is unchanged. To change a card, send a new `notice_id` with `supersedes`.
- `{"stored": false, "retry_after_secs": N, "why": "…"}`: try again later (rate cap, store
  unavailable).
- `{"stored": false, "refused": "…"}`: do not retry (malformed, unknown family, missing
  `title` or `why`).

When a field was cut, the answer and the stored record both carry
`"clamped": ["body", "evidence[2]"]`. The mind's test can then tell its 46th word was cut
rather than lost.

### A `{text}` notice through `harness.event`

This is today's path, kept for older harnesses:

- **A turn is in flight:** unchanged. It shows as the agent's status line.
- **No turn is in flight (#806's case):** stored as a notice with `family: "delegated"`,
  `title` set to the text's first line (clamped), the rest as `body`, `why` set to "Sent by
  ⟨harness name⟩ while nothing was running", `attention: inbox`, and an OS-made
  `notice_id` (`os-<revision>`). The answer is `{"kept": true, "stored": true, "notice_id": "os-…"}`,
  so an old harness that reads `kept` still works.

### Feedback: on `harness.poll`

When the person responds to a card, the next poll **by the harness that sent it** carries:

```json
"notice_feedback": [
  { "seq": 12, "notice_id": "rem-…", "kind": "useful",
    "seen_at": "2026-10-16T19:03:11Z", "opened_at": null, "at": "2026-10-16T19:03:20Z" } ]
```

- `kind` is one of `useful`, `not_useful`, `not_now`, `never_family`, `inbox_only`, `wrong`,
  `dismissed`, or `expired`. `dismissed` and `expired` are recorded without the person pressing
  a feedback button.
- Delivery is at-least-once. A harness acknowledges with `"feedback_ack": 12` on a later poll.
  Until then, the items keep coming back, across restarts too. The cursor is stored per harness
  id in the notifications store.
- Feedback goes only to the harness that sent the notice. No other harness sees it.

## How the OS decides interrupt or inbox

The notifications service decides once, when it stores the card, and records why:
`notice.granted = "interrupt" | "inbox"` and `notice.grant_reason`, one of `asked_inbox`,
`budget_spent`, `quiet_initiative`, `family_inbox_only`, `quiet_hours`, `rate`, or `granted`.

In order:

1. The person set this family to off: the notice is stored but hidden (`grant_reason:
   "family_off"`) and answered as stored, so the mind stops retrying. The mind should not have
   sent it. Only initiative families can be off. `reminder` and `delegated` are things the
   person asked for, so their row offers On and Inbox only, never Off.
2. The mind asked for `inbox`: inbox.
3. **Quiet hours** apply to every family, obligations included ("a requested reminder doesn't
   bypass quiet hours unless he sets it"). In quiet hours the card goes to the inbox and is
   toasted when quiet hours end, if it is still useful by then.
4. **Obligation families** (`reminder` and `delegated`) skip steps 5–7. They are what the person
   asked for, so they don't spend the budget.
5. **Quiet initiative is on:** every initiative family (`machine`, `calendar`, `discovery`,
   `start_of_day`) goes to the inbox, whatever the mind asked for.
6. The person set this family to inbox-only: inbox.
7. **The budget.** At most **2 granted interrupts per local day across all harnesses**,
   counted in the store, so the count survives restarts and is shared by every mind. When it is
   spent: inbox.

**What the shell does with a granted interrupt.** It shows a toast only if the screen allows it
at that moment. If something is fullscreen, a call is active, or focus is on, it holds the
toast. When that ends, it toasts the card if it is still useful and not yet seen; past
`useful_until`, it leaves it in the inbox. The record says how it was shown (`shown_as`: `toast`
or `inbox`) and when.

**No voice in Stage A**, whatever is asked.

## Buttons

At most one `action`. It only opens something, as the person, by the shell; no mind takes part.

| `kind` | `ref` | What the button does |
|---|---|---|
| `open_file` | an absolute path | Opens it read-only in its default app. Dropped unless it resolves (after symlinks) to a regular file under the person's home. |
| `open_event` | a calendar event id | Shows that event in Calendar. Dropped if Calendar does not know the id. |
| `open_note` | a Notes id | Opens that note. |
| `open_url` | an `http` or `https` URL | The button shows the host ("Open example.com"). Opens in the browser. |

Any other kind, or a `ref` that fails its check, is dropped. It is listed in `clamped` as
`action`, and the card still lands.

## Bounds that protect the store

- **Per harness:** 20 notices per hour and 200 undismissed. Past either, the answer is
  `stored: false` with `retry_after_secs`.
- **Mind notices are not evicted by other senders.** Today the store keeps 500 records and drops
  the oldest first, so `notify-send` in a loop could push out an obligation the OS acknowledged.
  Notices get their own allowance, counted apart from the 500.
- **Nothing acknowledged is deleted.** A dismissed or expired notice leaves the live file after
  the store's 7-day TTL, as today, but is appended to `notices-archive.jsonl` first. Archive,
  never delete.

## Who the sender is

The harness host lives in the shell. It already knows each attached harness from `SO_PEERCRED`
and the attach: id, account and verified process. It posts the notice to the notifications
service on that harness's behalf, with `notice.harness = { id, uid, verified }`.

The service accepts `notice.harness` **only from a caller it judges to be the desktop**
(`Sender.desktop`, the same judgment the approval card relies on). From any other caller it
strips the `notice` fields and stores a plain notification. So `yos notify` from a mind's tool
cannot forge another harness's notice or spend the budget.

The card names the sender in the same words the approval card uses, beside the mind's claimed
name.

## Controls (from the first live delivery)

In **Settings → Minds → Initiative**:
- **Quiet initiative**, a global switch. Obligations still arrive.
- A per-family row: **On**, **Inbox only** or **Off**. `calendar` is on by default and can be
  turned off in one tap from its card.
- **Quiet hours**: they are already a setting today, and they apply to notices.

Each card carries **Why you got this** (the `why` line and its evidence) and the feedback
buttons: Useful, Not useful, Not now, Never this kind (sets the family to Off), Inbox only, and
Wrong.

- `never_family` and `inbox_only` change these settings; the person can see and undo them.
  `never_family` is per family, never per mind, because the OS is one assistant.
- The mind applies them too. The OS enforces them whether or not it does.

## The inbox surface

A section of the Lens, **For you**, shows live cards: title, body, why, the action, and the
feedback row. Cards come newest first, with granted interrupts first among unread. The existing
notification centre keeps everything, notices included, with a family chip.

`yos describe shell` reports the inbox (count, unread, the newest five ids and titles) so that
an agent test can read what the person sees.

## Not in Stage A

- Voice.
- Background model calls; they wait for the paired latency test.
- Notices that act. Every button only opens something.
- Initiative from any harness other than the Mind. The method is open to every attached harness
  (one OS), but Stage A only tests the Mind.

## Tests (the PR's own, plus the 520 gate)

- **Contract:** an old record without `notice` still loads.
- **Contract:** a `notice` from a caller that isn't the desktop is stored as a plain
  notification.
- **Host:** `harness.notice` answers `stored` only after the service acknowledges. Kill the
  service mid-call: the answer is `stored: false, retry_after_secs`, never `true`.
- **Host:** dedupe by (harness, notice_id). The same id from another harness is a separate card.
  `supersedes` naming another harness's card is ignored and listed in `clamped`.
- **Host:** `{text}` with no turn in flight is stored (#806's case). With a turn in flight it is
  still a status line.
- **Grant:** budget across two harnesses: the third interrupt of the day goes to the inbox. Quiet
  hours hold a reminder. Quiet initiative holds `discovery` even when it asks for `interrupt`.
- **Clamp:** a 46-word body is cut to 45 and `clamped: ["body"]` is stored. An unknown action
  kind is dropped and the card lands.
- **Feedback:** delivered only to the sender. It comes back after a host restart until acked.
- **Retention:** 600 `notify-send` calls do not evict an acknowledged notice. A dismissed
  notice past the TTL is in the archive.
- **520, driven:** the Mind's Stage A build sends a reminder with no turn in flight. The card is
  in the Lens and in `describe`, and a toast appears once. "Not now" reaches the Mind's poll.
  After a reboot, the card and the feedback cursor are still there.
