# Spike: the bar and taskbar as wlr-layer-shell panels

Saga task 4, 2026-10-01. Branch `spike/layer-shell-panels`. Code: `crates/yantrik-panel-spike/`.
Screenshots and logs: `design/layer-shell-spike-2026-10-01/`.

## Verdict: GO, as a separate `yantrik-panel` process, with four conditions

The bar and a popover ran as real layer-shell surfaces, drawn by Slint's software renderer into
`wl_shm`, on one event loop, on a real compositor (labwc), and every behaviour the P0s need was
observed, not argued. The conditions are in section 7. Nothing here touched `yantrik-ui`,
`yantrik-ui-slint` or `rc.xml`.

Words used with care in this document:

- **Observed** means I compiled it, ran it, and saw it in a log or a screenshot.
- **Reasoned** means I worked it out from the code or the protocol and did not run it.
- **Not tested** means just that.

## 1. What was run, and on what

There is no desktop compositor here, but there is a way to run a real one without sudo:

- WSLg's own compositor (weston, `WAYLAND_DISPLAY=wayland-0`) **has no `zwlr_layer_shell_v1`**.
  Observed: the spike exits with code 3 and "this compositor has no zwlr_layer_shell_v1".
  That also shows the spike fails cleanly where the protocol is missing.
- labwc is not installed, and I cannot install packages. I downloaded Ubuntu's `labwc 0.7.1`
  (wlroots 0.17.1) and its libraries with `apt-get download` (no root), unpacked them under
  `~/dl`, and ran it on the **headless backend with the pixman renderer and two virtual 1280x720
  outputs**. labwc 0.7 exits at startup without `/usr/bin/Xwayland`, so the run happens inside a
  private `unshare -r -m` mount namespace with an overlay that holds a stub binary. Nothing on
  the real filesystem changed. `grim` (also unpacked, not installed) took the screenshots through
  wlr-screencopy.
- A headless compositor has no input devices, so two small test helpers drive it. They are in the
  crate and are test tools only: `probe-input` speaks `zwlr_virtual_pointer_v1` and
  `zwp_virtual_keyboard_v1`, and `probe-window` is an ordinary xdg toplevel.

What this is **not**: it is not the image's compositor. Debian 13 ships labwc 0.8.x on wlroots 0.18
(from memory; I did not check the ISO script, which only says `labwc`). The version I ran is one
release older. It is also not a GPU, not a real seat, not a VM, and not HiDPI.

The scripts that start labwc and run the scenarios were kept outside the repo (scratch). The
commands that matter are in section 8.

## 2. What was proven

| # | Claim | Evidence | Status |
|---|---|---|---|
| 1 | The crate builds against the lockfile with no new versions of any crate | `cargo build --locked --offline -p yantrik-panel-spike --bins` finishes; lock diff is one new package entry (section 6) | Observed |
| 2 | Bar: layer Top, anchored top+left+right, 32 px, exclusive zone 32, no keyboard. labwc answers `configure 1280x32` | `spike.log`, screenshot `01-bar-and-window.png` | Observed |
| 3 | Exclusive zone works: a window that asks to be maximized gets **1280x663**, which is 720 - 32 (bar) - 25 (labwc title bar). With `<margin top=32>` gone from rc.xml, this is what replaces it | `window.log` | Observed |
| 4 | One bar per output: two outputs, two bars, each configured at 1280x32 and each drawing its own clock | `01-bar-and-window.png` (both halves) | Observed |
| 5 | Pointer input reaches Slint: a click at the button's position makes `toggle-popover` fire, the popover opens, and the button text changes from "Open" to "Close" on both bars | `spike.log` ("pointer press at (1220.0, 16.0)"), `02-popover-open.png` | Observed |
| 6 | The popover (layer Overlay, 360x200) is drawn **above** a normal window. It is a separate `wl_surface` the compositor stacks, not something drawn inside the app | `02-popover-open.png` | Observed |
| 7 | The popover is above a **fullscreen** window too, while the bar (layer Top) is hidden by it. A bar that hides for a fullscreen video and a popover that does not is the correct split | `05-fullscreen-bar-hidden.png`, `06-fullscreen-popover-over.png` (popover opened by `--self-click`, which bypasses compositor input) | Observed |
| 8 | Keyboard: the popover (`KeyboardInteractivity::OnDemand`) received the keys, the app window under it received none, and typed text appeared in the Slint field. Escape closed the popover and focus went back to the app window | `spike.log`, `window.log`, `03-popover-typed.png`, `04-escape-closed.png` | Observed |
| 9 | Under an `ext-session-lock-v1` lock (the existing `yantrik-lock`), **every layer surface disappears**: both bars and any overlay | `07-session-locked.png` | Observed |
| 10 | Two Slint windows of two different components share one process, one platform and one calloop loop | the spike; unit test `two_windows_one_platform_pointer_keys_and_idle` | Observed |
| 11 | No busy loop. Settled windows paint nothing: 100 consecutive `draw_if_needed` return `None` for both windows. The loop blocks until Wayland has an event or Slint has a timer due | the same unit test; idle numbers in section 4 | Observed |
| 12 | Hover and press on the button repaint a **96x24** region of a 1280x32 bar, not the whole bar | the unit test prints it; `damage_buffer` receives only that box | Observed |
| 13 | The popover's keyboard focus and the shell's raise problem are different things: nothing in the spike raises anything. The bar and popover are in place from the start and stay there | (design consequence of 2, 6, 7) | Reasoned |

Test run, in WSL:

```
cargo test --locked --offline -p yantrik-panel-spike
  tests::which_slint_timers_keep_an_idle_bar_awake ... ok
  tests::two_windows_one_platform_pointer_keys_and_idle ... ok
  test result: ok. 2 passed; 0 failed
```

These two tests prove only the Slint half (events into the window, damage, idle). They say nothing
about Wayland, which is what the compositor runs above are for.

## 3. Findings that change the design

**3.1 A hidden window with a focused text field woke the process twice a second.** First idle run:
135 wakeups in 65 s with nothing on screen. Slint treats a new window as *active*, so the popover's
focused `TextInput` started its cursor-blink timer at creation, shown or not. The unit test
`which_slint_timers_keep_an_idle_bar_awake` showed it: `next timer Some(500ms)` for a popover that
was created and never shown, and `None` once told `WindowActiveChanged(false)`. The fix is two
lines (tell inactive windows they are inactive; the compositor's keyboard `enter` says otherwise),
and it is in the spike. After the fix, 5 wakeups in 65 s. **This matters to the real shell:** the
Lens and every text field behave the same. Every panel surface that is not keyboard-focused must be
sent `WindowActiveChanged(false)`, and a surface that holds a focused field will blink at ~2 Hz
while it has focus. That is expected, not idle, and it stops on Escape (observed: `wake: asked for None`).

**3.2 An `OnDemand` surface took keyboard focus when it was mapped, with no click on it.** In
`window.log` the app window gets `keyboard LEAVE` as the popover appears, and the popover gets
`keyboard enter`. (The click that opened the popover was on the bar, a different surface.) This is
labwc 0.7.1's behaviour; the protocol allows it. For a popover the person just opened, it is what
we want. **For an approval card that appears on its own while the person is typing, it would swallow
their keystrokes, and an Enter would land on a card they had not read.** So approval cards, vault
prompts and toasts must be `KeyboardInteractivity::None`, reachable by pointer, and the mind's
"allow" must never be keyboard-defaultable. Re-test on labwc 0.8 before relying on either behaviour.

**3.3 `request_redraw()` alone repaints nothing with `ReusedBuffer`.** Slint's software renderer
computes dirty regions from property changes, so a bare redraw request on an unchanged window
paints an empty region. My first benchmark measured exactly that (1.3 us for a frame). A full
repaint needs `RepaintBufferType::NewBuffer`, or a size change. The panel's `resize` path does this
correctly: the first frame paints the whole surface (observed: `1280x32` painted at configure).

**3.4 labwc sends the same `configure` many times.** The bar logged `configure: 1280x32` eight times
before the first paint, and again each time the popover opened. Ignoring same-size configures (the
spike's `resize` does) keeps this free. Every configure must still be acked: sctk does that.

**3.5 `zwlr_virtual_pointer` and `zwp_virtual_keyboard` are available to any client.** My test
helper, an unprivileged process with nothing special, bound both and moved the pointer and typed.
That is how labwc 0.7.1 is configured by default, and layer-shell adds a second power of the same
kind: any client of the Wayland socket may create an **Overlay** surface, which sits above fullscreen
windows and could imitate an approval card. This is not new (any window can already draw anything
inside itself), but moving approvals to the overlay layer makes the socket the trust boundary.
See condition 4.

## 4. What it costs

Everything below is the **software renderer**, as the shell uses on machines without a GPU.

**Idle** (release build, 2 outputs, bar plus a closed popover, labwc headless, WSL2 on a Ryzen 9
5950X; process counters from `/proc/self/stat`, so the resolution is one 10 ms tick):

| Run | Wakeups | Renders | CPU (user+sys) |
|---|---|---|---|
| 5 s | 3 | 2 (the first paint of each bar) | 30 ms |
| 65 s | 5 | 4 (first paint, then the clock minute on each bar) | 20 ms |
| 65 s, popover opened at 3 s (not keyboard-focused) | 9 | 7 | 30 ms |

The 20-30 ms are startup (fonts, first paint). Steady idle is below what this resolution can
see, and the loop wakes only for the clock (once a minute) and for Wayland events. Peak RSS with two
bars and the popover component: **18.6 MB** (`VmHWM`). Those are numbers for a 32 px strip; they
are not an estimate for a full-screen surface.

**Per frame**, a full repaint, with no compositor in the loop (`yantrik-panel-spike --bench`, release):

| Surface | Render | Convert to Argb8888 |
|---|---|---|
| bar 1280x32 | 37 us | 26 us |
| bar 1920x32 | 30 us | 36 us |
| bar 3840x32 | 35 us | 75 us |
| popover 360x200 | 69 us | 43 us |
| the bar component stretched to 1920x1080 | 305 us | 1,327 us |

The last row is a floor, not a forecast: it is the cheap bar component stretched over a screen.
Real screens are much busier. What it does show is that **the byte conversion costs more than
rendering at large sizes**. The renderer produces `PremultipliedRgbaColor` (R,G,B,A) and `wl_shm`
wants B,G,R,A. A small custom pixel type that implements Slint's `TargetPixel` in BGRA order would
remove the pass (reasoned; not tried). For panels of 32-40 px it does not matter.

**Partial repaints** (observed): a hover or press repaints 96x24 px; `damage_buffer` tells the
compositor only that box. The spike still copies the whole frame into the shm slot, because a slot
may be fresh memory. Keeping two slots and copying only damage is a refinement, not needed for panels.

**Memory per surface** is `width x height x 4` per buffer: 164 KB for a 1280x32 bar, 491 KB at 3840
wide, 8.3 MB for a 1080p full-screen surface (reasoned from the formula).

**Not measured:** a busy surface (an animated popover, a scrolling list), a 4K output, CPU of the
compositor itself, and anything on real hardware or in the VMs.

## 5. Architecture proposal

### 5.1 One process or two: two

Slint allows **one `Platform` per process**. The shell today runs on the winit backend (the
femtovg-or-software choice in `render_backend.rs`), and winit cannot create layer surfaces. Moving
the shell to a custom platform would make the whole 3,262-line `app.slint` window go through a
hand-written event loop, and would lose winit's text input, window management and AccessKit
bridge for every screen. A **separate process, `yantrik-panel`**, is the shape that already exists
twice in this repo: `yantrik-lock` is a Slint-on-wl_shm client with a stdin/stdout line protocol,
and the shell already starts it, restarts it and falls back if the compositor lacks the protocol
(`session_lock.rs`, exit code 3). The panel process copies that pattern and reuses this spike's
`platform.rs` (window per component) and `panel.rs` (surface, buffer, damage).

### 5.2 Surfaces, layers and components

All components below already exist as `in property` plus `callback` components with no hidden
state: `StatusBar` (`components/status_bar.slint`, 756 lines) has dozens of `in property` and its
callbacks, and `Taskbar` (328 lines) the same. They move **without a rewrite**: each is wrapped
in a thin `Window` (like `BarView` in the spike) and `app.slint` stops drawing them.

| Surface | Layer | Anchor | Size, zone | Keyboard | Slint component |
|---|---|---|---|---|---|
| Bar | Top | top, left, right | 32 px (`Theme.status-bar-height`), zone 32 | None | `StatusBar` |
| Taskbar | Top | bottom, left, right | 40 px (`Theme.taskbar-height`), zone 40 | None | `Taskbar` |
| Quick Settings, power menu, volume, network, battery popovers | Overlay | top-right under the bar (margin top 4), or bottom for taskbar menus | content size, zone 0 | OnDemand | `QuickSettings`, `PowerMenu`, new volume and network popovers |
| Toasts | Overlay | top-right (today bottom-right) | content size, zone 0 | None | `ToastBanner` |
| Approval card, vault prompt | Overlay | top-right, below the toasts | 404 px wide, content height | **None** (see 3.2) | `ApprovalCard`, `VaultUnlockCard` |
| Volume and brightness OSD | Overlay | bottom centre | content size, zone 0 | None, and an empty input region so clicks pass through (not tested) | new |
| Click-outside catcher for popovers | Overlay, under the popover | all four edges | whole output, zone -1, transparent | None | none (see 5.5) |
| Wallpaper and desktop | Background | all four edges | whole output | None | `DesktopScreen` background only |

A zone of 0 on the Overlay surfaces is deliberate: it makes them sit **below** the bar instead of over
it. Observed: with zone 0 and anchor top, the popover began under the bar (`02-popover-open.png`).

### 5.3 How the one-window shell splits

Today `app.slint` composes bar, taskbar, desktop, all hosted screens, Quick Settings, power menu,
toasts and the approval stack into one `AppWindow` (bar at `app.slint:2648`, taskbar at `:2742`,
approval stack at `:3028`, vault prompt below it). After the split:

- **`yantrik-ui` keeps one `AppWindow`**, now as an ordinary **maximized** toplevel, not fullscreen.
  Observed: a maximized toplevel gets the area under the bar (1280x663 above). It keeps the hosted
  screens, the Lens and the desktop content. The blocks at `app.slint:2648-3110` that draw the bar,
  taskbar, Quick Settings, power menu, toasts and approvals are removed, and so is the `<margin>`
  in `rc.xml`. The shell window is no longer why anything is "in front".
- **`yantrik-panel`** is new, one process, one `Window` per surface, as in this spike. It owns no
  state of its own: it draws what it is told.
- **The Lens, command palette, window switcher and clipboard panel stay in the main window for
  stage A.** They are tangled with chat state (`lens-chat-mode`, 2,407 lines in `intent_lens.slint`).
  They keep today's `raise_shell()` behaviour, which is correct for them (they are the shell's own
  UI). They can follow in stage B as Overlay surfaces with `OnDemand` keyboard.

### 5.4 State, input and the control surface

- **Data flows shell to panel as a snapshot of the properties the bar needs, over a Unix socket.** The
  repo already has `yantrik-ipc-transport` (JSON-RPC 2.0, newline-delimited, with peer credentials).
  The shell computes everything (battery, network, mind state, notifications count, open windows);
  the panel is a view. Updates are pushed on change, diffed, so an idle desktop sends nothing.
- **Events flow panel to shell**: `settings_tapped`, `power_pressed`, `anchor_clicked`, taskbar
  entry clicks, approval `allow`, `allow_session`, `deny`. These are the same callbacks `app.slint`
  wires today (for example `settings-tapped => { root.quick-settings-open = ... }`); they become
  messages the shell turns into the same actions.
- **Globals are per component instance in Slint.** `Theme`, `ThemeMode`, `AccentPreset`,
  `ThemeOverrides` and `Tr` are globals, so a second process has its own copies. The shell sets
  them from 8 places today (`ThemeMode` 2, `AccentPreset` 3, `ThemeOverrides` 1, `Tr` 2); the panel
  must receive the same values (theme mode, accent, overrides, language) in its snapshot and set
  them on each of its windows. This is a real piece of work and the most likely place for "the
  bar is a different blue" bugs.
- **Control surface parity.** Quick Settings, power and approvals are on `describe shell` and
  `act shell` today. The shell stays the owner of that surface: the panel reports "popover X is
  open", "approval Y is showing" back to the shell, so `describe` reads true values, and `act`
  commands go through the shell, which tells the panel. The panel has **no control surface of its own**,
  which keeps `control*.rs` as the one security boundary. Grades do not change.
- **Pointer** is delivered by sctk's `PointerHandler` and mapped by `wl_surface` to the window
  (observed). Press and release go to Slint as `WindowEvent`s. A callback that runs inside
  `dispatch_event` must not touch the app state directly; the spike defers through a `Cell` and
  acts after the dispatch (this avoided a re-entrancy problem).
- **Keyboard** arrives on whichever surface the compositor focuses (`enter`/`leave`). The spike
  maps `enter` to `WindowActiveChanged(true)` and forwards key text and special keys. The keymap is
  sctk's; modifiers, key repeat for held keys, and IME were not tested.
- **Focus rules to adopt:** bar and taskbar `None`; popovers `OnDemand`; toasts, approvals, vault
  prompts and OSD `None` (3.2); Escape closes a popover (observed) and focus returns to the app that
  had it (observed).
- **Approval cards** keep every rule they have today. The existing rule "not on the lock, login,
  boot or onboarding screens" is currently enforced by the shell's `current-screen`. Under the
  panel the compositor enforces it as well, because a locked session shows only the lock surfaces
  (observed, item 9). The shell must still not push approvals to the panel while locked, for the
  case of a compositor without `ext-session-lock`.

### 5.5 Popovers need a dismiss story (not tested)

Layer-shell has no `xdg_popup` grab. Waybar, eww and ags close popovers one of three ways: a
transparent full-output catcher surface under the popover, closing on `keyboard leave`, or closing
on pointer leave plus timeout. The spike has only Escape and the button. The proposal is the
catcher: one Overlay surface per output while a popover is open, zone -1, transparent, `None`
keyboard, that sends a "dismiss" message on any click. It costs one full-output `wl_shm` buffer
while open (8.3 MB at 1080p, reasoned) and must be an input-only region, which `set_input_region`
and a 1x1 buffer would allow. This needs its own small spike before stage A is signed off.

### 5.6 Multi-monitor

Observed: one bar per output, created when the output appears (`new_output`); the popover opens
on the output of the bar that was clicked. Not exercised: output hot-plug and removal (the code
retires the bar on `closed` and `output_destroyed`, but a bar's toggle callback holds its **index
at creation**, which goes stale if an earlier bar is removed; production should key by output
id), per-output scale, and the taskbar showing only that output's windows.

## 6. Cargo.lock changes

One entry added: `yantrik-panel-spike`. **No existing entry changed version.**
`cargo build --locked --offline -p yantrik-panel-spike --bins` and `cargo test --locked --offline -p yantrik-panel-spike` pass.
The new package's dependency list is: `chrono`, `slint`, `slint-build`, `smithay-client-toolkit 0.20.0`,
`wayland-client`, `wayland-protocols-misc`, `wayland-protocols-wlr`, `xkbcommon 0.8.0`,
`yantrik-design-tokens`. All were already in the lockfile. `smithay-client-toolkit 0.20.0` is the
version `yantrik-lock` already uses (the other copy, 0.19.2, is winit's). `wayland-protocols-wlr`,
`wayland-protocols-misc` and `xkbcommon` are used only by the two test helper bins.
The workspace `Cargo.toml` gained one member line.

## 7. Risks, and the conditions for GO

1. **Version: re-run on labwc 0.8 / wlroots 0.18 (the image's).** All compositor observations here
   are labwc 0.7.1. Focus-on-map (3.2), configure repeats (3.4) and the exclusive-zone arithmetic
   (title-bar height) could differ. This is a half-hour check with the same scripts. **Condition.**
2. **Accessibility.** The shell's winit window gets an AT-SPI tree from AccessKit. A custom-platform
   panel gets none unless we add one. The bar is the part of the desktop a screen reader reaches
   first. `a11y-service` and the control surface's `describe` cover agents, not Orca. Decide before
   stage A whether this is acceptable. **Condition.**
3. **HiDPI.** The spike renders at scale 1. Production needs `wl_surface.set_buffer_scale` (integer)
   or `wp_fractional_scale_v1`, plus `ScaleFactorChanged` to Slint, and buffers sized in physical
   pixels. `yantrik-lock` has the same gap today. **Condition.**
4. **The Wayland socket becomes the trust boundary for approval cards.** Any client can create an
   Overlay surface and, on labwc 0.7.1, bind virtual input (3.5). Mind accounts must not be able to
   reach the desktop's socket, or the compositor must filter those globals (I did not check what labwc offers for that). A fake approval card is a security story, so
   this goes through a security review. **Condition.**
5. **Crash and restart.** If `yantrik-panel` dies the person has no bar and no taskbar. The shell
   must restart it (as it does for `yantrik-lock`) and, for a compositor with no layer-shell (exit
   3), keep drawing the bar in-window as today. Not tested.
6. **Startup order.** The panel must be up before windows are placed, or the first windows are
   placed under the future zone. labwc re-arranges maximized windows when a zone appears (observed
   indirectly: the window was already maximized to 663 px high); normal windows were not tested.
7. **Frame pacing.** The spike paints as soon as Slint says it is dirty. Production should paint on
   `wl_surface.frame` callbacks, so an animated popover cannot outrun the compositor. The animation
   loop (16 ms while `has_active_animations`) is in the spike but no animation ran.
8. **Click-through and input regions** (OSD, toasts) were not tested. `set_input_region` exists in
   sctk; I did not run it.
9. **Popover size.** The spike uses a fixed size. Content-sized popovers need a
   `set_size` plus configure round trip each time the content changes, and a one-frame flash is
   likely unless the surface is made large and clipped. Not tested.
10. **Fonts.** The panel renders Barlow through Slint's fontdb in the same way the shell does; the
    screenshots show Barlow. The on-image font path was not tested.

## 8. How to reproduce

All commands run in WSL Ubuntu. Run inside Git Bash with `MSYS_NO_PATHCONV=1`.

```sh
cargo build --locked --offline -p yantrik-panel-spike --bins
cargo test  --locked --offline -p yantrik-panel-spike
cargo run --offline --release -p yantrik-panel-spike --bin yantrik-panel-spike -- --bench
# with a compositor that has layer-shell and WAYLAND_DISPLAY set:
yantrik-panel-spike --trace --exit-after 20       # prints a counters report at the end
probe-window --secs 15                            # a maximized toplevel: shows the zone
probe-window --fullscreen --secs 15
probe-input move:1220:16:2560:720 click           # click the button, 2 outputs side by side
probe-input type:hi key:1                         # type, then Escape (keycode 1)
```

The spike's flags: `--trace`, `--exit-after SECS`, `--self-click SECS` (synthesises one click;
this bypasses the compositor and is labelled so in the source), `--bench`.

## 9. Step list for the adoption story (saga task 21)

Estimates are my guesses from the spike, not measurements, in engineering days.

Stage A: the three P0 fixes (bar and popovers above apps, no raise-on-click, no margin hack).

1. Re-run this spike's scenarios on labwc 0.8 (the image's) and decide conditions 1-4. (0.5 d)
2. `crates/yantrik-panel`: promote `platform.rs` and `panel.rs`; one `Window` per surface; HiDPI,
   frame callbacks, per-output keying; restart and "no layer-shell" fallback exit code. (3 d)
3. Snapshot and event protocol over `yantrik-ipc-transport`; theme globals pushed to the panel. (2 d)
4. Wrap `StatusBar` and `Taskbar` as panel windows; remove them and the Quick Settings, power menu
   and toasts blocks from `app.slint`; shell window to maximized; remove `<margin>` from
   `rc.xml`; window list from `toplevel_watch.rs`. (3 d)
5. Popover surfaces with the dismiss catcher (5.5 first as a small spike). (2 d)
6. Control surface: panel reports popover state, `describe shell` reads it; tests in the style of
   `control.rs`. (1.5 d)
7. Preview harness scenes for the panel windows in `tests/ui-preview` (the bar and popovers draw
   through the same `MinimalSoftwareWindow`), plus a `verify-idle` for the panel. (1 d)

Stage A is about 13 days of work, with 4 to 5 of them on the critical path, so it is not a 24-hour
job as a whole; steps 1 and 2 plus a bar-only cut of 3 and 4 are the part that fits in about 24 hours.

Stage B: approval cards and vault prompts as overlay surfaces (with the keyboard and security
review above), OSD, then the Lens and the clipboard panel. (about 6 d)

## 10. Files

Added: `crates/yantrik-panel-spike/` (`Cargo.toml`, `build.rs`, `ui/panels.slint`, `src/main.rs`,
`src/panel.rs`, `src/platform.rs`, `src/tests.rs`, `src/bin/probe-window.rs`,
`src/bin/probe-input.rs`), this document, and `design/layer-shell-spike-2026-10-01/` (screenshots
and logs). Changed: workspace `Cargo.toml` (one member), `Cargo.lock` (one entry).
Not touched: `yantrik-ui`, `yantrik-ui-slint`, `config/labwc/rc.xml`.
