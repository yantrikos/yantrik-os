# The minds' surfaces: chat, agents, an overloaded dock, Alt+Tab (2026-10-02)

These are the specs for four surfaces: the chat and the Agents workroom (the parts of the OS that are its signature), the dock when it is full, and the window switcher. GPT‑6 Astra wrote them at Pranab's request. The renders are on the "Yantrik Shell 10×" canvas (https://claude.ai/artifact/FCLLZf6pgNfegzjsAY8TAU), on the boards Chat, Agents, Dock overloaded and Alt+Tab. The visual direction Pranab chose is the GPT render: a photographic wallpaper, solid charcoal, and a slim floating dock. See `ui-review-gpt6-astra-2026-10-02.md` for the rules underneath.

## Core idea
Chat and Agents are two views of the same work:
- **Chat is the relationship:** what you asked, what the mind understood, what it needs, and what it produced.
- **Agents is the workroom:** every mind's current desk, the decisions waiting, and the changes recorded.
- **Mind View is the actual desk.** Take over transfers control explicitly.

The object that connects them is the **work card**: a run's mind, state, desk preview and evidence. It appears compact in Chat and full size in Agents.

## Shared contract
- **Tokens:**
  - text #F2F4F5, secondary #A8B0B6;
  - panels #151A1E, bars and insets #101417;
  - borders 1px #343B41;
  - mind teal #69C8BC, needs-you amber #E7B567, folders #9DBDE5;
  - 16px radii, a 2px white focus ring.
- **Type:** Barlow. Body 15/22, reading text 16/24, labels 13/18, nothing essential below 12px.
- **Colour roles:** teal identifies minds and their presence. It is not a selection or success colour. Amber means a person's response is pending, nothing else. Selections use a neutral raised fill plus a white keyline.
- **One state vocabulary, everywhere:** Queued · Working · Needs you · Paused · Finished · Stopped · Couldn't finish · Connection lost.
  - "Finished · 3 file changes recorded" is allowed; "Everything is fixed" is not.
  - Unobservable work reads "Last update 10:42".
- **The top-bar chip** counts the minds with unresolved requests. Its tooltip gives the scope: "1 mind has 3 requests". It opens Agents, filtered to Needs you when that is non-empty.
- **The dock's Yantrik Mind button** opens Chat. Its amber dot covers requests from all minds.

## 1. Chat
**Layout**
- A right panel, 440px wide by default, resizable from 380 to 560px.
- 12px from the edges; it runs from 48px below the top to 12px above the dock.
- On narrow screens it becomes full width.

**Header (64px)**
- A 28px teal mark, the mind's name in 16px semibold, and its identity ("Local mind").
- The name opens a mind picker, whose rows show a real state ("Pi · Working"). Switching never silently re-targets an existing draft.
- Icon buttons: New chat, History (a 320px popover with search, date groups and unresolved markers), and Open in Agents.

**Messages**
- **User messages:** right-aligned white bubbles with dark text, at most 85% of the column wide, 12/14 padding.
- **Mind replies:** left-aligned with no bubble, a 6px teal dot in a 16px gutter, the name at the start of each group, 16/24 text.
- **Write results, not tool output.** "Created **Trip notes** in Documents." The evidence sits separately:
  - the path;
  - **Show in Files** · **View receipt**.
- Summaries are generated from recorded facts only. "Is present" is not the same as "created".

**The work card**
- One per reply that starts work, updated in place, 120–184px tall.
- Contents: a title (two lines at most), the mind and state, one observed activity line, an optional 96×60 desk preview, one primary action plus an overflow.
- The activity line comes from a real event ("Running a command"), never an invented narrative.
- When finished: "Finished · 12 moves recorded", with **Review changes** and **View desk**.
- **"Open the run →" goes from under every reply.** Casual conversation has no card.

**Request cards are first-class**
- **Approval:** an inset card with a 1px amber border.
  - It shows "Approval needed", the exact action, its scope and the facts about undoing it.
  - Buttons: **Approve once** / **Decline**, plus **Action details**.
  - The approval binds to the action as displayed, and a changed plan invalidates it. There is no approve-all and no Enter-to-approve.
  - Once resolved it collapses to "Approved by you · 10:42 — View action". Approved is not the same as done.
- **Question:** "Your answer needed", the named options, and **Write another answer**.
- **Handoff:** a factual event, "Handed to Hermes · Receives: this request and 2 attached files". It is an approval first if it needs consent.

**Below the conversation**
- **Current-work strip (48px):** "● Yantrik Mind · Working · View desk · Pause". With several runs it reads "3 runs in this chat · 1 needs you · View work".
- **Composer:**
  - The placeholder reads "Message Yantrik Mind…", with an attach button.
  - A mode chip "Ask ▾": Plan = "Inspect and propose. No changes allowed."; Ask = "Request approval for actions that require it."; Auto = "Act within granted permissions." The chip is shown only where the backend enforces the mode. A mode change applies to the next run.
  - Enter sends and Shift+Enter breaks the line. While a run is going, Send becomes "Send follow-up".
  - Streaming has no blinking ornament. The view auto-follows only when already at the bottom; otherwise "New reply ↓" appears.
- **Empty state:** "Your desk stays yours. Yantrik Mind can work in a separate desk. You can watch, review changes, or take over." Three starters: Plan something, Work with files, Explain what's on screen.

**Left out:** model selectors, token totals, narration of tool calls, prompt carousels, congratulations, duplicated approval banners.

## 2. Agents: a workroom
**Window and header**
- A 1200×800 reference window: a 64px header, a 248px left navigation, and the main area with 24px padding. No permanent DETAILS column.
- The header reads "Agents" (24px) with real counts, "2 working · 1 needs you", plus **Start work** and **History**.
- Pop out is removed; window controls do that job.
- **Start work** opens a 480px sheet: "What should get done?", a mind, the mode and permissions summary, an optional recipe, and **Start**.

**Left navigation**
- Workroom · Needs you (count) · History.
- **MINDS:** 64px rows, each with a teal mark, the name and a state ("2 working", "Idle", "Unavailable"), plus an amber count when requests are pending.
- Task agents group under their mind. Completed ephemeral ones move to History.
- Above 8 minds, a "Find a mind" field appears.

**Workroom overview**
1. **The decision shelf**, shown only when requests exist: "Needs you · 3 requests from 2 minds". Each card shows the mind, the task, the request and its age, with a specific action ("Review email", "Review command", "Choose folder"). At most three are expanded, then "View all 8 requests", oldest first.
2. **Working now**, a grid of desk cards: minimum 320px wide, 16px gaps, two columns at the reference size.
   - Each card is about 320×288: a 44px header (mind, task, state); a **16:9 preview of the real Mind View desk**; an observed activity line; and the actions View desk, Chat and ⋯ (Pause, Stop…, View activity, View changes).
   - A stale preview reads "Snapshot · 10:42". A missing one reads "Desk preview unavailable".
   - With no apps open, the card is compact: "No apps opened yet · Last event…".
   - Previews show for at most 12 desks, then compact rows with filters. Only visible previews refresh. The order stays stable while the person interacts.
3. **Empty state:** "No minds are working right now. Start a task, or return to a recent result." Show up to three recent results, then "View history · 140 runs".

**Task detail**
- The header has ← Workroom, the title (24px), the mind, the state and the effective mode, plus Chat, Pause/Resume and Stop….
- **Desk:** a preview up to 720×405 with **Enter Mind View**.
  - **Take over** first asks the mind to pause, then confirms: "You're controlling Hermes's desk. The run is paused."
  - It never claims exclusive control if pausing failed.
- **Changes:** a ledger of Files, Apps and External actions (each with its receipt). It distinguishes proposed, attempted and recorded. "No changes recorded", never "Nothing changed".
- **Activity:** a readable timeline. Low-level tool calls are grouped and expandable, and raw arguments stay inspectable.
- **Run details** (a disclosure): model, ids, turns, calls, tokens and cost. Cost is marked "Estimated"; anything missing reads "Not recorded".

**Left out:** the List/Overview toggle, Tasks as a separate inbox, always-visible billing, scores, avatars, fake progress, and simulated thinking.

## 3. The dock, overloaded
**Base**
- Centred, 48px tall, 12px off the bottom, 16px radius, #101417 with a neutral border, 4px padding.
- 40×40 buttons with 32px app tiles (20px glyphs on whole pixels) and 4px gaps. Changed from "24px icons" on 3 Oct 2026: a 24px tile left a 13px glyph, which Pranab saw on VM 520 as "not crisp but blunt".
- Maximum width: min(880px, viewport − 32px).

**Order**
- Apps | pinned and running apps | divider | Yantrik Mind.
- Apps and the mind are fixed anchors; only the middle overflows.

**States**
- running: a static 3px white marker;
- focused: a neutral raised fill;
- several windows: a small neutral numeral;
- a mind request: a 6px amber dot on the mind icon.
- No bounce, no magnification, no teal for ordinary apps.

**The apps a mind opens** stay off the person's dock. In Mind View, the dock shows that desk, labelled as such.

**Overflow**
- Never shrink icons or hit targets.
- When the apps don't fit, 32px page buttons appear: "› +6". Paging is instant, and the scroll wheel works over the dock.
- The order is stable: pinned apps first, then running apps in launch order.
- Alt+Tab reaching an app on another page reveals that page.
- The **Apps** launcher has a searchable **Running** section.

**Window list**
- Appears after 250ms of hover or on keyboard focus. It is 320px wide and at most 400px tall.
- Header: the app name and the window count.
- 56px rows: an optional 64×40 preview, a title of up to two lines, the workspace, and a current-window mark.
- Above 6 windows it scrolls; above 12 a "Find a window" field appears.
- A click activates the window. Clicking an app with several windows opens the list. Close actions are left out.

## 4. Alt+Tab, Omarchy-style
**The card**
- A centred opaque card, min(1040px, viewport − 64px) wide, with no dimming layer, 16px padding and 12px gaps.
- Cells are about 224×166, with a preview of about 208×117 at the source aspect ratio.
- The footer shows an 18px app icon, the app name, the class if it differs, and the workspace.
- **Selection:** a 2px white outline plus a raised neutral fill; no teal.
- The window's title sits on a separate opaque plate below the card, 40px tall and up to 720px wide: "Budget 2027 — LibreOffice Calc · Workspace 3".
- A window that can't be captured shows "Preview unavailable".

**Order and scope**
- Most recently used across all of the person's workspaces. The order is frozen when the switcher opens, and the first Tab selects the previous window.
- From the person's desktop, an open Mind View is one window.
- Inside Mind View, Alt+Tab switches that desk's windows, with the scope labelled ("Hermes's desk") and a visible way back to the personal desktop. Switching windows is not Take over.

**Keys**
- Alt+Tab steps forward and Alt+Shift+Tab back; the arrows move spatially.
- Releasing Alt or pressing Return switches; Escape cancels and restores focus.
- Mouse hover selects after an intentional move, and a click activates.
- When the switcher opens, the pointer's existing position must not override the keyboard selection.
- Windows are never activated just to grab a preview. A window that closes is removed; new windows wait for the next invocation.

**Many windows and small screens**
- 8 cells to a page, with the footer "9–16 of 37 windows · Page 2 of 5". Tab crosses pages, and there are page arrows and wheel paging.
- Below 760px wide it uses 3 columns, and below 560px it uses 2. Thumbnails never shrink to stamps.
- No search, launcher or history: those live in Apps › Running.

## The first-time moment
1. "Organise my Downloads folder."
2. The reply says what the mind understood, and a small desk appears.
3. It asks about a specific set of moves.
4. You approve, open Agents, and watch the same desk.
5. The result is "12 moves recorded", with a receipt.
6. Your own desktop never moved.
