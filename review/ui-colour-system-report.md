# UI colour system — report

Branch: `ui/colour-system` (from origin/main). Security review: approval card buttons restyled; behaviour unchanged.

## What a person sees
- One system accent, soft blue #8FB4E3 (dark label #0E1215). It means ON and PRIMARY.
- Buttons come in three kinds: primary (filled blue), secondary (transparent, 1px #4A535B outline, white label), destructive (filled red #E5484D, white label). Hover and pressed are neutral steps; focus keeps the 2px white ring.
- Toggle tiles: off = opaque #1E252B with a #2C343B border (visible on a #151A1E panel); on = filled blue with dark icon and text.
- Sliders: fill and knob are blue; track #343B41.
- Approval card: "Allow once" is the primary, "Deny" secondary.
- Selections (lists, Alt+Tab, taskbar active window) are neutral with a white keyline, not the accent.
- Delete / Erase disk and install / Delete this record are destructive; the Shut Down row text uses the destructive red.

## What changed
- `theme.slint`: accent preset 0 is soft blue (light-mode variants added); tokens `destructive*`, `text-on-destructive`, `tile-off*`, `tile-border`, `outline-border`; `track-bg`; `text-on-accent`.
- `y_button.slint`: kinds 0/1/3. Variant 2 ("tertiary") is kept as an alias of secondary; all call sites moved to 1. New `pointer-only` property: the button never takes focus, so Tab can't reach it and Enter/Space can't press it.
- `y_toggle_tile.slint`, `y_slider.slint`, `y_list_item.slint`: as above.
- Duplicate button styles removed: PillButton (now a YButton wrapper, with `destructive`), problem_report FlatButton, snippet_manager and spreadsheet hand-built Delete buttons, the approval card's hand-built Deny/Allow rectangles.
- New scene `verify-colour-system` (`tests/ui-preview/colour_system_probe.slint`, `src/colour_system_tests.rs`, added to validate.sh): reads pixels for each kind and tile state, and checks that Tab/Enter/Space never press the approval pair while a click does.

## Approval security properties
The card's buttons are `pointer-only` YButtons, so nothing is focused when the card appears and Enter/Space cannot press them (previously they were plain TouchAreas, equally unfocusable). `control_approvals.rs`, `card_watch.rs` and the callbacks (`deny`, `allow`) are untouched.

## Verified
- `cargo test -p yantrik-ui-kit`: 7 passed, including `an_idle_window_stops_drawing` (2 tests).
- `cargo check -p yantrik-ui`: passes (compiles all .slint).
- `cargo check --manifest-path tests/ui-preview/Cargo.toml --profile fast`: passes (the new scene and all existing scenes compile).

## Not verified
- The ui-preview binary could not be linked: rustc is OOM-killed (~14 GB) compiling the Slint-generated code in this 15 GB sandbox, even at opt-level 0. So no `verify-*` scene (including `verify-colour-system`, `verify-kit-controls`, `verify-cards-waiting`, `verify-qs-levels`, `verify-idle`) was run and **no PNGs were looked at**. The pixel assertions in the new scene are unrun.
- `cargo test -p yantrik-ui --bin yantrik-ui`: NOT run. rustc was OOM-killed compiling `yantrik-ui-slint` (same ~14 GB limit). The `control_approvals.rs` and `card_watch.rs` tests were not run here; neither file is changed.
- Colour consequences to eyeball: every `Theme.accent` user is now blue, including mind surfaces that used the old teal accent; `Theme.cyan` (mind teal) and amber are unchanged. The launcher button and unread dots are accent-filled. Light-mode values are my picks.
- Not changed: the approval card's third "Allow … for this session" row (deliberately the quietest control), PowerButton icons in the launcher footer, app-local ribbon/icon buttons; there is no power-off confirm dialog to restyle (the power menu rows run immediately).
