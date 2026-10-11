# PR #573: fixes for security review round 3

Review: `review/pr-573-security.md`. Every HIGH and MEDIUM finding is fixed, and so are L3, L4 and L5.
Earlier fixes and tests are kept.

| Finding | Status | What changed | Test that would have caught it |
|---|---|---|---|
| H1 grade below the repo's | Fixed | `disconnect_network` and `set_wifi` are declared and published `dangerous` for every reading, as `wifi_disconnect` / `wifi_radio` and the Network Manager app already are. No count of links lowers them. | `disconnect_and_radio_off_are_dangerous_for_every_reading`, `the_shells_grade_matches_the_tools_for_the_same_verbs` |
| H2 stale grade, no re-check | Fixed | The handler compares the grade the gate asked about with the grade for the current state and refuses if it rose or the reading is stale (`grade_still_holds`). The work closure checks again against `network::read_fresh()` right before the D-Bus call. | `a_call_graded_at_one_level_is_refused_if_the_grade_has_risen`, `both_handlers_recheck_the_grade_before_acting` |
| M1 `connect()` could create for a mind | Fixed | `connect()` refuses `!by_person && plan != UseSaved` on its own fresh reading, before any D-Bus write (`permitted`). | `a_minds_join_may_only_use_a_saved_profile_inside_connect_itself` |
| M2 unbounded threads / connections / joins | Fixed | One shared system-bus `Connection`. A single join slot (`begin_join`) taken before the thread is spawned. A mind's joins are spaced 5 s apart. A mutex serialises the radio and disconnect calls. | `only_one_join_runs_at_a_time_and_a_mind_cannot_loop_them`, `the_slot_is_released_when_it_is_dropped`, `calls_share_one_connection_and_connect_requires_the_slot`, `a_minds_join_takes_the_slot_and_hears_a_refusal` |
| M3 mind disconnects a wired link | Fixed | The control surface calls `disconnect(false)`. Wired links are skipped, and refused with the reason when they are the only one. The person's popover Disconnect passes `true`. `dangerous` also means a person approves. | `a_mind_cannot_disconnect_a_wired_link`, `the_control_surface_never_disconnects_a_wired_link` |
| M4 listener exit freezes the picture | Fixed | The listener resubscribes with growing backoff and logs. While it is down, or a read fails, the monitor polls every 15 s. A failed read sets `stale`, and `describe shell` shows `network.stale`. | `a_dead_signal_listener_restarts_and_the_monitor_never_freezes_silently`, `describe_says_when_the_picture_is_stale` |
| L3 attribution | Fixed | The mark names the caller (`mind_view::requester_now()`), not the selected mind. | `the_mark_names_the_caller_not_the_selected_mind` |
| L4 inline D-Bus fallback | Fixed | `answer_later(..).or_else(|w| w())` is now an error. | `work_that_has_nowhere_to_go_is_refused_not_run_inline` |
| L5 poll timeout read as failure | Fixed | `join_step` treats "no answer" as keep waiting until the 45 s cap, and only a failed state deletes the profile. | `a_join_poll_with_no_answer_keeps_waiting_until_the_cap` |
| L1 `describe` lists networks and `known` | Left | Part of the parity requirement: a mind needs the list to pick a `connect_wifi` target. Limiting it is a product decision. | |
| L2 open evil twin autoconnect | Left | Needs a Slint change (caption) and a policy change on person joins. It needs its own PR and a rebuild of the UI crate. | |
| L6 secret-hygiene wording | Left | Comment rewording across files. Not exploitable, and outside this PR's concern. | |
| L7 panel stacking | Left | Cosmetic, in `control_overlays.rs`. A separate concern. | |
| L8 non-UTF-8 SSIDs | Left | Fails safe. The fix means carrying raw bytes through the model, which is a separate change. | |
| L9 `wifi_connect` tool takes a password | Left | Pre-existing, outside the diff. The review asks to track it separately. | |

## What was run, and what was not

* `cargo test -p yantrik-os --lib network`: 28 passed, 0 failed (21 before, 7 new here, 1 reworked).
* `cargo test -p yantrik-ui --bin yantrik-ui network`: **not run.** `yantrik-ui-slint` was killed by the sandbox (SIGKILL, out of memory) while compiling. No other `yantrik-ui` code compiled either, so the edits in `control_network.rs` and `wire/network.rs`, and the new tests in them, are **unverified by the compiler**. I read the diff for types, moves and the source-scan tests' string matches. Please run the command above on a machine with about 14 GB of RAM before merging.
* `cargo test -p yantrik-os` (whole crate) was not run in full; only the `network` filter was.
* No preview or screenshot checks: no `.slint` file changed.
