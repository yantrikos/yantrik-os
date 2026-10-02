# PR #581 fixes: the window switcher, reworked so Alt+Tab always works

Review: `review/pr-581-review.md` (verdict DO NOT MERGE). Branch `ui/alt-tab-switcher`, rebased on
`origin/main` (conflicts in `rc.xml`, `app.slint`, `control.rs`, `docs/app-control.md`; both sides kept:
the cheat-sheet `@help` comments and `cheat_sheet` describe/panel from #578 stay, as does the switcher's).

## The design change

Alt+Tab is labwc's own window cycling again (`NextWindow` / `PreviousWindow`). Hold Alt, tap Tab, release
Alt switches, inside the compositor, so it works when the shell is busy, hung, held by an approval card or
not running. Nothing about Alt+Tab goes through `yos` any more.

- **Look.** labwc's OSD is themed to the shell's card in `config/labwc/themerc` (`osd.*`: #151A1E panel,
  neutral edge, 16px padding, a 2px white ring on the selected window, no teal) and `<windowSwitcher>` in
  `rc.xml`. labwc draws a plain list of titles; it has no thumbnails here, and no preview is claimed.
- **The shell's card** is kept only as a separate overview on **Super+Tab** / **Super+Shift+Tab**
  (`open_switcher`). It never replaces the native path. A test pins both.
- The duplicate switcher was already gone from this PR (`window_switcher.slint`, the desktop's
  `WindowSwitcher`); nothing else was truly unused. `wire/window_switcher.rs` is the taskbar's wiring and stays.

## Findings

| # | Finding | Fix |
|---|---|---|
| 1 BLOCKER | Alt release does not switch | Alt+Tab is labwc's again, so release switches natively. The card is Super+Tab (Enter/click switches, Esc cancels); its comments no longer claim Alt. Test: `alt_tab_is_labwcs_own_and_the_card_is_a_separate_super_tab` |
| 2 BLOCKER | No fallback, dies silently | Same change: no `yos` in the Alt+Tab binding, so a hung shell or a waiting approval card cannot stop it. `hold_windows` refusing the *overview* is now only the overview's refusal. Same test |
| 3 BLOCKER | Blocking process calls on the UI thread | `open` and `commit` hand the window listing, front lookup, raise and activation to a worker with `answer_later`; the UI callbacks spawn a thread; results reach the card by `upgrade_in_event_loop`. Every compositor request is bounded (1.2 s, `windows::ask_compositor`). Windows now come from the foreign-toplevel stream's in-memory copy, no process at all, and `wlrctl` only when the stream is off. Test: `nothing_blocking_runs_in_a_handler_or_a_callback` |
| 4 | Same-titled windows collapse | Cells keyed by the stream's toplevel id (`toplevel_watch::windows`, ids and app ids); ordering by id; activation by id (`toplevel_watch::activate`, via the stream's handle and seat). Same-titled windows are shown "Title", "Title (2)". Tests: `windows_with_the_same_title_are_both_kept_and_told_apart`, `windows_are_keyed_by_the_toplevel_id_not_the_title` |
| 5 | Closed windows stay listed | A failed activation removes that window by id, republishes, keeps the card up, and the answer says so. Tests: `a_closed_window_is_removed_and_the_selection_stays_put`, `a_closed_window_is_taken_off_the_card_which_stays_up` |
| 6 | Cancel raises a window with no hold | Cancel now asks `hold_windows("switcher_cancel")` before giving a window back; refused, the card is put away and the window left where it was, with a note. Tests: `opening_committing_and_giving_a_window_back_wait_for_a_card`, `a_held_cancel_puts_the_switcher_away_without_raising_a_window` |
| 7 | Mind View scope missing | Not built. Stated in the module docs and in `describe` (`left_out`). Left for its own story |
| 8 | Missing spec items undeclared | Footer class/workspace, "across all workspaces" evidence, real-aspect previews: listed in `describe` (`left_out`, `previews`). The labwc list does show every workspace (`allWorkspaces="yes"`) |
| 9 | Magic px in `alt_tab.slint` | All sizes/spacings/weights/strokes are tokens; new `sw-*` block in `theme.slint` for the switcher's own sizes and breakpoints. Verified by `grep` (no `px` literal but `0px`) |
| 10 | Hand-rolled page arrows | The kit's `YIconButton` with `Icons.chevron-left/right`. The key handler moved to the ancestor so keys still work after a click on a button (ui-preview step added) |
| 11 | Test gaps | Added: dedup, closed-on-commit, describe shape, hold/answer_later source order, labwc binding test rewritten. Not added: a `verify-idle` redraw count for the card (see below) |
| 12 | Misleading "not the desktop" error | "has no status bar yet" |
| 13 | `switcher_move` hard-codes 4 columns | Optional `columns` argument (1 to 4, default 4); `switcher_move_reads_the_columns_the_card_is_drawn_with` |
| 14 | Garbled doc comment | Rewritten with the ordering rule |
| 15 | Order source not stated | `describe` `order_source` says "compositor focus stream" or "listing order (the focus stream is off)" |
| 16 | `switcher_commit` grade implicit | `.risk("standard")` written; test asserts it |

## Tests

`cargo test -p yantrik-ui --bin yantrik-ui` (CARGO_BUILD_JOBS=1, CARGO_PROFILE_DEV_DEBUG=0, a 12 GB swapfile after the first build was OOM-killed): **1020 passed, 1 failed, 1 ignored**. The one failure, `harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`, is this sandbox's shell printing `nvm` before the installer's output (`left: "nvm\n✓ uv ready"`); it is in code this PR does not touch. The 20 new or rewritten tests here (`alt_tab` 10 incl. 2 new, `control_switcher` 11, `toplevel_watch` 1 unchanged) all pass: `cargo test ... -- alt_tab control_switcher toplevel_watch` gives 31 passed, 0 failed.

## Preview

`cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/ui-validation/alt-tab.png 1280 800 verify-alt-tab`
passes all its assertions, including the new one that Escape still reaches the card after a click on a page
arrow. Looking at the PNG (`alt-tab-20.png`) caught a bug the assertions did not: the first version of the
arrows had no size, so they filled the card and drew one chevron in the middle; they are now 28px compact
icon buttons at the card's two corners. The narrow (700 px) scene was not rendered.

## Not verified / left out

- No real labwc here: the themerc/`windowSwitcher` look, `allWorkspaces`, and `activate(id)` against a
  live compositor are unexercised. The OSD keys are labwc 0.7+; an older labwc ignores ones it does not know.
- `activate(id)` queues a `zwlr_foreign_toplevel_handle_v1.activate` on the stream's connection; its
  success is "the request was queued for a window still listed", not "focus moved".
- Card idle redraws (`verify-idle` style) were not added; the card has no timers or animations.
- Mind View scope, workspace in the footer and previews are still not built (findings 7 and 8).
