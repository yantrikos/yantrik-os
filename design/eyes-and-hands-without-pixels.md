# Eyes and hands without pixels

Measured on the live machine, 17 September 2026, build v0.1.0-179-g6fc8b13. Every number and
quotation below came from that machine, not from reading the source.

The goal: a desktop whose mind sees every app and drives every app without looking at a
screenshot — safely, and fast enough that the seeing is not the slow part. The same model that
lets a mind see should be the one that makes the UI keyboard- and screen-reader-quality, because
they are the same question asked twice.

## What was already there, switched off

The accessibility bus was running the whole time: `at-spi-bus-launcher` (pid 763),
`at-spi2-registryd` (pid 775), `org.a11y.Bus` on the user bus, `libatk-bridge2.0-0t64` installed.
The registry listed exactly one application — `lxpolkit`, with no windows.

One property explains it. Every toolkit checks `org.a11y.Status.IsEnabled` before publishing
anything, and on this machine:

```
IsEnabled            b false
ScreenReaderEnabled  b false
```

Setting `IsEnabled` to true, and relaunching Chromium with `--force-renderer-accessibility`:

```
:1.26  yantrik-ui                  (the shell itself)
:1.27  yantrik-notes
:1.25  yantrik-calendar
:1.24  yantrik-email
:1.23  yantrik-download-manager
:1.28  yantrik-terminal
:1.29  Chromium
```

and the trees have real content:

```
Chromium        application → frame "data:text/html,<h1>Yantrik eyesight test…"
                  → panel → button Minimize / Maximize / Restore / Close
yantrik-notes   → Notes → "Desktop smoke test", Export, Import, Structure, Summarize, Template
```

So our Slint apps have accessibility already — they were waiting for a switch nobody set. The
desktop's own `a11y` service, meanwhile, answers `a11y.status` with "GTK, Qt, Chromium and Firefox
publish their widget trees here" and `a11y.windows` with `{"count": 0}`; its binary contains no
AT-SPI code at all. It was a claim, not an implementation, and with the switch off nothing
contradicted it.

Two more measurements that shape the design:

- **The same question, two ways:** `yos describe notes` as a subprocess takes **132.1ms**; the
  identical call on the unix socket takes **1.1ms** (0.8ms warm). Every action an agent takes today
  pays a 120× tax for process spawn.
- **Walking a tree with `busctl` costs 500–700ms** for a shallow walk — because each node is
  another process. A client holding one D-Bus connection is the difference between "eyesight" and
  "a slideshow".

## The shape

**Four sources, one model.** Nothing universal exists, so do not pretend otherwise:

| Surface | Channel | Why |
|---|---|---|
| Our apps | their own `describe` / `act` | It is the app's real state, not a guess from widgets |
| Our apps' UI | AT-SPI via Slint | Roles, focus order, labels — what a person with a keyboard uses |
| The browser | CDP | The DOM is richer and more stable than the browser's widget tree; the companion tools already speak it |
| Other foreign apps | AT-SPI | GTK and Qt publish here when the switch is on |

Anything else — canvas apps, games, remote desktops — is **declared unsupported**, not silently
half-seen.

**Invalidation, not a transcript.** AT-SPI is a firehose: caret moves, focus churn, text deltas.
A mind does not want the firehose; it wants to be told what stopped being true. Two layers: ingest
raw signals into a bounded cache, and publish semantic invalidations — `focus_changed`,
`window_removed`, `subtree_dirty`, `operation_completed` — coalesced per object over a short
interval. Bootstrap with a snapshot plus a cursor. On overflow or reconnect, say "gap, resync"
rather than pretending the stream was complete. Sequence numbers mean ingestion order, not causal
order across applications.

**Identity is not a title.** An AT-SPI window is not an authoritative compositor surface on
labwc, and titles collide and change. Until there is trusted compositor-side mapping, the model
reports uncertainty rather than claiming which window is in front.

**Grading a verb is not grading its consequence.** The dangerous sequence is concrete: an agent is
allowed to fill a form, and then `invoke`s an accessible button labelled "Allow" in another
window. The verb was permitted; the consequence was consent. So:

- The broker derives authority from the caller's identity, the grant's scope and the target's
  identity — never from a grade the caller declares.
- Read, text-entry, activation and submission are separate permissions. An unknown foreign action
  is unknown impact, not "probably safe".
- **The desktop's own consent UI is outside automation authority.** Consent arrives through a
  trusted input path; an accessible label cannot authenticate it.
- Accessible text is untrusted content — never instructions — and may contain secrets even when
  the widget is not a password field.

## The order, with what "done" looks like

1. **Stop the false claim, turn on the switch.** The session sets `org.a11y.Status.IsEnabled`, and
   the browser launches with renderer accessibility. `a11y.status` reports what is actually
   connected — and distinguishes "no bridge", "app not responding" and "empty tree". *Done:* the
   seven applications above appear, and the service says so honestly. Hours.
2. **A real client, and a fast one.** Implement the a11y service as an AT-SPI client over one
   persistent connection, with node and time budgets, cancellation and partial-result markers.
   Same move for the control surface: a persistent session and a bounded `snapshot` index instead
   of a subprocess per question. *Done:* `describe` of the focused window under 50ms warm, and the
   132ms spawn tax gone. Days.
3. **One vertical slice of hands, not eleven schemas.** Displayed control → accessible node →
   permitted action → observed state change, for exactly one app and one browser page. *Done:* fill
   and submit a form with no screenshot, and the OS can show afterwards what was touched. Days.
4. **Events.** The invalidation stream above, sharing the run event spine. *Done:* a mind learns a
   dialog opened within 100ms without polling, and a slow subscriber gets a gap marker rather than
   silence. Days.

   *Built for foreign windows and our own apps' accessibility trees, 15 November 2026 (#1256,
   #1257):* `a11y.now` answers from a model the service keeps current, and `a11y.since` waits on
   a ring of changes. Measured on VM 520 at an earlier head of #1257: a dialog opening is told
   12 ms after the application shows it, and listening costs no CPU while the screen is still.
   Text changes, the budgets and the model's shares have changed since then (see below), and
   the bench has not run on that code. It does not share the run event spine yet, and the browser's own
   channel (CDP) and our apps' `describe` do not feed it.

   **Decided with it:**
   - *Who may read the stream.* The person's own programs only, as with `a11y.windows`: the
     service's socket opens no mind door, and the companion has no tool for `a11y.now` or
     `a11y.since`. A Mind is not given the stream until two things exist: a per-window consent
     rule, and the secret-name filter of #1247 on every title and element name it carries. A
     pushed timeline of focus names and titles is a different thing from a look someone asked
     for, even where each carries the same words.
   - *What it never says.* The text that changed; anything about a focused password field, or
     about a focused element whose role could not be learned (it is treated as a password): for
     the whole application that field is in, not only the element, and of every kind (text,
     children, names). When the focus is not known, after a gap or across the private time,
     that holds for every application until the next focus signal says what it is. Anything from
     the time the person was private, which is told afterwards as one gap; a change held through
     a private time shorter than the service's interval between looks is forgotten too, because
     the service reads afresh whether the person is private when a change begins to be held (this
     is covered by a test with a fake bus, not yet run on a live one). How many changes an event
     stands for is not told, and one text-changed signal is one keystroke, so typing is told as
     activity and not as keystrokes: "text is changing in this window" falls due on a grid
     shared by every window, the multiples of 2 s from the start of the service, at the first one
     at least 2 s after the change, and every event with text in it is stamped to its step.
     Children and name changes from the application that has an editor focused take the same
     lane, because a rich editor announces typing that way. A reader learns *that* someone typed
     in a window, and in which 2 s step; not when in it, how fast, how much or how often. What
     it still sees: that the window was changing and that the application has an editor
     focused (an unfocused window's changes are the quick lane's, millisecond by millisecond);
     a window retitled because it was edited is told at once like any retitling; and a title
     set during the private time that is still current is in `a11y.now` afterwards, because it
     is the present state of the window, as `a11y.windows` has always said. Whether Chromium
     and Firefox editors announce typing as children is not proven live.
   - *Who is believed.* Any program can join the accessibility bus. That a connection left is
     believed only from the bus, that an application left only from the registry, and that a
     window came or went only from the connection that owns it.
   - *Whose time.* Every question to an application has a time limit and is charged for the time
     it took, to its connection (50 ms of waiting a second, and 20 questions a second) and to
     everyone together (250 ms a second); one that answers slower than 100 ms three times, or
     does not answer, is left alone for 10 s. When everyone together has used the total, a
     connection that owes less than a tenth of its own share is still let in, as long as the
     watcher is not more than a second behind with what that connection is expected to cost
     (the cost of its last answer), so connections that each keep within their share cannot
     keep a quiet one out; there is no cap on the number of questions of all connections
     together. A window whose question was refused is kept (one slot per connection, 64 in all),
     asked about again within 100 ms of being allowed, and a listing, which asks for nothing
     from the budget, is asked for: the window is found within about a second of its being
     refused, at worst. Nothing is asked of a connection the model has no window of and the
     last listing did not name; a window announced by such a connection wants a listing
     instead. Listings ask applications sixteen at a time under 400 ms each and 1 s in all, skip
     the slow and those that owe their share, count an answer over 25 ms as a strike, ask at
     most 256, and are charged to everyone and followed by a pause of three times their length
     (at least one second): they take a quarter of the watcher's time at most. `a11y.describe`
     and `a11y.act` stop waiting after 10 s. The model keeps at most 64 windows for one
     connection and 512 in all. When it is full, a process (by pid; connections with no known pid
     share one) with fewer than its equal share still gets one, by closing the newest window of
     the process with the most, the one learned last among equals; never the front window, the
     focused window, or the newest window of an application that has either. Five hundred
     connections of one program are one share.
   - *The name.* These two methods stay `a11y.now` and `a11y.since`: they are what the
     accessibility bus knows. `screen.now` and `screen.since` are kept for the model that joins
     this source with our apps' own `describe`, the browser and the compositor.
5. **Roles and labels in our own UI.** No `.slint` file sets an accessible role today, so our apps
   publish names without roles. This is the same work as keyboard navigation and screen-reader
   support: one model, two consumers. *Done:* every interactive element in the shell and one app
   has a role and a label, and tab order matches what the eye sees. Days, per app.
6. **Budgets in the gate.** p95 latency for describe and act, and an idle-CPU budget — an idle
   Terminal currently burns 1.5 cores, which no functional test noticed.

## The trap

Generating `describe`/`act` for the remaining apps from each app's state model and calling
accessibility solved. State describes data; it does not describe rendered controls, focus order,
modal ownership, or what is offscreen. Eleven generated schemas would leave keyboard users and
minds looking at different machines. Earn the generator with one vertical slice first.
