# Top bar and Today (plan PR 3.1): report

Branch `ui/top-bar-and-today`. Security boundaries touched: the control surface (`control_overlays.rs`,
`control.rs`): two new `safe` actions and two new `describe shell` fields. Needs the usual review.

## What a person sees
- A 36px opaque #101417 bar with a 1px #343B41 edge (tokens `bar-bg`, `bar-border`).
- Left: the Yantrik mark (opens the Lens), then ONE chip, "● Minds", plus "· 2 need you" in amber when minds
  have unresolved requests (counts come from the Agents workroom's own `request_minds`/`needs_count`, which come
  from the approval cards, questions and waiting jobs). Tooltip "1 mind has 3 requests". Click opens Agents on
  Needs you, or on the workroom when nothing waits.
- Centre: "Fri 2 Oct · 14:32", centred on the screen (measured: 639 vs 640 at 1280, 959 vs 960 at 1920).
- Right: mode chip, a moon when Do Not Disturb is on (click turns it off), network, sound (only with an audio
  device; wheel nudges, middle click mutes), battery (only with one), Quick Settings. 18px icons, 8px gaps.
- Click the clock or press Super+N: Today, a 380px popover centred under the clock: date, DND switch
  (YToggleTile, accent when on), the month (the Calendar app's grid, now the shared `MonthGrid`), today's events
  from the calendar service, the five newest notifications with their buttons, "All notifications".
  "Reading the calendar…" and "The calendar didn't answer…" are real states, not an empty day.

## Removed from the bar
CPU/MEM; the companion dot and word "Yantrik"; project pill; privacy/provider chip; active-mind chip; whisper and
queue counts; unread badge; INC and DND text badges; free-AI "clipboard history paused" chip; the power button.
Kept, only while true: the red "no model" and "minds can't reach" notices (a calm bar over a dead assistant
would lie).
Where each can still be reached: CPU/MEM in Settings/System monitor; power menu by Super+Escape and the command
palette. **No other home today** for the privacy chip, INC, unread count and clipboard-paused chip: decide if
they belong in Quick Settings or Today (notification polish is PR 3.2).

## Underneath
- `components/status_bar.slint` 749 -> ~200 lines; new `bar/{minds_chip,bar_clock,mode_chip,sound_indicator,
  notice_chip,today_state}.slint`, `popovers/{today_popover,today_notification}.slint`, `month_grid.slint`
  (extracted from `calendar.slint`; the app uses it too).
- `yantrik_ipc_contracts::calendar::month_grid` is the one grid builder for the Calendar app and Today.
- `wire/today.rs`: reads the month on a worker thread (2s timeout), never the UI thread; no timer.
- Clock: `wire/timers.rs` now one single-shot timer re-armed on each minute boundary (was a repeating 30s timer).
  Removed the bar's 200ms "thinking" pulse timer with the companion dot.
- Agents counts are published from the existing 250ms Rust tick once a second while Agents is not open.
- `status-bar-height` 36px, `bar-icon-size` 18px (YIndicator everywhere), rc.xml `<margin top="36">`.

## Control surface (parity)
- `act shell open_today` / `close_today`: `safe`; `open_today` calls `hold_windows("open_today")` first, raises the
  shell, answers with the observed `open` and `raised`. Added to the source-scan list in `card_watch.rs`.
- `describe shell`: `today {open, close_with}` and `bar_minds {label, tooltip, minds_needing_you, requests, opens}`.
- Super+N in rc.xml with a cheat-sheet `@help` entry. `docs/app-control.md` updated.

## Verification (what I ran)
- `cargo run --profile fast -- ... verify-top-bar` (tests/ui-preview): PASS. Asserts opaque bar and edge,
  amber only with requests, teal dot, clock centred at 1280 and 1920, chip/clock/switch/notification-button/
  "All notifications" clicks reach their callbacks, Today centred, under the bar, above the dock at 800 and 600px
  tall, calendar-unavailable differs from an empty day, 0 redraws settled with Today open.
- Also PASS: verify-battery, verify-network, verify-qs-levels, verify-calendar (no panic), verify-cheat-sheet,
  verify-dock, verify-kit-controls, verify-idle.
- `cargo test -p yantrik-ui-kit`: 15 passed (including `an_idle_window_stops_drawing`).
- `cargo test -p yantrik-ipc-contracts calendar`: 4 passed. `cargo check -p yantrik-calendar`: ok.
- `cargo test -p yantrik-ui --bin yantrik-ui`: 1163 passed, 2 failed. One was mine (a source-scan of the old bar
  text in `wire/harness.rs`), fixed and re-run: passes. The other,
  `harness_install::a_coloured_installer_reaches_the_row_as_plain_text`, prints "nvm" ahead of its output in this
  sandbox; it is about the installer's stderr and I did not touch it. I did not re-run the whole suite after
  the one-test fix.

## Not run / not verified
- `verify-bar-overlays` FAILS at its last stage: "the lock screen shows no panel: ~32k pixels changed" (setting
  `power-menu-open` on screen 3). The Quick Settings, power and clipboard stages before it pass. I changed that
  test only to open the power menu by flag (the bar has no power button now). I did not build `main` to check
  whether the lock-stage failure predates this branch; its cause is not established. Needs a look.
- Not run: `cargo build --workspace --locked`, `cargo test --workspace --locked`, the CONTRIBUTING selftests, any
  real-machine check (calendar service, minute-boundary timing, labwc margin, Super+N).
- `tests/ui-preview/preview.slint` did not compile on main (`OsdWindow` was not exported for `osd_tests.rs`); I
  added the one-line export. `verify-osd` itself was not run.
- Today's panel uses `Theme.bg-elevated`, which is blue-grey in dark mode, not the render's #151A1E charcoal
  (a kit-wide YPopover colour; not changed here). The month is Sunday-first (shared with the Calendar app); the
  render is Monday-first. With 5 notifications the list scrolls on an 800px screen.
- The Today grid's days are not clickable and there is no month navigation.

Screenshots: `review/ui-top-bar/` (calm and needs-you desktops, bar strips at 1280/1920, Today, DND on, 1024x600,
calendar down). Generated with `verify-top-bar`.
