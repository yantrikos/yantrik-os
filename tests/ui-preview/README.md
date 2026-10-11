# Native shell visual checks

This headless runner compiles the production Slint desktop, launcher, taskbar,
status bar and UI kit. It renders with Slint's software renderer; it does not
start services, attach a mind, or replace the running desktop. Sample names and
system readings are fixture data. The `app` scene demonstrates the shared frame,
not a running Notes application.

Run the complete interaction and responsive rendering checks with
`bash tests/ui-preview/validate.sh` after dependencies have been fetched. It uses
the existing `CARGO_TARGET_DIR` when set and writes scene images under
`target/ui-validation/`.

From the repository root, on a machine with the normal Slint build dependencies:

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/desktop.png 1280 800 desktop
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/desktop-light.png 1280 800 desktop light
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/agent.png 800 600 agent
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/launcher.png 800 600 launcher
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/frame.png 800 600 app light
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- verify-controls
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/unused.png 800 600 verify-launcher
```

The last two commands dispatch actual keyboard and pointer events. They assert:

- Enter and Space activate the focused shared button.
- A focused button cannot fire after becoming disabled or loading.
- Re-enabling restores activation.
- Disabled inputs cannot be cleared; enabled inputs can.
- Launcher selection opens the expected application and closes the overlay.
- A reopened launcher can be dismissed with Escape.

The runner is a separate Cargo workspace so production release builds do not
include test fixtures. Its lockfile pins the same Slint version as the OS.
For complete-shell integration, also run `cargo check -p yantrik-ui` and
`cargo test -p yantrik-ui-kit -p yantrik-design-tokens` in the main workspace.

## Actual app interiors

The `notes`, `files`, `files-grid`, `files-empty`, and `settings` scenes instantiate
the production NotesEditor, FileBrowser, and SettingsScreen components. Their
content is deterministic fixture data, and service callbacks are recorded without
performing disk operations. Unlike the `app` scene, these are actual app interiors.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/notes.png 1280 800 notes
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/files.png 800 600 files-grid
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/settings.png 1280 800 settings light
```

`files-home` (grid) and `files-home-list` draw Files at home as the desk-and-mind redesign
has it: the places sidebar, folder tiles with item counts and "changed … ago" (one folder
whose count could not be read, drawn as unknown), and the recent row under the grid.

A target directory shared with another checkout of this repository can run THAT checkout's
build script: the preview's build.rs bakes in `CARGO_MANIFEST_DIR`, and cargo hashes a path
package by its path relative to the workspace, so the script from the other tree is reused
and compiles the other tree's .slint files. The symptom is an error about a property that is
plainly in the file. Use a target directory per checkout, or
`cargo clean -p yantrik-ui-preview` before building.

App interaction regression checks (1280×800):

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/unused.png 1280 800 verify-apps
```

This sends real pointer/key events to the production components and checks theme
selection, repeated activation of the active theme, wallpaper keyboard activation,
disabled history navigation, and distinct file-grid hit targets across rows.

The file-grid probe resizes the native window from 1280×800 to 800×600 and verifies
that the last file remains selectable on the wrapped second row. The standalone
Notes wrapper should also be checked with `cargo check -p yantrik-notes`; its
preferred-size constraints can reveal resize cycles hidden by a fixed fixture.

`verify-overflow` exercises the production app header with keyboard and pointer:
disabled commands, arrow/Enter selection, the final menu command, and Escape.
`verify-idle` settles a production Terminal screen, then checks for zero requested
redraws over 1.5 seconds. This is a render-loop regression check, not a substitute
for measuring the deployed process's CPU and memory.

The app probe also switches Notes side panels, saves with the assistant open,
and resizes from a wide window with both panels to a compact window. It checks
that the writing area is not squeezed between both panels at compact widths.

## Agents

`verify-agents` draws the production Agents screen (and an agent's own window) from
fixture data — two agents, pi's session with a finished command's terminal opened, a
reported call, a running command's live output and a failed one — and sends real
pointer events: the list reports the pointer over it and leaving it (so rows hold
still under it), a row selects, a tab filters, Stop reaches the agent shown and a card
opens from its line. It writes the screen, the screen in the light theme, and the
window beside the output path.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/agents.png 1280 800 verify-agents
```

`verify-agents-catalog` draws the same screen with an agent started from the agent catalog — its
row names its role, its details say the role's reach — and New agent opened "from the catalog",
listing the shipped roles with their purposes. Real pointer events pick a role, press Start (which
hands the picked role its task, and starts no mind) and switch the dialog back to "A mind". It
writes the dialog, the role picked, the "A mind" view, the light theme and the role's row beside
the output path.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/agents-catalog.png 1280 800 verify-agents-catalog
```

## Mind panel

`verify-mind-panel` draws the whole shell — app.slint's `App`, every screen, the status bar,
the taskbar and the mind panel — from fixture data (two agents, one waiting for the person, a
recipe in flight, three recent actions; the names are fixtures, not a running machine) and sends
real pointer events: an agent's row opens the Agents screen on the Active tab with that agent
selected, the strip is present over the Agents and Files screens and opens the panel there, and
the chevron folds it. With the panel open beside Files, a click at the far right of the title-bar
row must land on the panel, not on the Files window's × — the maximized window stops short of
the panel. It writes the desktop (dark, light, agent mode, and with nothing known yet) and the
Agents and Files screens with the strip and with the panel open, beside the output path.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/mind-panel.png 1280 800 verify-mind-panel
```

## Recipes

`verify-recipes` draws the production Recipes screen from fixture data — a recipe waiting on
the person's answer, one running, one waiting on a timer, one paused, one failed on an unbound
`{{topic}}`, one done after taking a jump, and a built-in never run — and sends real pointer and
key events: a choice answers, a typed answer goes on Answer and on Enter and never empty, a row
opens from its name, a tab filters, Pause reaches the recipe shown, and Cancel asks before it
cancels. It writes the screen, the screen with a running recipe opened, the cancel question, the
Finished tab with the failed recipe opened, and the light theme beside the output path.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/recipes.png 1280 1000 verify-recipes
```

## Approvals

`verify-approval-card` draws the Lens waiting on the longest real approval card — the paragraph
`shell` publishes for `run_recipe`, an agent row, two arguments, the session row — at 1280×800,
and sends real pointer events: it scans for Deny and Allow the way a person looks for them and
asserts both answer, on one row, with the whole button inside the panel above the reply box, that
the session row answers too, and that the card leads with the description's first sentence (#218).

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/approval-card.png 1280 800 verify-approval-card
```

## Review stills (light theme and Nightfall)

`review-still <name>` renders one still for a design review, in a theme a test VM cannot be
switched into without a pointer. Each run renders one still because every scene shares the one
headless window. There are no assertions: these are pictures.

- `appearance`: Settings → Appearance, light, soft-blue accent (also writes `-scrolled`).
- `quick-settings`: the whole shell with Quick Settings open, light, laptop shape.
  `quick-settings-dark` is the same in dark, to hold beside it.
- `lens-composer`, `lens-composer-dark`: the chat panel's composer with the line saying where the
  words go and the "Mode:" chip, at 1280×800.
- `app-header`: Calendar at 1280×800 with an ordinary day, light.
- `approval`: one ApprovalCard at its natural height (440 wide, on the Lens panel's ground), light.
- `approval-nightfall`: the same card on Nightfall: dark, violet (accent index 2) and its palette
  overrides from `themes/nightfall.toml`, applied as `wire/theme.rs` applies them.
- `approval-lens`, `approval-lens-nightfall`: the card in the Lens at 1280×800, where its details are clamped.
- `icons-desktop`: the whole shell's desktop at 1280×800 — the workspace row's 48px tiles and the
  dock's 32px ones, including a third-party app with a theme icon and one with neither — dark,
  then light (`-light`), and the dock alone at 4x, pixel for pixel (`-dock-zoom`).
- `icons-launcher`: the Apps launcher open over the desktop, every tile at 48px, dark and light.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/light-appearance.png 1280 800 review-still appearance
```

## App labels

`verify-labels` draws the kit's AppLabel (the name under every app tile: 13px medium, on whole
pixels) on the lake wallpaper across the sky, the ridge and the reflection, beside the same names
in the card ink, and on a glass card the way the desktop's workspace draws them. It asserts that
a label on the wallpaper is white and that its 1px shadow darkens the wallpaper under it, on the
brightest row.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/labels.png 1280 800 verify-labels
```

## Taskbar window menu

`verify-taskbar-menu` draws the whole shell — app.slint's `App` — with two fixture windows on
the taskbar and sends real pointer and key events (#232): a right press on an entry opens that
window's menu without activating it, the menu draws above the bar, choosing Close fires the
action callback under the control surface's own name for it (`close_window`), Escape puts the
menu away, a left click still activates the window, and the Menu key and Shift+F10 open the
focused entry's menu. It writes the shell with the menu open, and with it opened from the
keyboard, beside the output path.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/taskbar-menu.png 1280 800 verify-taskbar-menu
```

## System Monitor

`verify-monitor` draws the production System Monitor screen from fixture data — four cores and
two disks at part usage, three processes — and pins the layout a person complained about (#220):
every bar fill (health, swap, each core, each disk) starts at x=0 of its track instead of being
centred inside it, selecting a process does not grow the header or move the list under the
pointer, an empty list echoes the filter word ("No process matches 'xylo'") and only a machine
that reported nothing says "No process data", and the AI workloads card says plainly that nothing
is measured while no source feeds it, and drops that line the moment a model name reports.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- verify-monitor
```

## Text Editor

`verify-editor` draws the production text editor (`apps/text-editor/ui/app.slint`) holding the
very document from issue #328 — the file whose rendering crashed the VM's window in euclid's
i16 coordinate cast — and asserts it renders instead of panicking (#328). Oversized documents
of the shapes that crashed are windowed by the editor's own logic and checked in
`cargo test -p yantrik-text-editor`.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/editor.png 1100 760 verify-editor
```

## Weather

`verify-weather` draws the production Weather dashboard from fixture data with the context rail
open, the way the running app has it, and measures the hero's stat-tile grid against the space
the rail leaves (#220). At the app's default 1000×720 window the tiles' right edge must stay
inside the content area — they used to run under the rail — while remaining wide enough to read;
at 1400 wide the grid must return to its 499px design width.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- verify-weather
```

## Installer

`verify-installer` draws the production installer (`installer.slint`, #400) — Welcome, You, Disk,
Review, Installing, Installed — from fixture disks and layouts, with the production rules
(`crates/yantrik-ui/src/installer_rules.rs`, included by path) behind every field, at 1280×800
and at 800×600. It sends real key and pointer events: the layout list opens and a row picks a
layout the shell is told about, the name field takes typing on arrival and the username and
computer name follow it, a password mismatch keeps Next disabled and a pointer on Next does
nothing, a written username is not overwritten, a disk is chosen by pointing, a timezone the
zone database lacks blocks Install, Install hands the shell exactly what was chosen, 100%
becomes Installed, and Restart now reaches the shell. It writes each screen beside the output
path.

```sh
cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/installer.png 1280 800 verify-installer
```
