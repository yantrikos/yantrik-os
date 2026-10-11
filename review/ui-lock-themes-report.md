# Lock screen and themes: report

Branch `ui/lock-and-themes`, from `origin/main` at `c5c530f` (#590). Plan PRs 6.1 and 6.2 in
`design/ui-overhaul-2026-10-01.md`, with the lake default wallpaper.

Security review: the lock screen was re-laid out; authentication unchanged. What is new on that
path is listed under "What the lock client can now do" so a reviewer does not have to find it.

## What a person sees

- **A new install and a reset start on the lake.** The desktop shows the 2560×1600 lake photograph
  (covered, not stretched). Settings > Appearance > Wallpaper lists Lake first, with its preview.
  The other seven wallpapers stay. A saved choice is never replaced: only a settings file with no
  wallpaper in it gets the lake (test `the_lake_is_the_default_wallpaper_and_a_saved_choice_is_never_overwritten`).
  Existing installs that saved `serenity` on their first run (it used to be the written default)
  keep it: a file cannot say whether `serenity` was chosen or defaulted, and keeping it is the safe reading.
- **Settings > Appearance starts with theme cards**: Lake (charcoal and soft blue, the look the shell
  has always had) and Nightfall (deep blue-black, violet accent, the nightfall wallpaper). Each card
  shows its wallpaper and its own colours. Choosing one sets the colours, accent, wallpaper and dark flag
  together and also restyles labwc's title bars, menus and Alt+Tab list, new foot terminals and the GTK
  colour scheme, then asks labwc to reload.
- **The lock screen** (both the compositor's session lock and the shell's own fallback) is now: the
  person's wallpaper, softened; a large clock and the date; an avatar disc with their initial and name; a
  password field with the kit's eye to show or hide what was typed and an arrow to send it; a status line
  with network and, only on a machine with a battery, its percentage; "3 notifications" with "Details are
  hidden until you unlock" (a count, never content); Suspend, Restart, Shut down. Restart and Shut down
  ask once more (the button turns to "Confirm restart"). The `○/◉` text glyph is gone.

## What changed underneath

| Area | Change |
|---|---|
| Wallpaper default | `wire/settings.rs`: `DEFAULT_WALLPAPER = "lake"`, `WALLPAPER_PRESETS` public with lake first; `app_context.rs` uses both instead of its own copy of the list; `desktop.slint` and `settings.slint` list lake. |
| Themes | `crates/yantrik-design-tokens/themes/{lake,nightfall}.toml` (palette, accent, wallpaper, dark flag, whether the palette overrides the tokens). `wire/theme.rs` parses them, applies `ThemeOverrides` / dark / accent / wallpaper on the UI thread, and on a worker writes the labwc `themerc`, foot's `[colors]`, GTK `settings.ini` and `gsettings color-scheme`, then `labwc --reconfigure` (3 s timeout, only when the themerc changed). |
| Lock picture | One kit component, `YLockView` (`crates/yantrik-ui-kit/slint/lock_view.slint`), drawn by both lock screens; the shell's `lock.slint` and `yantrik-lock`'s `lock.slint` are thin wrappers. Lock tokens (`lock-*`) are in `theme.slint`; the kit gained an `eye-off` icon. |
| Blur | `lock_wallpaper.rs`: the wallpaper is blurred once, on a worker, when chosen (and at start if the cache is missing or for another wallpaper). Source is the embedded 320×200 preview (or a custom file shrunk to 480 wide); 2 box passes of 2 px; written as a PNG in `~/.cache/yantrik/` that both lock screens load. No live blur. |
| Lock client | `yantrik-lock` takes `--name --wallpaper --network --notifications` from the shell, reads the battery itself from sysfs each draw, and takes pointer input. |
| Parity | `describe shell` has `theme` (current and the list); `act shell set_theme theme=<id>` is **sensitive**, runs the same `theme::choose` the cards run, and answers with the theme the shell reports back. |
| Settings saved | `theme` and `theme_chosen` added to `settings.yaml` (both defaulted, so older files load). |

Themes are kept small on purpose: two themes, ten palette colours, and the frame/menu/OSD colours are
derived from the palette (`frame_colours`), so a new theme is one small file.

## What the lock client can now do (for the security reviewer)

Authentication is untouched: `lock::check_unlock`, PAM / `unix_chkpwd`, the delay after wrong tries, the
socketpair protocol (`locked`, `secret <text>`, `ok` / `no …`), `ask_shell`, the compositor session lock
(#313) and all of `lock.rs`'s and `session_lock.rs`'s tests are as they were. New:

1. **Pointer input** on the lock surfaces (`wl_pointer` with a themed cursor). Only the lock's own
   surfaces receive it, as with the keyboard. It can press the eye, the arrow (which calls the same
   `ask_shell` Enter calls) and three power buttons.
2. **Power buttons run `systemctl suspend|reboot|poweroff`** from the lock client, as the person's own
   session (logind / polkit's active-session rule), the same call `wire/power.rs` makes. None unlocks
   anything: suspend leaves the session locked; reboot and poweroff end it. Restart and Shut down need a
   second click. A reviewer may prefer the client to ask the shell to do it, over the existing channel;
   that is a small change and is the one thing I would ask them to decide.
3. **The shell passes the client four more flags**: the person's name, a network name, a notification
   *count*, and the path of the blurred PNG. No secret, no notification text.
4. **The typed entry is shown only while the eye is open**: the view is given `revealed-text` only then;
   when the eye is shut the view holds no copy of what was typed. (`App.entry` already held it, as before.)
5. The shell's own fallback screen: its `FocusScope` now sits beneath the picture instead of over it
   (it took clicks for itself, so the eye, arrow and power buttons never heard them), and its real
   `TextInput` is made not to paint (transparent, 1px face): a clipped or faded input still painted its
   text cursor, a white line, at the top left of the software-rendered screen. The first key still only
   hands the keyboard to the input, as before. Typing, Enter, `try-unlock` and the PAM check path are
   unchanged.

## Verification

Commands and results (exact counts are in the PR body too):

- `cargo test -p yantrik-ui --bin yantrik-ui`: 1227 passed, 1 failed, 1 ignored (after the rebase onto main at 9b462c3). The failure is
  `harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`, which fails here for a
  reason outside this change (see below). New tests: 12 in `wire::theme`, 6 in `lock_wallpaper`, 2 in
  `wire::settings`, 3 in `control_theme`, 1 in `session_lock`.
- `cargo test -p yantrik-ui-kit`: 15 passed (the idle-window and Timer scans cover `lock_view.slint`).
- `cargo test -p yantrik-lock`: 3 passed (new).
- `tests/ui-preview`, built with `--profile fast`: `verify-lock` at 1280×800 and 1920×1080,
  `verify-themes` at 1280×800, `verify-lake-desktop` at 1280×800, all PASS; the existing
  `verify-settings` at 1280×800 PASS (its click coordinates moved by the height of the theme cards).
  Screenshots are in `review/ui-lock-themes/`.

The scene for the lock screen presses the real thing: it types, opens and shuts the eye (the dots become
the text and back, pixel for pixel), sends with Enter and with the arrow, clicks Suspend, and clicks
Restart and Shut down twice. It also asserts that the screen asks for no redraw over 1.2 s at rest.
The shell's own screen and the compositor's `LockView` render the same pixels (their PNGs are the same
size to the byte), because both are `YLockView`.

The picture compared with the render: `design/renders/03-lock-screen.jpg` is crisper and darker than the
shipped lake can be (it has snow-lit peaks, and the clock has to read over them), so the wallpaper is
dimmed by a scrim and softened; the layout (clock and date above, avatar, name and field below the middle,
status top right, notifications bottom left, power bottom right) follows the render. Differences: the
avatar disc is neutral, not teal (teal is for minds); no "Minds paused while locked" line (nothing here
knows that is true); no Accessibility button (nothing to open yet); no keyboard layout (no reading of it).
The blurred wallpaper comes from the 320×200 preview, so on a 1920-wide screen it is soft to the point of
a smudge; a larger source for big screens is the obvious next step.

Not run here: a real labwc (so `labwc --reconfigure`,
the compositor lock with a real pointer, and the cursor on a lock surface are checked by their tests and
by reading, not on a screen); the `cargo build --workspace --locked` and `cargo test --workspace --locked`
CI jobs; the selftests in `docs/CONTRIBUTING.md`.

## Found and not fixed (other stories)

- `tests/ui-preview` did not build on `main`: `src/osd_tests.rs` uses `OsdWindow`, which `preview.slint`
  never exported. One line added here, because nothing could be verified without it.
- `harness_install::a_coloured_installer_reaches_the_row_as_plain_text` fails in this cloud sandbox: the
  job runs `sh -lc`, and the sandbox's login profile prints `nvm` first. Unrelated to this change.
- A theme changes the palette but not the other window frames' *size* (title bar height etc.): by
  design, sizes stay in `config/labwc/themerc`. The deploy script copies the shipped themerc on every
  start; the shell rewrites it only after a theme has been chosen, so a machine that never chose one is
  left exactly as it was.
- No light theme ships. The dark/light switch still works on top of a theme; a theme's own palette gives
  way to the stock light colours in light mode (overrides are mode-blind).
- foot reads its colours at start: terminals already open keep theirs.
- The lock shows an initial, not a photo: the shell has no per-user avatar image to read yet.
- Keyboard layout and media controls from plan 6.2 are not on the lock screen: the shell has no
  keyboard-layout reading, and a media control on a locked screen needs its own review.
