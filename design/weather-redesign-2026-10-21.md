# Weather redesign brief (21 Oct 2026, revision 2)

Step 7 of [the app framework](app-framework-2026-10-21.md): the second of Weather's two PRs. The first (#898) moved the app onto the framework with nothing drawn changed. This one changes only what is drawn, so a regression is never ambiguous between framework and presentation. It follows [the art direction](art-direction-2026-10-20.md) row by row; the bar is the Alt+Tab switcher (#876), which Pranab named as the quality he wants across the OS.

Revision 1 was critiqued by GPT-5.4 as the art director (§8); this revision takes every correction. The decisions marked ★ are Y's under the 20 Oct delegation and are Pranab's to overturn.

## 1. What is on screen today

VM 520, 1280×800, dark, the rail open (`w520/weather-898.png` in the session's evidence folder; the kit's `WeatherProbe` draws the same screen at 1000×720):

- The base window: title bar with the app icon, "Weather", the place and "Updated just now" under it, a C/F segmented switch, **Change** and **AI Insights** buttons, a refresh icon button, the window controls.
- A red-tinted notice strip when the reading came from Open-Meteo directly.
- **The hero card**: a 62px light-weight temperature with a 54px tinted icon, condition and "Feels like" under it; beside it six **metric tiles** (humidity, wind, UV, visibility, pressure, cloud cover), each a bordered card with an icon, an uppercase 10px label and a 16px semibold value. A card holding cards.
- **Hourly forecast**: an eyebrow with an orange rule, a source line and a timezone line; a bordered card of eleven columns, the current one outlined in blue, each with time, icon, temperature and a small amber bar (the hour's place in the day's range), a precipitation percentage under some.
- **5-day forecast**: an eyebrow with an amber rule; a bordered card of rows: day / icon / precipitation / low / a blue-to-amber gradient bar / high.
- Below the fold: alerts, air quality, saved places (a panel that opens from **Change**), the AI panel (opens from **AI Insights**).
- The rail: CONTEXT rows (the conditions, "last updated") and NEXT (the one suggestion, "What should I plan for?").

What a person comes to this screen for, in order: *what is it like outside right now; what will it be like later today; what about the next days; is anything dangerous.* The present screen answers all four but makes them compete: five bordered boxes of similar weight, three accent colours (blue, amber, orange), a red strip before the first number.

## 2. Where it breaks the art direction

| Rule | Break |
|---|---|
| §2 one ground, elevation is overlap; never cards inside cards; a fill change *or* a hairline | Hero card holds six bordered tiles; hourly and daily each in a bordered card with border and fill |
| §2 gold is Yantrik's, blue is attention, neither decorates | Amber bars on every hour, a blue-to-amber gradient on every day, orange and amber eyebrow rules |
| §2 Barlow scale; tabular numerals; no shrinking text for density | 10px uppercase tile labels; a 62px one-off (the app's one design-lint entry); figures not tabular |
| §2 radii; a filter is not a capsule | The place chip is a pill |
| §4.3 status furniture, §4.5 nesting, §4.6 decorative colour, §4.7 tiny type, §4.10 fake certainty, §4.12 borders on every control | All six |
| §H chrome: no second band under the title bar | The hourly's source line and timezone line |
| §G: typography and honest plots over chart boxes | The hour bars and day gradients show a position in a range nobody asked about |
| Release gate: is the person's content the strongest thing? | The strongest thing today is the box structure |

## 3. The redesign

**One hierarchy, no exceptions beyond it:** (1) danger, (2) the reading, (3) today, (4) this week. Everything sits on the ground, set in the Barlow scale, separated by space and one hairline where two regions meet. Blue appears in Weather's content in exactly three places: the current hour, the refresh affordance once the reading is stale, and the selected place in the popover. Gold appears only in the rail, which the app does not draw. The weather icon's day/night tint is the app's one colour, and it is subordinate.

### 3.1 Alerts (danger first)

The only thing above the reading. The kit's `NoticeBar`: Level 1 fill, a 2px warning or danger leading rule, title 14 / 600, body 14 / 400, 16px padding, 12px between bars; no icon unless the kit's bar already has one. **At most two bars**; a third and beyond fold into one `YListItem` row, `2 more alerts`, which opens them. 24px below the last bar before the reading.

The Open-Meteo notice is not an alert and not a strip: it is the freshness line's business (3.2).

### 3.2 The reading

Ground, no card, 24px window padding (§2).

- The icon at **36px** in its day/night tint, 12px left of the number, on the number's baseline cluster. In the light theme, where the tint fails contrast on `#F4F6F9`, the icon falls back to `text-secondary`, never to the warning role (no false urgency).
- The temperature at **36 / 400, tabular**, the unit at **20 / 400** beside it. No new display size: 72px was the old billboard with a token on it (★ the one `px-font` design-lint entry is removed by dropping the size, not by promoting it).
- The condition line at **16 / 400 `text-secondary`**, 12px under the reading: `Partly cloudy · Feels like 15°`. One line, tail-truncated.
- **The measured pairs**, 24px under the condition line: a `MeasuredPair` kit component (label 14 / 400 `text-secondary`, 8px, value 14 / 600 tabular `text-primary`) laid out as a **fixed grid: three pairs per row and two rows** at normal widths, **two per row and three rows** when the content is under 560px (a third-screen window; at 1000×720 the rail leaves 720px, which holds three). 24px between pairs, 12px between rows. Labels never wrap; values are single-line. The six: `Humidity 48%`, `Wind 11 km/h WSW`, `UV 5`, `Visibility 16 km` (a number, or `—` when the source gives none; a qualitative word belongs on the condition line, not among measurements), `Pressure 976 hPa`, `Cloud 58%`. No icons.
- **Freshness**, 12px under the pairs, 12 / 400, one line, by a strict policy and otherwise absent:
  - under 15 minutes and the service answered: **no line**;
  - 15–30 minutes: `Updated 19 min ago` in `text-secondary`;
  - over 30 minutes: `Last reading 48 min ago` in the warning text colour, and the title bar's refresh button takes the attention tint;
  - a reading that came from Open-Meteo directly: ` · Open-Meteo fallback` appended to whichever of the above shows (and the line shows even under 15 minutes). No sentence about who did not answer.

  The title bar's subtitle keeps only the place.

### 3.3 Today

Heading `Today` at 16 / 600, 32px above, 12px to content. No eyebrow, no source line, no timezone band. When the place's timezone differs from the machine's, the heading line carries ` · Bentonville time` at 12 / 400 `text-secondary`.

A single row of columns on the ground, **64px** wide at 100% (56px only when no visible column carries a precipitation figure), 8px top and bottom padding inside a column: time 12 / 400 (`20:00`; `Now` for the current hour), icon 20px `text-secondary`, temperature 14 / 600 tabular, and a fourth slot for precipitation at 12 / 400 `text-secondary` shown only at 10% or more; the slot is structural spacing, never a blank text. The current hour: a 2px accent leading rule and the Level 1 accent fill, nothing else. Hover on another column: the Level 1 neutral fill only. Keyboard focus: the kit's focus treatment, no second border. As many columns as fit, at least six; the row scrolls horizontally under six, opened with the current hour visible and two columns before it where there are two. No bars.

### 3.4 This week

Heading `This week` at 16 / 600, 32px above, 12px to rows. Rows on the ground, 44px, a 1px hairline at 7% between rows, no border around the group; each a `YListItem` so hover and focus come from one place. The first row is `Today`. Columns: day 56px at 14 / 600; a 24px icon slot with a 20px icon; low in a 32px slot, right-aligned, 14 / 400 tabular `text-secondary`; a 12px en dash; high in a 32px slot, right-aligned, 14 / 600 tabular `text-primary`; precipitation in a 56px slot, right-aligned, 12 / 400 `text-secondary`, only when non-zero. No gradient bar, no plot.

### 3.5 Air quality

Not a measured pair: it is not a weather measurement and its scale is opaque. It appears as a `NoticeBar` only when unhealthy (3.1's rules), and otherwise as one secondary line under the freshness line, `Air quality 42 · Good`, at 12 / 400 `text-secondary`, only when the source gives it.

### 3.6 Places and units

★ **Change** is removed. The place name in the title bar is the popover's anchor: the kit's `YPopover` under it, saved places as `YListItem` rows (the active one with the accent leading rule, not also a fill), a `YInput` to add, remove on hover, keyboard through the rows. No panel below the fold. C/F stays the kit's `YSegmented` in the title bar. Refresh stays an icon button in the title bar (the data cannot promise sensible refresh on its own) and takes the attention tint only when stale.

### 3.7 AI

★ **AI Insights is removed from the title bar**, and the app's own AI panel with it. The rail's one suggestion, `What should I plan for?`, is the entry point; the answer is the rail's card. When the rail is hidden (art direction §C: hidden by default), activating a Weather suggestion is impossible, so nothing is lost; if a future title-bar action must ask the Mind, it opens the rail and focuses the card, and never an app-local panel. `Command::Explain`, `Explained`, `DismissAi` and the `ai` part leave the model: the prompt lives in `rail.rs` alone.

### 3.8 Text behaviour

The condition line, the freshness line, headings and daily rows never wrap: tail-truncate. Measured-pair labels never wrap; values are single-line. The reading's number never truncates. Checked at 125% and with the longest strings in the fixtures (`Wind 118 km/h WSW`, `Pressure 1 012 hPa`, `Visibility 16 km`).

### 3.9 Light theme

Every colour is a token, so the light theme comes from §2's table; the one check is the icon tint's contrast (3.2).

## 4. What does not change

The model's data, the intents, `describe`, the rail's source, the file, the timers: all of #898. The redesign touches `ui/weather.slint` and `view.rs`, removes the AI commands and the `ai` model part, adds a `fresh: Freshness` projection to the model (`Fresh`, `Updated(minutes)`, `Stale(minutes)`, with `fallback: bool`), and adds to the kit only `MeasuredPair` (a label/value pair with the fixed widths above), which System Monitor and Network will take next.

## 5. Renderer limits (from the Alt+Tab work)

Software GL on 520: opaque rectangles, clipping, borders and translation only. No blur, no gradients on rounded rectangles (#172: square corners), no shadows inside the content. A hover is a fill change. Nothing animates except the popover's open (120 ms ease-out translate, §2 motion); nothing runs a timer while idle (the kit's `no_window_asks_to_be_redrawn_for_ever` asserts it).

## 6. Evidence before merge

On 520, from the same binary swap as #898's evidence: 1280×800 dark and light, 100% and 125%; 1000×720 with the rail open (the #220 case) and half-screen (the #505 case); keyboard through the places popover; `yos describe weather --json` with the same keys as #898's (the `ai` keys were never in it); frame timings idle and while the hours scroll. Screenshots in the PR, the design-lint baseline at zero entries for `apps/weather`.

## 7. Decisions taken (★)

1. No display-size token: the temperature is 36 / 400.
2. **Change** removed; the place name opens the popover.
3. **AI Insights** removed; the rail is the one entry point for the Mind in Weather.
4. The hour bars, the day gradients and any inline plot are gone from this release.
5. Air quality is a secondary line, a `NoticeBar` when unhealthy, never a measured pair.

## 8. Critique 1 (GPT-5.4 as the art director, 21 Oct 2026), condensed

> The brief is directionally right, but still too self-impressed by its own decluttering. It swaps obvious card clutter for quieter special casing.

Its corrections, all taken in this revision: 72px is a one-off laundered into a token — use 36/400 with the unit at 20/400 and a 36px icon; freeform measured lines break on real data — a fixed pair grid (3×2, 2×3 constrained) with a `MeasuredPair` component, labels never wrapping, `Visibility` a number not prose; the freshness line carried backend drama — a strict policy with no line under 15 minutes; the title bar is a toolbar — drop **Change**, the place name anchors the popover; the hour row needs 64px columns, a defined initial scroll, a timezone note in the heading when zones differ, and a structural (not textual) precipitation slot; the daily row needs fixed slots and no `0 mm`; air quality is not a core metric; alerts need a stacking rule (two, then `n more`); define overflow behaviour; fewer exceptions — one hierarchy; and the AI answer must never land somewhere invisible (open the rail, or better, drop the button). Its answers to the open questions: drop every bar and plot; 72px is too large; neither prose lines nor a four-column table but the fixed grid; open the rail for the answer, and prefer removing the app-specific button.
