# PR #585 review fixes

Review: `review/pr-585-review.md` on `origin/review/pr-585-review`. Rebased on origin/main; #583 is not in main yet, so its one `taskbar.slint` edit (the running marker is `Theme.text-primary`, not the accent) had nothing to carry into. The dock's marker is already `Theme.dock-marker`, white. If #583 lands second its `taskbar.slint` hunk is a modify/delete: take the deletion.

| Finding | Fix | Test |
|---|---|---|
| B1 `#[allow(dead_code)]` on the wrong module | `mod dock_model;` is above the ICE note; the attribute is on `mod windows;` again, with a comment saying why | `dock_model::tests::the_dead_code_allowance_guards_windows` |
| S1 32px icons | `dock-icon` is 24px (spec §3); the false "spec allows 24-36px" comment is gone | `dock_icons_are_the_specs_24px` |
| S2 a title change kills hover and the list | `models::update` writes a same-apps refresh into the existing model row by row; only a changed set of apps replaces it | `models::tests::a_refresh_that_keeps_the_rows_updates_them_in_place`, `..._changes_the_rows_replaces_the_model` |
| S3 wheel at one position | One `YWheelArea` under the whole bar; every `DockBtn` forwards `wheel` (a button's own area accepts the scroll and was swallowing it over Apps and the mind button). The preview scene's sweep, which asserted nothing (`if false`), is now five real assertions | `the_wheel_pages_over_the_whole_bar` and `verify-dock` (Apps, middle app, both page buttons, mind button) |
| S4 margin and token can drift | Test parses `rc.xml` `<margin>` and compares top and bottom with the status-bar and taskbar tokens | `the_compositor_margin_is_the_docks_height` |
| S5 list not keyboard-reachable | NOT FIXED, left out: the list opens on keyboard focus and Escape closes it, but a Tab-only user cannot choose a row. Needs arrow-key handling in the button's FocusScope. | none |
| S6 spec items | NOT BUILT, listed here for the PR description: 64×40 previews, "Find a window" above 12 windows, the Apps "Running" section. Alt+Tab reveal is poll-driven (about 3 s): `dock_bar::publish` computes it from the polled window list, and there is no Alt+Tab code in this PR. | none |
| S7 untested claims | `describe_value` is split from `for_describe` so the `dock` shape (`buttons`, `page`, `needs_you`, per-button keys, `shown`) is tested without a screen; `mind_view_label` is a pure function with a test; the mind-apps exclusion is pinned by a source scan of `windows::shell_windows` | `describe_shell_dock_has_buttons_page_and_needs_you`, `a_minds_apps_stay_off_the_dock`, `the_dock_reads_the_shells_window_list`, `mind_view_is_labelled_by_the_mind_or_by_its_own_name` |
| N1 English where Tr exists | not changed ("Earlier apps", "Later apps", "Yantrik Mind"); the old taskbar's `chat-with` / `companion-count` are still dropped | |
| N2 "1 windows" | "1 window"; the stale count while windows close is not fixed (Slint cannot count a model by app in an expression) | `the_list_header_says_one_window` |
| N3 literals | tokens added: `dock-label-h`, `dock-list-head`, `dock-numeral`, `dock-desk-strip`, `dock-divider-inset`, `dock-check`, ratios; the 11px text uses `fs-micro` | |
| N4 old dock tokens | `dock-height`, `dock-icon-size`, `dock-item-size` deleted (no users); comment names `grounded_dock.slint` | |
| N5 orphaned comment in app.slint | the `dock-*` properties moved above the menu comment | |
| N6, N7, N8 | not changed (N7: clicking the front window's row minimises it, via `taskbar-window-clicked`; worth a follow-up) | |

## What was run
- `cargo test -p yantrik-ui --bin yantrik-ui` (12GB swap, `CARGO_BUILD_JOBS=1`, no debuginfo; the first run failed on a missing 'float' type, fixed): 1012 passed, 2 failed. One failure was mine (`the_dock_reads_the_shells_window_list` read a rustfmt line break) and is fixed (54 dock tests pass on re-run). The other, `harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`, is in code this PR does not touch; it fails the same way on #583's branch.
- `cargo test -p yantrik-ui-kit`: 7 passed.
- `verify-dock` rendered and looked at (`dock-list.png`): PASS, including the new wheel assertions.
- Not run: `verify-idle`, `verify-taskbar-menu` after the final edits, `cargo build --workspace --locked`, the full `cargo test --workspace`.
