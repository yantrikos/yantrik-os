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

   *Built for foreign windows and our own apps' accessibility trees, 15 November 2026 (#1256):*
   `a11y.now` answers from a model the service keeps current, and `a11y.since` waits on a ring of
   changes. Measured on VM 520: a dialog opening is told 51 ms after the application shows it,
   listening costs no CPU while the screen is still, and 4,000 signals from one window in 2.4 s
   become 18 events. It does not share the run event spine yet, and the browser's own channel
   (CDP) and our apps' `describe` do not feed it.
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
