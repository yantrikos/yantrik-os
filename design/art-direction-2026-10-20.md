# Yantrik OS art direction (20 Oct 2026)

GPT-6 Astra, from the brief in this folder's sibling and eight screenshots of VM 520, after it art-directed Alt+Tab (#876), which Pranab called "the quality of UI I want in our OS". This is the spec the shell's visual work follows; each change cites its row in section 5.


The strongest thing in these screenshots is not the AI branding. It is the confidence of Alt+Tab: real content, a dark ground, one clear selection. Notes comes closest because it leaves room for the work.

**Make that restraint the identity of the whole OS.** Yantrik should feel observant, composed and exceptionally legible—not like a dashboard constantly proving that an AI exists.

The biggest visual improvement is subtraction: **give the person their screen back.**

---

## 1. Design language

### The feeling: night architecture

Deep navy planes, crisp typography, generous empty space and small, precise signs of attention. The photographic desktop supplies atmosphere; application content supplies colour. The shell does not compete with either.

### The recognisable idea: the living edge

Yantrik’s distinctive element should be **a narrow edge that opens into useful context**:

- A selected item receives a short, soft-blue leading rule.
- The Mind lives at the right edge and opens inward when invited.
- A relevant event appears there once, then settles.
- Approval is a deliberate interruption, not another chat bubble.

This is a spatial language, not an effect. It needs only opaque rectangles, clipping, borders and translation. No blur, glowing particles or simulated intelligence through animation.

**Gold identifies Yantrik. Blue identifies your current attention. Neither decorates empty space.**

---

## 2. The visual system

All measurements below are logical pixels at 100% scaling.

### Ground and elevation

Use **one ground and three surface levels**. Elevation means overlap—not “this content deserves a card.”

Hex colours are `#RRGGBB`; alpha is specified separately. Shadow offsets are `(x, y)`.

| Level | Use | Dark fill | Hairline | Shadow |
|---|---|---|---|---|
| Ground | App content, full-screen destinations | `#0B121C`, 100% | None | None |
| 1 — Structural | Sidebars, title bars, grouped settings | `#111B28`, 100% | `#FFFFFF` at 7%, only at a structural boundary | None |
| 2 — Floating | Dock, desktop ask field, Mind panel, Lens, menus | `#172333`, 100% | `#FFFFFF` at 11%, 1px | `#000000` at 28%, blur 20px, offset `(0, 6)` |
| 3 — Interruptive | Approval dialogs, critical confirmation | `#1C293B`, 100% | `#FFFFFF` at 16%, 1px | `#000000` at 40%, blur 32px, offset `(0, 10)` |

Additional rules:

- Floating app windows use Ground content with a Level 1 title bar and **one outer Level 2 shadow**.
- Maximised windows have no exterior shadow or rounded exterior corners.
- Separate adjacent regions with **either a fill change or a hairline**, not automatically both.
- Never shadow rows, charts, text inputs or cards inside cards.
- Modal scrim: `#03070D` at 56%, no blur.
- Hover: white at 4% over the existing fill.
- Pressed: white at 7%.
- Selected row: accent at 12%, with a 2px accent leading rule.
- Keyboard focus: 2px accent outline, with a 2px gap. Do not rely on the selection tint alone.

**Do not fake glass across the interface.** The wallpaper currently showing through the workspace card and Mind panel makes both look muddier, not more premium. Opaque navy is the correct material here.

### Light theme

Light is not inverted dark. It is pale mineral paper with navy ink.

| Role | Light |
|---|---|
| Ground | `#F4F6F9` |
| Level 1 | `#EAEFF5` |
| Level 2 | `#FFFFFF` |
| Level 3 | `#FFFFFF` |
| Structural hairline | `#14253B` at 10% |
| Floating hairline | `#14253B` at 14% |
| Interruptive hairline | `#14253B` at 18% |
| Level 2 shadow | `#14253B` at 12%, blur 20px, offset `(0, 6)` |
| Level 3 shadow | `#14253B` at 18%, blur 32px, offset `(0, 10)` |
| Hover / pressed | `#14253B` at 4% / 7% |
| Modal scrim | `#14253B` at 28% |

Keep floating surfaces white; distinguish them through boundaries and shadows, not increasingly grey boxes.

### Corner radii

Use a small family:

- **4px:** keycaps, tiny badges.
- **8px:** buttons, fields, selected rows.
- **12px:** menus, meaningful cards, floating app windows.
- **16px:** Lens, expanded Mind, approval dialogs, desktop ask field.
- **Dock:** 16px top corners, **0px bottom corners**, flush with the screen edge.
- Circles only for actual avatars, status dots and the compact Mind mark.

Remove arbitrary pill shapes. A filter is not a capsule unless it belongs to a compact segmented control.

### Spacing and density

Base scale: **4, 8, 12, 16, 24, 32, 48, 64px**.

- Standard button: 32px high; 12px horizontal padding.
- Primary form action: 36px high.
- Icon button: 32 × 32px; 16–18px icon.
- Top-bar target: at least 32 × 32px.
- Navigation row: 36px.
- Single-line data row: 44px.
- Two-line content row: 64–72px.
- Window content padding: 24px.
- Large destination padding: 32px.
- Section gap: 32px; label-to-content gap: 12px.
- Card padding: 16px; approval card padding: 24px.
- Reading measure: 60–75 characters, not the full width of a maximised window.

Do not solve density by shrinking text. Remove redundant content first.

### Barlow type scale

| Size / weight | Use |
|---|---|
| 36 / 500 | Desktop greeting |
| 28 / 600 | Destination title, occasional empty-state heading |
| 24 / 500 | Important resource value |
| 20 / 600 | Dialog title |
| 16 / 600 | Section heading |
| 15 / 400 | Chat, memory statements, reading-oriented UI |
| 14 / 400 | Standard controls and list body |
| 14 / 600 | Row title, sender, selected navigation |
| 12 / 400 | Timestamp, secondary metadata |
| 11 / 500 | Keycaps and genuinely compact labels only |

- Body line height: 20px at 14px; 22px at 15px.
- Reading text: 16px / 24px.
- Headings: approximately 1.2× line height.
- Sentence case everywhere except established abbreviations.
- Default letter spacing: 0. Section eyebrows, when indispensable: 12px, 500, +0.8px.
- Use tabular numerals for clocks and resource values where the font supports them.
- Monospace only for code the person is intentionally viewing.

The current 9–11px metadata is a major quality problem. It makes the OS look like a scaled-down prototype.

### Colour roles

| Role | Dark | Light |
|---|---|---|
| Primary text | `#EDF2F8` | `#172437` |
| Secondary text | `#B0BDCE` | `#495B72` |
| Muted metadata | `#8D9EB3` | `#607189` |
| Disabled | `#65748A` | `#8794A5` |
| Selection / focus accent | `#8FB4E3` | `#315F96` |
| Success | `#8CC8A6` | `#276946` |
| Warning / approval | `#E3BD78` | `#825514` |
| Danger | `#EE989F` | `#B03645` |
| Brand gold | `#C9A66B` | `#846020` |

The brand’s soft blue remains the dark-theme accent. Light mode uses a darker functional counterpart for contrast.

**Accent discipline:**

- Blue means selection, keyboard focus or the object currently being acted on.
- Default primary buttons are high-contrast neutral: dark-theme `#E4EBF4` with `#111B28` text; light-theme `#20334D` with white text.
- Success, warning and danger appear only for real states, with a label or icon.
- Charts are neutral unless highlighting a selected series.
- Brand gold belongs to the Y emblem and brand moments—not every Mind response.
- App icons retain their identities. The “one accent” rule applies to shell chrome, not photographs, documents or app artwork.

Validate normal-sized text against WCAG AA in the rendered UI, particularly over hover and selection fills. Disabled text must never carry information required to make a safety decision.

### Iconography

Use one coherent outline family:

- 20px design grid; 1.5px stroke.
- Rounded joins, restrained geometry.
- 16px icons for dense controls; 20px navigation; 24px feature symbols.
- No emoji as interface icons.
- No coloured square behind every toolbar icon.
- Use the existing Y as the brand mark. Give the Mind one consistent symbol; stop alternating between Y avatars, turquoise rings and unrelated AI sparkles.

App icons: redraw as a coherent 32/48px family with simple silhouettes, two or three tones, and at most one shallow gradient. Remove glossy bevels and heavy internal shadows.

### Motion

One gesture per transition: **arrive from the relevant edge, then stop.**

| Interaction | Motion |
|---|---|
| Hover / press | 90ms colour transition |
| Selection change | 120ms fill/rule transition; no row movement |
| Menu opening | 140ms opacity + 4px translation |
| Mind / Lens opening | 220ms opacity + 16px translation from the right |
| Close | 160ms, reverse direction |
| Approval arrival | 180ms opacity + 8px upward translation |
| Pinned panel resize | 220ms width transition, only on explicit user action |

Opening easing: cubic Bézier `(0.2, 0, 0, 1)`. Closing: `(0.4, 0, 1, 1)`.

Never animate:

- Wallpaper.
- Idle Mind symbols.
- Resource values with rolling counters.
- Chat token arrival with individual fades.
- Notification rows endlessly rearranging.
- Automatic window movement in response to an unsolicited suggestion.

A working state can use a small progress indicator, but not an ambient “breathing” aura. Reduced motion uses immediate geometry changes and, at most, an 80ms fade.

---

## 3. Surface direction

## A. Top bar: one identity, one safety control

Keep it **36px high**, opaque `#0B121C`, with a 1px bottom hairline.

**Left**
- Y emblem, 18px, gold, inside a 36px target.
- Nothing labelled “Minds.”

**Centre**
- `Tue 20 Oct · 01:10`, 13px / 500.
- Clicking opens calendar and notifications.

**Right**
- One clearly named mode control.
- Network, volume, brightness, each in a 32px target.
- 12px right inset.

Replace “Minds: Auto” with language that describes permission:

- **Mind · Suggest only**
- **Mind · Act within limits**
- **Mind · Paused**

Map these labels to actual backend permissions; do not imply capabilities that do not exist. “Act within limits” opens a concise explanation of those limits and explicitly says **“Risky actions always need your approval.”**

The control remains visible when the Mind panel is closed. In immersive full-screen applications, provide a small non-layout-reserving safety capsule; its position must not obscure application controls.

No second model selector in the top bar. No teal “alive” dot when nothing is happening.

## B. Desktop home: let the lake be the desktop

**Remove the entire “Your workspace” card.** It duplicates the dock and occupies the photograph’s strongest area.

At 1280 × 800:

- Greeting group starts at **x=96, y=112**.
- Greeting: `Good evening, Pranab`, 36px / 500. Use the appropriate local-time salutation.
- Remove the date above it; the bar already has the date.
- Remove “A little clarity. Room for everything you do.”
- Ask field: **560 × 56px**, x=96, y≈180, Level 2, 16px radius.
- Placeholder: **“Ask the Mind…”**, 15px.
- Small `Super` and `K` keycaps at the right; resolve the hint for the actual platform binding.

Below the ask field, permit **one** relevant continuation, only when grounded:

> Continue Release checklist  
> Notes · edited yesterday

Use a simple two-line row, not another dashboard card. If there is nothing useful, show nothing.

Remove “Yantrik Mind is within reach” and “Open companion.”

### Wallpaper direction

Choose photography with:

- Broad quiet sky in the upper-left.
- Horizon between 52–62% of the frame.
- Restrained highlights.
- No subject sitting beneath the greeting.
- No baked-in stars, logos or artificial AI light trails.

Prepare dark and light crops offline rather than applying multiple live translucent overlays. Keep the current mountain-lake idea; choose a darker, less busy dusk/night crop for dark mode.

### Dock: retain the owner’s decision

Keep it **centred, content-hugging and flush to the bottom**.

- 48px total height.
- 32px icons in 40px slots.
- 4px gaps; 8px outer horizontal padding.
- Opaque Level 2 fill.
- 1px top/side hairline.
- 3px neutral running dot; a short blue rule for the active app.
- No magnification wave or bounce.
- Tooltips after 500ms.

Reserve its 48px band for maximised windows so it never hides bottom controls.

## C. Mind panel: not a permanent tenant

**No, it should not take width from every screen by default.**

The current panel makes every application feel subordinate to the Mind. Its content is mostly a status report about the software itself.

Replace that architecture with three explicit states:

### 1. Hidden — default

No reserved width. Top-bar mode control and `Super+K` remain available.

On a fresh install, desktop and apps start here. Existing per-place choices should be retained during migration.

### 2. Strip — user-pinned

- **40px wide**, opaque Level 1, at the right edge.
- Mind button near the top.
- At most one meaningful pending-approval count.
- Expand control at the bottom.
- No vertical text, mini dashboards or unlabelled coloured indicators.

The strip must be a choice, not the minimum tax for using Yantrik.

### 3. Expanded — user-pinned

- **304px wide**, not one-third of the screen.
- 16px padding.
- Plain structural pane when docked; no floating-card shadow.
- Opening via the global shortcut initially overlays the app. Reserving width requires an explicit **Pin** action.
- Remember Hidden / Strip / Expanded per place.

Below 1100px workspace width, use an overlay rather than reducing the app below a usable layout.

### Expanded content

Header:

> **Mind**  
> Act within limits

Then only relevant sections:

**Here**
> Release checklist  
> Current note

Or, when unknown:

> I don’t know which project this belongs to.  
> **Choose project**

**Now**
> Comparing the two drafts  
> Started just now · **Stop**

If idle, use one quiet line: **“Nothing running.”** No large empty “Working” section.

**Needs your approval**  
Show a real approval card only when necessary.

**Recent**
Show at most two useful, verified outcomes. Older actions belong in Activity, not permanent desktop furniture. A seven-hour-old render is not “Now.”

Remove:

- “Uses its own models.”
- “6 can answer · 4 remember.”
- Total memory count.
- Repeated provider dropdowns.
- “Done without asking.”
- Repetitive tool telemetry.

Make freshness honest: if context has not refreshed, say **“Last checked 8 min ago.”** Never turn last-known state into a present-tense assertion.

### Approval card: a shared safety component

Every risky action must present:

1. **The action:** “Send this email?”
2. **The target:** recipient and subject.
3. **The consequence:** what will leave the device or change.
4. **The scope:** once, for these items, not a vague blanket permission.
5. **Preview**, where applicable.
6. Explicit buttons: **Cancel** and **Send email**.

Use a warning icon and warm leading rule—not a fully amber panel. Destructive actions use a danger-labelled confirmation.

Pending approvals cannot disappear into a cleared notification feed. If the action, recipient or scope changes after review, approval must be requested again.

## D. Lens: a conversation, not a debugging console

Lens and the expanded Mind panel are **two views of the same right-side host**. Never show both side by side.

At 1280 × 800:

- Width **440px**.
- Right inset 12px.
- Top y=48.
- Bottom 12px above the dock.
- Level 2 fill, 16px radius, 20px inner padding.

Header: **Mind**, new conversation, history menu, close. Remove “Attached mind,” the extra identity icon and the permanent download control.

Conversation:

- Assistant text directly on the ground, 15px / 22px.
- User messages on a restrained Level 1 inset, 12px radius; no bright white bubbles.
- 24px between turns.
- Tool activity collapsed into a readable disclosure: **“Checked open windows”**.
- Failed actions state the limitation before offering an alternative.

For the example shown:

> **I haven’t run it.**  
> I can’t execute commands on this device. I can help you run it yourself.

Do not follow that with a long defensive paragraph or an unsupported account of which apps are open.

Composer:

- Minimum 88px high.
- Field and send button, not a nest of pills.
- Attachment control secondary.
- One compact disclosure beneath: **“Processed on this device”** or **“Sent to [provider]”**, based on actual routing.
- Keep remote-processing information visible; put model configuration in its detail popover.
- Never claim local processing when the route is unknown.

Proactive suggestions should be a single optional row above the conversation—not a permanent “FOR YOU” feed.

## E. Notifications: an inbox for things that matter

Use a **760px maximum-width list**, centred in the available content area, with 32px top padding.

Header:

> **Notifications**  
> 3 unread

Actions: **Mark read**, overflow menu. Put destructive bulk clearing in the menu.

Group by **Today / Earlier**, then by source where useful.

Each item gets:

- 20px source icon.
- 14px / 600 outcome title.
- At most two lines of useful preview at 14px / 20px.
- 12px relative timestamp.
- At most two actions.
- 16px padding and 8px separation.
- Neutral background; blue unread dot. Do not make the whole feed a stack of elevated cards.

Crucially, **fix what generates notifications**:

- Routine memory maintenance is not a notification.
- Personality interpretations are not notifications.
- A reflection is not an event.
- Notify for something requested, something needing attention or a meaningful verified completion.
- Deduplicate equivalent events.
- Batch low-priority completions.

Replace “Done” with the actual result, if verified. Do not cosmetically turn unreliable internal text into a confident summary.

Remove all “AI-generated reflection” prefixes and process-confirmation lines. User-relevant evidence belongs in an optional **Details** disclosure, expressed plainly—not as raw process output.

## F. Memory: something the person can understand and correct

Title: **Memory**. Subtitle: **“What the Mind remembers about you and your work.”**

Structure:

- 200px navigation column.
- Main list with maximum reading width of 760px.
- Categories: **All, About you, Preferences, Projects, Conversations**.
- Search at the top; no count as the hero metric.

Rows should be statements:

> **You live in Bentonville, Arkansas**  
> From your conversation on 19 Oct  
> Edit · Forget

For an inference:

> **You may prefer short morning summaries**  
> Inferred from recent conversations · **Confirm or correct**

Do not silently rewrite inference as fact.

- Remove episodic/semantic badges.
- Remove importance bars.
- Deduplicate identical facts with retained provenance.
- Present conflicting facts for review rather than silently choosing one.
- Exclude infrastructure telemetry and model self-descriptions from personal memory.
- Move genuine security incidents to a separate, understandable safety history. Do not conceal an actual incident by merely filtering away its ugly label.
- Explain the scope of **Forget**, including any limits on deletion, rather than promising more than the system can do.

Use separators and 16px vertical padding, not full-width rounded slabs. Expand a row to inspect its origin or correct it.

This screen is a trust interface, not a database viewer.

## G. System: precise, not theatrical

Keep resource monitoring, but rename the destination **System Monitor** if that is all it does.

With the Mind hidden, use a **1040px maximum-width content area**, centred, 32px top padding.

Top row:

- CPU
- Memory
- Storage

Three equal columns, 24px gaps. Each has a 14px label, 24px value and one 12px explanation. Only show values the system actually measures.

Below:

- CPU history: 160px plot height.
- Memory history: 120px plot height.
- 24px section separation.
- Charts sit directly on the ground, not inside large contrasting boxes.
- 2px neutral trace.
- 6% neutral area fill if necessary.
- Three faint gridlines.
- Explicit scale and **“Last 60 seconds.”**

A flat chart is allowed to be flat. Do not exaggerate variation to make the system feel alive. Missing samples appear as gaps, not invented continuity.

For femtovg, bounded-resolution chart images generated from samples are acceptable; do not construct hundreds of translucent rectangles per plot.

Place processes below the overview. Show swap only when configured or in use. Replace raw load thresholds with a concise status and a **Details** disclosure. Never show “Healthy” if the relevant measurements are unavailable.

## H. App windows and Email

### Shared chrome

Use Notes as the starting structure: useful sidebar, restrained toolbar, large content plane.

Standard title bar:

- **48px high**.
- 16px left/right padding.
- 24px app icon, 12px gap, 14px / 600 title.
- App actions in one row.
- Window controls: 36px-wide targets with 16px icons.
- No repeated back button on destinations that have no navigation history.
- No second decorative title band beneath the title bar.

Title bars and sidebars share Level 1. The document remains Ground. One search field per task; one primary action.

Remove idle status bars unless they carry necessary state. Show “Saving…” and failures when relevant; do not permanently spend 40px saying “Ready when you are.”

### Email at 1280px

With the Mind hidden:

- Folder sidebar: **208px**.
- Message list: **344px**.
- Reading pane: remaining width.
- At narrower widths, switch to list/detail navigation rather than compressing all three columns.

Folders:

**Inbox, Sent, Drafts, Archive, Spam, Trash**

Map provider-specific folders to their correct meaning; Gmail’s **All mail** should remain “All mail,” not be incorrectly relabelled Archive. Preserve custom folders without exposing protocol namespaces.

Other corrections:

- One search field in the toolbar.
- One **Compose** action in the sidebar.
- Remove the extra Compose in the toolbar and empty pane.
- Use `10:35`, `Yesterday`, `18 Oct`; full local date/time on inspection.
- Message rows: 80px, sender / subject / preview.
- Unread: semibold sender plus blue dot.
- Neutral count badges; stop scattering blue pills down the sidebar.
- Drop initial avatars in the dense list unless a meaningful, stable sender identity is available.
- No randomly changing colour for repeated messages from the same sender.
- Do not fetch remote avatar imagery automatically without a privacy decision.
- Reader text: 16px / 24px, maximum width 680px.

Empty reader:

> **Choose a message**  
> Your conversation will appear here.

No giant envelope badge or third invitation to compose.

### Notes

Preserve its spacious document region and clear library.

Improve, rather than redesign:

- Raise small metadata to 12px.
- Replace plain shortcut strings with shared keycaps.
- Remove the redundant idle footer.
- Put the empty-state icon in the same alignment as its text—or remove it.
- Remove Refresh if the library genuinely updates automatically.
- Apply the shared colour and chrome tokens.

---

## 4. What to remove everywhere

1. **AI self-advertising:** “uses its own models,” “AI-generated,” “attached mind.”
2. **Repeated entry points:** workspace app grid plus dock; multiple searches and Compose buttons.
3. **Unsolicited status furniture:** permanent Mind panel, stale activity and empty working sections.
4. **Internal representations:** IDs, infrastructure paths, process names, raw enums and storage categories.
5. **Card nesting:** card → section card → row card → pill.
6. **Decorative colour:** rainbow metadata, blue buttons everywhere, turquoise idle dots.
7. **Tiny typography used to rescue overcrowding.**
8. **Empty-state marketing paragraphs.**
9. **Routine success notifications and personality commentary.**
10. **Fake certainty:** present-tense claims based on old data, inferred facts presented as known facts.
11. **Decorative motion:** breathing, bouncing, scanning and continual shimmering.
12. **Default borders on every control:** use spacing and grouping before adding boxes.

Do not remove uncertainty, provenance, processing location or permission boundaries merely because they are visually inconvenient. Make them concise and intelligible.

---

## 5. Prioritised implementation plan

Ordered by visual impact per unit of work, with safety-preserving dependencies noted.

**Shared** means the change improves multiple surfaces through tokens or a reusable component. Sizes are relative implementation effort, not calendar estimates.

| # | Change | Size | Scope | Fixes |
|---|---|---|---|---|
| 1 | Remove workspace duplication, desktop slogan and companion footer; keep dock unchanged in position | Small | Single screen | Desktop |
| 2 | Replace the two Minds pills with one clearly labelled, persistent permission control | Small | **Shared shell** | Every screen |
| 3 | Make Mind hidden by default for new places; retain existing preferences and reliable access to pending approvals | Medium | **Shared layout** | Desktop, all apps and destinations |
| 4 | Introduce navy surface, text, radius and border tokens; remove translucent content cards | Medium | **Shared tokens** | Entire OS |
| 5 | Apply the Barlow scale and minimum metadata size; standardise buttons, fields and keycaps | Medium | **Shared components** | Entire OS |
| 6 | Suppress routine reflection/maintenance notifications; remove debug metadata from visible summaries | Medium | **Shared content pipeline** | Notifications, Mind activity |
| 7 | Normalise Email folders and dates; remove duplicate search, Compose and unhelpful avatars | Small | Single screen | Email |
| 8 | Unify window title bars and sidebars around the Notes structure | Medium | **Shared chrome** | All apps, System, Memory, Notifications |
| 9 | Build the shared approval card and pending-approval entry point before consolidating Mind surfaces | Medium | **Shared safety component** | Mind, Lens, Notifications, action flows |
| 10 | Rebuild Mind as Hidden / Strip / Expanded with a fixed 304px panel and honest freshness states | Large | **Shared shell + state model** | Every screen |
| 11 | Simplify Lens into readable conversation, collapsed tool activity and clear processing disclosure | Medium | Shared Mind host | Lens, contextual assistance |
| 12 | Rebuild Memory presentation and its provenance, deduplication and correction flow | Large | Single destination + data model | Memory, Mind trust |
| 13 | Replace System’s large chart boxes with measured typography and honest inline plots | Medium | Single screen | System Monitor |
| 14 | Curate wallpaper crops, unify app icons and finish dock geometry | Medium | **Shared visual assets** | Desktop, dock, app chrome |
| 15 | Add the motion specification, reduced-motion behaviour and software-GL performance pass | Medium | **Shared behaviour** | Entire OS |

### Release gate

Review every surface at **1280 × 800, dark and light, 100% and 125% scaling**, with keyboard navigation and software GL.

Ask:

- Is the person’s content the strongest thing on screen?
- Is blue telling me where my attention is?
- Can I understand what the Mind may do without opening chat?
- Can I distinguish known, inferred and unavailable information?
- Does this surface still work with the Mind completely hidden?
- Is every visible box earning its place?

**The target is not “more beautiful AI.” It is a beautiful desktop whose intelligence becomes visible exactly when it is useful.**
