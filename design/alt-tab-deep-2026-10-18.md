# Alt+Tab: the deep, with minds in it

18 October 2026. Mockup: https://claude.ai/artifact/7RZy2waNnVRS6rit9gnLQ9. Pranab asked for this after seeing Omarchy's Alt+Tab ([Fathom](https://github.com/mtolhuys/fathom)) and approved the direction the same day. GPT-6 Astra red-teamed the first draft; its review changed four things, listed at the end.

> **The mockup's first two boards predate the review.** They put the guess and the waiting approval *first in the stack*. That's now out: the first card is always the previous window, and the guess and approvals sit in their own lane (see "Order never surprises you").

## What a person gets

Hold Alt and tap Tab. Your windows come up as a **stack in time**:

- **The stack.** The previous window is a large card in front. Each window behind it is smaller, steps up and to the right, and fades with age.
- **The time line.** A thin line on the right reads "how long ago" from *now* to *2 h*, with a dot for every window.
- **Moving through it.**
  - Tab goes deeper, Shift+Tab comes back up.
  - Typing filters by app, title or mind.
  - Releasing Alt goes there. Escape stays put.
- **A quick tap** switches straight back, exactly as today.

Two things no other switcher has, each in **its own lane** beside the stack, never in its order:

1. **Minds at work.** Mind View is a card with a live pulse and a status line, e.g. "Hermes · step 4 of 8 · writing main.js".
   - A pending approval appears there too, marked amber: "Waiting on you".
   - Going to an approval opens a **details view with nothing to press** (see "Approvals").
2. **Likely next.** One small chip names the window Yantrik thinks you want, with its reason, e.g. "Likely next: Text Editor. You copied 2 lines from it and haven't pasted them."
   - One key jumps to it: ↑, or its number.
   - It never moves the selection by itself.

## Order never surprises you

Alt+Tab is muscle memory, so these rules are fixed:

- **The first Alt+Tab always selects the previous window.** A quick tap and a held Alt start from the same selection, so how fast the screen draws never changes where the same gesture goes.
- **The current desktop comes first** (Pranab, 18 Oct). The stack is this desktop's windows by recency, then a divider reading "Other desktops", then their windows by recency. "The previous window" means the previous window on this desktop. A desktop with only one window goes back to the last window on any desktop, so Alt+Tab never does nothing.
  - Nothing is hidden: every window on every desktop stays reachable from the keyboard, as today.
  - Going to a window on another desktop switches to that desktop.
- **The order is frozen while the switcher is open.** A window that opens, a mind that starts or an approval that arrives mid-session doesn't reshuffle the stack.
- **Nothing is ever inserted ahead of the current selection.** Minds and approvals have their lane. They never jump the queue: a mind could otherwise raise "waiting" events to keep itself first.

## Who owns the keys: the compositor

`control_switcher.rs` records why Alt+Tab is labwc's own today:

- the shell once bound Alt+Tab to its card, and release-to-switch did nothing;
- a busy or missing shell meant no Alt+Tab at all.

Astra's review sharpened the reason. A client that maps mid-chord:

- never sees the Tab that opened it;
- can't learn of an Alt that came up before it mapped;
- can't safely race a fallback helper: one would switch at 151 ms and the other again on release.

So **labwc owns the switch session**, from the first Alt+Tab to the release or Escape:

- the chord, the order, the selection and the one commit (activating the window) stay in the compositor, as today;
- our switcher is **a renderer only**. labwc tells it "session S: these windows, in this order, this one selected", then each move, then "committed" or "cancelled";
- the renderer draws, and can ask labwc to move the selection (a click, a typed filter). It never activates anything itself.

Consequences:

- **If the renderer is slow, missing or hung,** labwc keeps the session and its own OSD, and switching works exactly as today. labwc drops a late renderer's session, and a late surface never gets keyboard focus.
- **Alt released before the renderer draws** is simply a commit in labwc, the quick tap. No pill is needed.
- **The lock screen cancels the session inside labwc,** pending activation included.

This is a **labwc patch**, carried in our labwc build, and the first real use of the compositor bridge in #849. It's small in labwc terms: the window switcher already owns this session for its OSD. The patch adds an external renderer for it on a private protocol, offered only to the switcher's process.

### Step 0: read and trace before building

1. Read labwc 0.8.3's keyboard handler and window-switcher code. Record exactly:
   - what it does with a bound key's release;
   - whether `onRelease` exists on this tag and what it releases on;
   - how modifier state reaches a surface that gains focus mid-chord.
2. Trace it on 520 with `WAYLAND_DEBUG` on a test client.
3. Write the result on #849 with the patch's shape and size.

No UI work starts before this. If the patch turns out large, that is a decision for Pranab, not something to work around.

## Built from what exists

| Piece | Reused from |
|---|---|
| Window model: order, labels such as "Terminal (2)", cells | `alt_tab.rs` (`order`, `labelled`, `Cell`) |
| App tiles and names | `window_name` (the dock's), `AppTile` |
| Minds and approvals | `describe shell`'s `mind_view` and `pending_approvals` |
| Title cleaning (control and bidi characters, length) | the shared sanitiser |

The Super+Tab overview stays as it is, a grid that is also on the control surface. Both read the one `alt_tab.rs` model.

## Pictures of windows

On wlroots 0.18 (checked on 520) labwc offers no per-window capture. Astra is right that cropping the screen at focus-out isn't a window picture: it can catch the window that rose over it, a notification or a password prompt. So:

1. **Tiles for everything, first.** These are the app's tile, name and title, as the overview draws today.
2. **Our own apps draw their own card** (Slint renders its window to an image). This is one control action, `snapshot`. It is answered only on a channel the switcher gets at supervised launch, not to anyone who can name the switcher's executable. The image is kept for the session only and cleared on lock.
3. **Third-party windows stay tiles** until labwc, not just wlroots, offers per-window capture with a permission policy (#849).

**What "never to a mind" can honestly mean.** The Yantrik Mind runs under its own account (`yantrik-mind`, uid 999). The person's runtime directory `/run/user/1000` is mode 700, so it can't reach the person's Wayland socket (checked on 520, 18 Oct). For it the boundary is real. A harness that runs under the person's own account can reach the Wayland socket and anything else the person can. For those minds, the switcher's rules are behaviour we keep, **not a security boundary**. The design says so, and the general fix belongs to the agent isolation work, not to Alt+Tab.

## Approvals

The rule "no key in the switcher approves" is necessary but not enough. A queued Enter, key repeat or a pointer release can land on the approval right after focus moves. So:

- Going to an approval opens it in a **details state**. Its Allow and Not now buttons are disabled until the keys and pointer have been released and a short settle time has passed. Then it needs a fresh, deliberate press.
- Consent stays bound to the request as shown: requester, command, scope and expiry. That's the approval card's existing contract; this design adds the settle rule to it.
- Alt+Tab is never globally disabled while an approval is open. That would be a focus trap a mind could trigger. The card is modal only for its own window.

## The guess, honestly

- **Inputs** are facts the OS already has: the copy transaction (Steers, #847), the last clip receipt and contract steps that name a window or file.
- **Rules,** not a model. Each is tested as "given these facts, this chip, with this sentence". No rule fires, no chip.
- **A verified event isn't verified intent.** That's why the guess is a chip and not the order.
- **Reason lines must not leak.** The chip says "you copied from it", never what was copied. It isn't drawn while a private window is in front.
- **Measuring.** Counts only:
  - chip taken;
  - chip ignored;
  - switch-back within 5 s after a commit;
  - cancels;
  - extra Tabs per session.
  
  "First card taken" is not a metric: release always takes it.

## Security notes, written first

- **Activation.** Only labwc commits activation. The renderer can't activate, focus or raise anything.
- **Identity.** Windows are identified by labwc's own view identity. `app_id` and the title are client-asserted, used for display only, and cleaned. The shell and Mind View are recognised by the view labwc launched or attached as them, not by title. That closes #848's review finding 2.
- **Who drives the switcher.** `yos act shell` actions that move the selection carry the caller's identity. A mind's call can't impersonate a person's navigation: the commit is still a person's Alt release.
- **Crashes.** If the renderer dies, labwc ends its session and takes back the keyboard. Exclusive focus never outlives the renderer.
- **Filter text and the guess** stay in the renderer's memory and are never logged.
- **Tests:**
  - both Alt keys;
  - releasing in reverse order;
  - key repeat;
  - Shift+Tab;
  - Alt released before the renderer maps;
  - locking mid-session;
  - the renderer killed mid-session;
  - typing while Alt is held, with a non-US layout.

## Acceptance check (for the build PRs)

1. On 520 with 4 apps open, hold Alt and tap Tab: the stack draws within 150 ms. Releasing Alt commits the selected window in labwc, and the focused toplevel after release is that window.
2. With the renderer killed (`pkill -x`), Alt+Tab works exactly as today, labwc OSD included.
3. A quick tap goes to the previous window on the current desktop, every time, at any renderer speed.
4. With windows on two desktops, the current desktop's windows come first, then the "Other desktops" divider. Going to one past it switches desktop.
5. A window opening mid-session doesn't reorder the stack.
6. An approval pending: it shows in the minds lane, never ahead of the selection. Going to it opens the details state with disabled buttons, and a held Enter doesn't approve.
7. A window retitled "Yantrik OS" stays in the stack.
8. Typing "tum" leaves only matching windows. Escape restores the frozen order.
9. The Likely next chip appears only when a rule fired, and its reason names the event, never the content.

## Order of work

1. **Step 0:** read and trace, report on #849, with the patch's size.
2. **The labwc patch:** a session with an external renderer on a private protocol, plus its tests.
3. **The renderer:** the stack, the time line, tiles, filter, frozen order. Today's OSD remains the fallback.
4. **The minds lane** and the approval details state with its settle rule.
5. **Likely next:** rules, chip and counts.
6. **Our apps' own cards,** then third-party capture when labwc offers it.

Each step merges on its own and is tested on 520.

## What the review changed (GPT-6 Astra, 18 Oct)

Astra's full answer: `design/alt-tab-deep-astra-review-2026-10-18.md`.

1. **The compositor owns the session.** The first draft had a client detect Alt release, with a 150 ms helper as fallback. That races (switch at 151 ms, switch again on release) and can't recover an Alt released before mapping.
2. **The first card is always the previous window.** In the draft the guess and approvals led the order, which breaks muscle memory, and a mind could game "waiting".
3. **Approvals open a settled details state.** Navigation can become consent through a queued Enter.
4. **No screen-crop thumbnails, and honest boundaries.** A crop at focus-out isn't a window picture, and "never to a mind" isn't a boundary for a mind under the person's account.

**Kept against its advice:** the time line, which only displays what labwc already knows, and the depth stack itself, which is the reason for doing this. Its other suggested cuts (prediction-first, approval-first, crop thumbnails, the pill) are taken.

## Decided

- **Current desktop first,** then the others behind a divider (Pranab, 18 Oct).

## Open questions for Pranab

- The time line in real minutes, or only *now / earlier / long ago*?
- The Likely next key: ↑, or a number?
