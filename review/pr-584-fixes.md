# PR #584 fixes: the Agents workroom

Review: `review/pr-584-review.md` (verdict MERGE AFTER FIXES; no BLOCKER). Branch `ui/agents-workroom`,
rebased on `origin/main` (no conflicts).

## Findings

| # | Finding | Fix |
|---|---|---|
| 1 SHOULD-FIX | Magic px and numeric literals in the four new Slint files (about 158 `px` and ~20 weights) | Every size, spacing, radius, stroke and font weight in `agents_workroom`, `agents_start`, `agents_nav` and `agents_task` now comes from `Theme`: the existing `sp-*`, `r-*`, `h-*`, `fw-*`, `hairline`, `focus-ring-width`, `bar-icon-size`, plus a new `wr-*` block (the spec's 44/64/84/104/248/320/440/480 and the mark sizes) and `sp-half`/`sp-1h`/`sp-2h`/`sp-3h` half-steps in `theme.slint`. Only `0px` is written out. Tests: `the_workroom_screens_use_tokens_and_no_pixel_or_weight_literals`, `every_theme_token_the_workroom_screens_name_exists`. Look change to check on a screen: the count badge's pill radius is `height / 2` (was a literal 10px, same look); nav item radius 10px is `r-md` (8px); the nav header weight 700 is `fw-headline` (600) |
| 2 SHOULD-FIX | Size and one concern | The fixes are three commits on top of the two already on the branch (screen, then `show_workroom`), each reviewable or bisectable alone: the approvals popup predicate and its tests; the `show_workroom` read-back and docs; the tokens and the child-runs list (those two share the Slint files, so they are one commit). **Not done:** moving the desk menu and confirm dialog out of `agents.slint` (1,252 lines): they bind `AgentsState`, which is defined in `agents.slint`, so a new file importing it would be a circular import. The right split is `AgentsState` to its own file first, which touches every screen of the Agents UI and is a follow-up. The `show_workroom` commit (`control_workroom.rs`, a describe block, docs) is separable and a reviewer who wants it as its own PR can cut it at its commit |
| 3 SHOULD-FIX | `pane_agent` changed semantics; only a string-presence test | The predicate is now a pure function, `pane_agent_of(on_agents_screen, detail_open, selected)`, which also treats an empty key as "no pane" so the popup keeps the card. Behavioural tests: `the_popup_hides_a_card_only_while_its_runs_pane_is_open`; and `nothing_that_clears_the_selection_leaves_the_pane_flag_open` checks `refresh` closes `detail_open` whenever nothing is selected or the run is gone, and that every `selected = None` in `wire/agents.rs` is followed by `set_detail_open(false)` or a `refresh`. I checked the paths the review named: 155 and 1636 set it true after setting the selection (and `refresh` runs straight after); 272, 325, 557 to 560 clear it; 380 sets it after selecting the new agent; `leave_run` clears both. **Security boundary:** this is `control_approvals.rs`; the playbook asks for a security review of this file |
| 4 SHOULD-FIX | Lost coverage: sub-agents of a run | The task detail lists the runs the opened one started ("Started by this run": who, title, state in the screen's words; choosing one opens its run), from `Store::children_of`, scoped to the run chosen. Test: `the_task_detail_lists_the_runs_the_opened_one_started`. Stop on a parent still stops them (`launch::stop_on`, unchanged) |
| 5 NIT | `on_stop` / `on_start` block the UI thread; 250 ms repeating TICK | Not changed: pre-existing and unchanged by this PR (noted in the review as not a regression). **Follow-up to file:** move `launch::stop`/`start` to a worker and replace the TICK with event-driven refresh |
| 6 NIT | `show_workroom`'s `raised` / `narrowed_to` not fed back as errors | The answer carries a `note` when the page or narrowing is not as asked, the screen is not showing, or the shell could not be raised. Test: `the_answer_says_when_the_screen_is_not_as_asked` |
| 7 NIT | UI-only sequence and `workroom_minds_now()` locks | No change (the review found them fine) |
| 8 NIT | `docs/app-control.md` paragraph too long | The workroom sentence is its own paragraph, with the read-back note |
| 9 NIT | `app.slint` export list | No change |

## Tests

`cargo test -p yantrik-ui --bin yantrik-ui` (CARGO_BUILD_JOBS=1, CARGO_PROFILE_DEV_DEBUG=0, CARGO_INCREMENTAL=0, 12 GB swapfile): **1027 passed, 1 failed, 1 ignored**. The failure, `harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`, is this sandbox's shell printing `nvm` ahead of the installer's output (`left: "nvm\n✓ uv ready"`), in code this PR does not touch. One existing test, `the_catalog_dialog_offers_every_role_and_says_the_list_scrolls`, matched the literal `2px` in `agents_start.slint`; it now matches `Theme.sp-half`. New: `the_popup_hides_a_card_only_while_its_runs_pane_is_open`, `nothing_that_clears_the_selection_leaves_the_pane_flag_open`, `the_answer_says_when_the_screen_is_not_as_asked`, `the_task_detail_lists_the_runs_the_opened_one_started`, `the_workroom_screens_use_tokens_and_no_pixel_or_weight_literals`, `every_theme_token_the_workroom_screens_name_exists`; all pass. The Slint crate compiles with the new tokens and the child-runs list.

## Not verified / left out

- Rendered previews (`tests/ui-preview`) were not run for the token change: the Slint crate takes ~10 minutes
  and 14 GB to build, and the small look changes listed under finding 1 were not looked at as pixels.
  Run `verify-agents*` and look at the PNGs before merging.
- `agents.slint` was not tokenised (about 90 `px` lines, mostly inherited from the old screen); the review
  asked for the new files and the spec numbers.
- Pause/Resume, Take over, desk previews, Queued: still left out, as the PR description must say.
