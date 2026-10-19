# Calendar heads-ups: the OS raises them itself

16 Oct 2026. Builds on the initiative inbox (#831, `design/initiative-inbox-2026-10-16.md`).

## What Pranab decided

- **The OS raises calendar heads-ups, not a mind** (option (a)). The Mind won't read the calendar
  while no turn is running (E.INIT1 amendment, yantrik-mind d0593f4). The OS owns the calendar,
  knows when the desktop is locked, and owns the budget and the person's controls.
- **The early heads-up is on by default, with a one-tap off.** This is the `calendar` family's
  default from the four families.
- **Only events with a location get the early heads-up**, 30–90 minutes ahead, e.g. "Dentist at
  15:00, Elm St, in 60 minutes". Every other timed event gets only its own reminder.

The Mind adds what only it knows (e.g. "you asked me to bring the passport") inside a turn the
person starts, through `upcoming`. Nothing in this spec gives a mind new authority.

## What exists today

`services/notifications-service/src/reminders.rs` re-reads the calendar's files every tick. It
announces each timed event once, at the event's `reminder_minutes`, 10 by default when the event
doesn't set one. All-day events are never announced. It keeps one record per event and start
time (`Sent`, on disk in the calendar directory), so a moved event is news again. The card is a
plain notification, app "Calendar", with no Notice family, no action and no controls.

## The two announcements

| | The event's own reminder | The early heads-up |
|---|---|---|
| Which events | every timed event (unchanged) | timed events whose `location` is non-empty after trimming |
| When | `reminder_minutes` before the start (unchanged) | 60 minutes before the start |
| Skipped when | (never; unchanged) | under 30 minutes remain (late boot, event added late); or the event's own `reminder_minutes` is 30 or more, since that reminder is already early |
| Family | `reminder`, an obligation: the person set it, and today it always arrives | `calendar`, initiative |
| Budget | none (as every obligation) | spends one of the 2 a day; held by quiet initiative, inbox-only and off |
| Quiet hours | held, as every family | held, as every family |
| Card | "Dentist in 10 minutes", "Starts at 15:00 on Friday 16 October" (unchanged wording) | "Dentist at 15:00, Elm St", "In 60 minutes. It has a place, so you may need to leave early." |
| `why` | "You set this event to remind you 10 minutes before." (or "Calendar events remind you 10 minutes before." when the event left it at the default) | "This event has a place, so it may need travel." |
| Action | `open_event` with the event id | `open_event` with the event id |

- An event without its own `reminder_minutes` still gets the default 10. That is today's
  behaviour, and moving it under the budget would silently drop reminders.
- **One card per event.** When the own reminder fires and the event's early card is still
  undismissed, the reminder `supersedes` it. The person sees the newer card, and the inbox doesn't
  hold two cards for one dentist.
- The location is the person's own text. It goes through `notice::check`'s clamps like any notice
  text, and is never parsed, looked up or sent anywhere. There is no travel-time estimate, because
  there is no map service and no egress for one.

## Who sends it

- One code path: both announcements go through `Store::add_notice`, called in-process (as
  `announce` calls `add_from` today). This is the same grant order, the same sidecar records and the
  same feedback as a mind's notice. Do not write a second grant path.
- The harness is a **reserved id `desktop`**, reserved at `open` for the service's own uid, as the
  Mind's id is. No socket caller can bind it, so no harness can post as the calendar or read its
  feedback. Name "Calendar".
- **Notice ids are deterministic:** `cal-reminder:<event id>:<start>` and
  `cal-early:<event id>:<start>`, with `os_id: false`. Then a restart that re-announces is answered
  as a duplicate, and `supersedes` can name the early card. `Sent` stays as well (keyed by event,
  start and kind; read an old file's entries as kind `reminder`), because records are pruned and
  ids alone would re-announce after a prune.
- **Rate caps:** `desktop` notices are not counted in, or refused by, the per-account and
  every-harness caps. A busy calendar must not crowd out minds, and a flooding mind must not block
  a reminder. Their volume is bounded by events, and `add_event` already has its burst cap (#332).
  They do count toward the obligation interrupt limit per hour, so a calendar full of 10:00
  meetings still waits in the inbox past 4 interrupts an hour.
- **Feedback** is recorded on the notice, and `never_family` and `inbox_only` change the settings
  as for any family. It is **not queued** for a harness, because nothing polls as `desktop`. A
  queue nobody drains would push minds' feedback out of the shared cap.

## Controls (unchanged UI, now reaching calendar)

- **Settings → Minds → Initiative → Calendar: On / Inbox only / Off.** That is the one-tap off
  for early heads-ups. "Never this kind" on an early card sets it to Off.
- The event's own reminder is controlled where the person set it: the event's reminder field.
  "Never this kind" on a reminder card sets `reminder` to inbox-only (the obligation rule from
  Stage A), which is per family, not per sender, because the OS is one assistant. The card's own
  `why` says what set it.
- Quiet initiative holds early heads-ups. Own reminders still arrive.

## The locked desktop and Private mode

- Toasts never draw on lock, login, boot or onboarding (`wire/notifications.rs`, `locked_screen`).
  A granted card that arrives while locked waits in the notification centre and is not toasted
  later. A heads-up shown after its moment is noise.
- Private mode changes nothing here. This is the person's own machine reading its own calendar
  files in the notifications service, as it does today. No mind is involved and no door is opened.

## Not in scope

- Travel time, maps, or any network use.
- Any mind reading the calendar while idle (blocked by design, E.INIT1 d0593f4).
- Heads-ups for all-day events.
- Changing `reminder_minutes` defaults or the Calendar app's reminder UI.

## Tests

- **Pure `due`:** for an event with a location, 61 min out → nothing; 60 → early; 29 → no early
  (the own reminder only). For an event with no location, or with spaces only → never early. An
  event with `reminder_minutes = 45` → no early. All-day → neither. A moved event → both again.
- **Grant:**
  - the early card spends the budget, and a third early card in a day goes to the inbox;
  - calendar Off → stored hidden; inbox-only → inbox; quiet initiative → inbox;
  - quiet hours hold both;
  - the own reminder never spends the budget.
- **Supersede:** the reminder dismisses the event's undismissed early card. A dismissed early card
  is left alone.
- **Identity:** a socket caller attaching as `desktop` is refused. `desktop` feedback is not
  queued. Minds' rate caps don't count `desktop` notices.
- **Restart:** a restart inside the window re-announces nothing (both from `Sent` and from the
  duplicate id). An old `Sent` file reads.
- **Driven test on VM 520:**
  1. Add "Dentist, Elm St" 65 minutes out and "Standup" (no location) 65 minutes out.
  2. Check the early card for the dentist at about 60 minutes (toast, Calendar chip, Open event
     works) and none for the standup.
  3. Check the dentist's own reminder at 10 minutes replaces the early card, and the standup's
     reminder arrives.
  4. Set Calendar to Off, add another event with a location: no early card, but its reminder
     arrives.
  5. Put the evidence on the PR, then roll back.

## Security notes for the review

- The reserved `desktop` id: binding, spoofing, and whether its feedback can leak.
- The location text in cards: clamped, never interpreted.
- No new socket, door, wire method or egress.
- The cap exemption must not open a flood path: only the in-process calendar loop posts as
  `desktop`.
