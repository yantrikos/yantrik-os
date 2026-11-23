# Y Anywhere: drag the Y onto any window, or press Super+Y

Status: phase 1 (the compositor) built and tested, 23 Nov 2026; phases 2 to 5 to come. Prototype: https://claude.ai/artifact/DLFdtyX3iM2is17vhCooUM

## The experience

**Two ways in, one result.**

- **Drag.** The gold Y sits at the right end of the dock, breathing slowly. Press and drag it: it follows the
  pointer. The window under it lights up with a soft-blue halo and a one-line label: *Ask about Terminal*, and
  a quip that fits what is in it (*"Ooh, the borrow checker. Grab popcorn."*). Release: the Y flies to the
  window's top-right corner, lands with a small gold ripple, and the Lens opens **anchored to that window**.
  Drop it on the wallpaper and it returns to the dock: *"That's the wallpaper. Lovely, but it has no
  questions."*
- **Shortcut.** Super+Y does the same for the focused window: the Y flies from the dock to it.

**What opens.** The Lens, as a bubble anchored to the window instead of the bar:

1. one line that knows what it is looking at (*"The borrow checker strikes again."*);
2. the scope, always visible: *Seeing only Terminal · nothing else on your screen*;
3. **Last time here**, from memory: what the person did or asked in this app or document before
   (*"you hit E0502 in drag.rs on Tuesday; collecting the hit first fixed it"*). Shown only when there is
   one; never invented;
4. three suggestions shaped by the window's kind (error → explain / fix / why again; long read →
   summarise / what it means for me / save to Notes; numbers → where it goes / anything odd / chart);
5. the composer, placeholder fitted to the window (*Ask about this error…*).

The answer streams into the bubble. Esc or the close button puts the Y back on the dock. Opening the full
Lens from the bubble keeps the conversation (it is one Lens turn with a window attached, not a second chat).

**Later, the other direction:** drop text, a file or an image *onto* the Y to ask about that.

### Voice

Witty in the first line only, never in the answer, and never at the person's expense: the quip is about the
content (*"A 14-minute read. I can do it in 14 seconds."*), not the user. One vocabulary with the rest of the
OS (design/experience-final-2026-10-10.md): it is the Lens, the minds answer, nothing new to learn.

### Colour and motion

Gold is the Y's and only the Y's (brand mark). The halo and the chips use the system accent; amber stays the
"in flight" colour and is not used for the halo. One decisive motion per step: the drag follows the pointer
with no lag, the landing is one overshoot arc (420 ms), the bubble rises 8 px (220 ms). Nothing animates that
is not happening; the dock Y's breathing stops while a mind is answering elsewhere.

## Why this needs the compositor

labwc tells no client where a window is (design/capture-2026-10-15.md), so today the shell can neither say
which window is under a point nor take a picture of one window. Both gestures need both. We already carry a
labwc patch with exactly the right pattern: a privileged protocol offered only to a helper labwc launches
itself, with per-window capture by scene-subtree render (packaging/labwc, Alt+Tab, #849/#867).

### Protocol: `zyantrik_window_locator_v1` (added to the same patch)

Offered, like the switcher renderer, only to the client labwc launched from `<yantrik locator="command">`.

| Request | Answer |
|---|---|
| `at(x, y)` (layout coordinates) | `window(id, app_id, title, x, y, w, h)` for the topmost mapped toplevel there, or `none` |
| `focused()` | the same for the focused toplevel, or `none` |
| `capture(id, max_w, max_h)` | `picture(id, fd, w, h, stride)`: that window's own scene subtree rendered offscreen (the switcher's `capture` code, moved to a shared helper), never what overlaps it |

Rules carried over from the switcher protocol: one client, rate-limited captures, nothing while the
session is locked, ids stable while the window lives. The helper never learns about a window it did not ask
about, and it can only ask while the person is holding the Y or has just pressed Super+Y (the helper enforces
the gesture; labwc enforces the client).

### Pieces

| Piece | Where | Size |
|---|---|---|
| Locator protocol + labwc patch | `packaging/labwc` (patch rev 3, `build-deb.sh REVISION`) | M |
| `yantrik-y`: layer-shell overlay, Slint software renderer (the `yantrik-switcher` / `yantrik-lock` pattern). Idle: the dock seed. Dragging: a full-screen transparent top-layer surface that owns the pointer, calls `at()` at ≤ 60 Hz, draws halo + label | `crates/yantrik-y` | M |
| Super+Y | `config/labwc/rc.xml`: `W-y` → `yos act y look` (the helper also answers the control surface) | S |
| Window context into the Lens | `yantrik-ui`: `open_lens` gains `about={window id, app, title, picture?, text?}`; the bubble placement; the scope line | M |
| What the mind is given | ours: `app.describe` / a11y text (#1257, "semantic for ours, visual for theirs"); theirs: the captured picture to a vision-capable route, else the title and an honest "I can see the title, not the contents" | M |
| Last time here | YantrikDB recall scoped to `app_id` + document title, written as one memory per asked turn (`about: app/title`, the question, the answer's first line) | S |
| Quips and suggestions | a small table by window kind (terminal error, long text, numbers, code, mail, image, unknown), chosen from the a11y text / title; unknown kind gets generic suggestions and no quip | S |

### Privacy

- The drop *is* the consent, for that window, for that one ask. Nothing is captured until release.
- The bubble always names what is seen. Capturing a window counts like a screenshot for the capture notice
  (capture/notice.rs).
- A window whose app is marked private (password manager, vault) is refused at `at()` time: the halo turns
  grey and says *"That one's private, even from me."*
- Pictures stay in memory for the turn; only the text of the exchange is remembered, never the picture.

## Constraints agreed with the Yantrik OS session (22 Nov)

- **Freeze first.** Pranab paused new initiatives at ~02:00 UTC on 22 Nov until the core pipeline passes;
  this starts only when he picks it over that. Never dispatch ci.yml to our runner for this work until the
  train (#1336, #1357, #1349, #1355, #1343, #1306, #1359, #1360) has landed; hosted PR runs are fine.
- **Build on, not beside:**
  - `open_lens about=` builds on #1349's reply path (Lens markdown), after it merges;
  - capture audit lines go through #1357's audit store (`mind-audit.jsonl`);
  - `W-y` in `config/labwc/rc.xml` is rebased after #1355.
- **Patch layout.** The locator is its own patch file in `packaging/labwc`, built into the same package. The
  shared scene-capture helper lands in the first patch, and the switcher is re-pointed at it, so each
  protocol can be reviewed or dropped alone.
- **Opus security review** (capture is a screen-capture capability), expected to cover:
  - the launched client is the only caller;
  - capture happens only on the person's gesture, never at a mind's request;
  - private mode refuses capture;
  - a sentence says where a foreign window's picture goes before a cloud model sees it;
  - every capture is audited.
  The PR text names classes of risk, not recipes (public repository).
- **VM 542** for compositor work. 520 is Pranab's live session and 541 is the train's test machine.

## Phases

1. **Compositor**: locator protocol in the labwc patch, probe client, tests on a VM (as the switcher was).
2. **Super+Y first**: `focused()` + `capture()` → Lens bubble with the window attached. Ships value with no
   drag UI.
3. **The drag**: `yantrik-y` seed and overlay, halo, landing.
4. **Memory and voice**: Last time here, quips and suggestions by kind.
5. **Drop onto Y**: text, files, images.

## Open questions for Pranab

- Seed placement: the dock's right end (prototype) or a corner of the screen.
- Super+Y vs another chord (Super+K is the Lens; Super+Y is free in rc.xml).
- Whether the quips ship on by default (a Settings toggle, "Playful first lines").
