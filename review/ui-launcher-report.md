# Launcher and consistency sweep — report

Branch `ui/launcher-and-consistency`. Story: the Apps launcher (spec §3, plan PR 6.4's consistency sweep).

## What a person sees

- **One Apps launcher**, opened by the dock's Apps button and Super+Space: an opaque centred panel (`#151A1E`, 1px border, 16px radius), the search field focused on open.
  - **Running** first: the apps that have a window, with a numeral on one that has several. It is the dock's own rows, so the launcher, the dock and `describe shell` agree.
  - **All apps** under it: a grid of the same full-colour rounded-square `AppTile` the dock draws, only bigger.
  - Typing filters both sections. Enter opens the top match (a running app comes first). The arrows move, crossing from Running into All apps by column. Esc closes. Enter with nothing matching does nothing.
  - A mind's apps never appear: Running is read from the dock's rows, which come from the person's window list, which leaves out what a mind opened (a source-scan test holds this).
  - Opening goes through the existing `launch_app` path. A running tile focuses the open window instead of starting a copy.
- The old category rail is gone (search replaces it); pins, the power row and the rescan-on-open stay. There is one launcher: `AppGrid` was rewritten in place, not added beside.
- **One desktop home.** Agent mode is `DesktopHome` with `agent-mode` set: it adds Suggestions and the day-one card and drops the recents and the mind line. The ~380-line twin in `desktop.slint` is deleted.
- **Screens 21, 27, 28** (Packages, Devices, Permissions) carry their own window buttons, and their frame draws no bar. Packages and Devices put them in their existing header row; Permissions had no title row, so it gets one thin 36px bar (its name and the buttons). Packages' header also stretches now, so the buttons sit in the corner.
- **A restored window has one bar.** The frame's bar is not drawn for any screen that carries its own controls (it was drawn when restored); a drag strip under the header moves the window and a double-click maximises. Applies to all eleven hosted screens.
- **Command palette** (`Ctrl+Shift+P`): the input row now shows a search icon, a stretched field with a placeholder, and takes the keyboard (Esc, arrows, Enter). The panel is solid. The old one collapsed the field inside a centred row and never took focus.
- **Button kinds:** Packages' "Upgrade All" was a bespoke green pill; it is now `YButton` primary.

## Control surface

- `describe shell` → `launcher`: `{ open, query, running: [{app, label, windows}], apps_shown }` (was `{ open }`).
- `open_app name=launchpad` / `show_screen screen=launchpad` now call `card_watch::hold_windows("open_launcher")` first (the launcher comes over the shell); the answer is unchanged otherwise. A source-scan test checks the order.
- No new action: opening an app by name already exists (`open_app`).

## Files

Added: `crates/yantrik-ui/src/wire/launcher.rs`, `tests/ui-preview/src/launcher_tests.rs`, this report.
Changed: `app_grid.slint` (rewritten), `desktop_home.slint`, `desktop.slint`, `window_frame.slint`, `command_palette.slint`, `app.slint`, `package_manager.slint`, `device_dashboard.slint`, `permission_dashboard.slint`, `wire/app_grid.rs`, `wire/dock_bar.rs`, `control.rs`, `icons.rs` (the category label the rail used), `tests/ui-preview/{preview.slint,src/main.rs,validate.sh}`.

## Left out, on purpose

- **`status_bar.slint` and `quick_settings.slint`:** two other agents are rewriting them, so neither was touched.
- **Retiring the legacy `font-*` tokens:** not possible without touching `quick_settings.slint` (it still uses six of them), and ~380 uses remain in ~40 other files whose sizes differ from `fs-*`. Needs its own change after the Quick Settings rewrite lands.
- **Command palette into the Lens:** fixed in place instead; folding it into a 2,300-line component is a separate piece of work.
- **Bare Super key:** labwc's `rc.xml` binds `W-space`. I could not check on a real compositor that a modifier-only keybind fires on release without breaking the Super chords, so I did not add one.
- **Agents (34) and Recipes (35)** still use the frame's bar with no header ("third look" in the audit); not in this story.

## Found, not fixed

- `tests/ui-preview` did not compile on main: `osd_tests.rs` uses `OsdWindow`, which `preview.slint` never exported. One export line added (needed to run anything).
- Devices' tab-bar buttons (AI Explain, Export, Refresh) are top-aligned in the 48px bar, not centred (visible in `restored-window.png`).
- The restored frame's top corners are square under the software renderer (the screen's header paints its own corners; the old frame bar rounded its own). Cosmetic; visible only restored.
- Right-click on an agent-mode pin tile used to unpin it; the unified home does not have that (the launcher's pin badge does).
- The audit text says Packages/Devices/Permissions "use AppHeader"; in the code they draw bespoke rows, which is why they needed buttons added before the frame bar could go.

## Verified

Run on a 4-core, 15 GB box with a 12 GB swapfile, `CARGO_BUILD_JOBS=2-3`, `CARGO_PROFILE_DEV_DEBUG=0`; no OOM.

- `cargo test -p yantrik-ui-kit`: 15 passed, 0 failed.
- `cargo test -p yantrik-ui --bin yantrik-ui`: **1193 passed, 1 failed, 1 ignored** (after rebasing onto main at a47437a) The one failure, `harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`, gets `"nvm\n✓ uv ready"` for `"✓ uv ready"`: the login shell the job runs prints `nvm` from this sandbox's profile. That file is untouched here; I did not run it on a clean `main` to confirm.
  - One test needed updating for the move: `lens::tests::the_ask_bar_hints_say_the_key_rc_xml_binds_to_open_lens` scanned `desktop.slint` for "Super K"; the chip now lives only in `desktop_home.slint`.
  - New tests: `wire::launcher` (matching, describe shape, Running reads only the dock's rows, the dock refresh updates the launcher), and `open_launcher` asks `hold_windows` first.
- ui-preview (`cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- … <scene>`), all PASS: `verify-launcher-scenes` (new), `verify-launcher`, `verify-apps-button`, `verify-dock`, `verify-screen-controls` (28 screen/state pairs), `verify-apps`, `verify-mind-panel`.
- Not run: the rest of `tests/ui-preview/validate.sh`, `cargo build --workspace --locked`, `cargo test --workspace --locked`, the CONTRIBUTING selftests, and anything on a real compositor (Super+Space, `act shell` calls).
- Screenshots (under `target/ui-validation/`, not committed): `launcher.png`, `launcher-search.png`, `restored-window.png`, `restored-packages.png`, `desktop-agent.png`.
