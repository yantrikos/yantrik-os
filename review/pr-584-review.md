# Review: PR #584 (ui/agents-workroom) - Agents workroom

## Summary
Replaces the Agents list/details screen with the spec section 2 workroom: header counts, nav (Workroom / Needs you / History / MINDS), decision shelf, desk cards, task detail, Start-work sheet, History. It adds one control action, `show_workroom` (safe), and a `workroom` block in `describe shell`. It changes `pane_agent` in control_approvals.rs. It deletes agents/route.rs, agents_route.slint and the route preview test.

Done well: approval handling is not weakened. Shelf actions only open the run; Allow/Deny still live in the shared ApprovalCard bound to the request id, and there is no Enter/autofocus path. `show_workroom` calls `hold_windows` before moving anything and has a source-order test. It reads the page back from the screen. It is correctly `safe` (it only changes the page; mind arg validated against real minds). No dangling references to route/RouteStopData/old tab props remain on the branch (checked with git grep). The state vocabulary is present (wire/agents_workroom.rs:296, 622-632). The new Slint is honest about missing features: it draws no Pause and no preview.

Scope vs spec: this is a deliberate subset. Pause/Resume, Take over / Enter Mind View, desk previews (max 12, visible-only refresh), "View changes/View activity" overflow and Queued state are NOT built, because nothing in the shell can do them yet. So no unsafe pause/take-over actions exist to grade, and nothing claims exclusivity. The PR description must say these are left out.

## Findings

### BLOCKER
None found.

### SHOULD-FIX
1. **Magic px / numeric literals in new Slint, against the playbook ("tokens, not literals")**: agents_workroom.slint (about 55 lines, e.g. :100 `height: 84px`, :112 `padding: 14px`, :210-228, :256-260), agents_start.slint (about 51), agents_nav.slint (about 28), agents_task.slint (about 19). Colours are tokenised (no hex found) and `Theme.mind` / `fs-page` were added, but sizes/spacings are literals. Failure: a later density or scale change misses these. Fix: add size and spacing tokens to theme.slint (card heights, 44px header, 64px nav rows from the spec) and use them. Many may be inherited from the old agents.slint style; at minimum the spec numbers (44/64/320/16) should be tokens.
2. **One concern / size**: 3919 insertions across 25 files: a screen rewrite, the removal of route, a new control action, an approvals-pane predicate change and a theme token, all in one PR. Two commits exist (50f0160 screen, 37384e7 show_workroom) and split cleanly. The wire/agents.rs rewrite (2678 on main -> 2651, +673/-673 churn) and agents.slint (1412 -> 1245) remain very large files, against the "small files" rule. New files are fine (agents_workroom.rs 1139 incl. about 400+ test lines; agents.slint still 1245). Fix: ask for the control surface commit to land as its own PR, or at least confirm the reviewer can bisect by commit. Move agents.slint overflow (desk menu, confirm dialog) into agents_task/agents_workroom.
3. **control_approvals.rs:1606-1621 `pane_agent` semantics changed**: the popup now suppresses an agent's card whenever `detail-open` is true and the selected key's agent matches. Previously the gate was `view == "list"`. Scenario: if `detail_open` stays true while the TaskDetail is not actually rendered (e.g. `show_workroom` -> `invoke_show_section` -> `leave_run` race, or a window-level pop-out), the approval is hidden from the popup but not drawn in a pane, so the person never sees it. The source-scan test asserts only string presence. Fix: add a behavioural test, and ensure every path that clears the selection also clears `detail_open` (wire/agents.rs:272, 325, 557-560 do; verify 155, 380, 1636 against `leave_run`). This is a security-boundary file, so say so in the PR description and get the security review the playbook requires.
4. **Lost coverage with no equivalent**: route.rs (476 lines with unit tests) and route_tests.rs (verify-agents-route, 210 lines) are removed from the tree and validate.sh. The replacement activity timeline/changes ledger has tests in agents_workroom.rs, but nothing covers the "children / sub-agents of a run" path that the route showed (Stop on a parent also stops children, see launch::stop_on). Confirm the task detail still shows child runs or note the loss in the PR.

### NIT
5. Pre-existing and unchanged but now on the new screen: `on_stop` / `on_start` (wire/agents.rs about :184-215) call `launch::stop` / `launch::start` synchronously on the UI thread (these touch the harness host and may block), and the 250 ms `TimerMode::Repeated` TICK in wire/agents.rs:284 still polls. Not a regression, but the playbook's "no repeating timers / off the UI thread" rule applies to new work, so file a follow-up. The settled-workroom 0-redraw test (agents_tests.rs:451, 518) is good.
6. `show_workroom` result's `raised` is `raised_shell().is_ok()` and is not fed back as an error if it failed. It is reported honestly in the answer, so this is fine. Consider also reporting `narrowed_to` mismatch if a filter is ignored.
7. The `invoke_filter_mind` / `invoke_show_section` / `set_current_screen` / `navigate` sequence in control_workroom.rs:83-88 runs inside the handler. It is UI-only (no blocking), so OK. `workroom_minds_now()` (wire/agents.rs:637) takes the store/approval locks inside the control handler; it is read-only and short.
8. docs/app-control.md:211-214: the sentence now runs long in one paragraph; and the table row mixes `show_workroom` into the `show_agent` row, which is fine.
9. app.slint exports a long list of structs (WorkNavData...); only the wiring needed was added. Fine.

## Checklist (from the review brief)
- New action grading: `show_workroom` safe (correct). No pause/stop/take-over/approve action added. Existing `stop_agent` standard, `new_agent`/`hand_off` sensitive are unchanged.
- Answers read back: yes (page, narrowed_to, on_screen, raised).
- Blocking calls: none new; pre-existing stop/start on UI thread (nit 5).
- hold_windows: present before the first state change/raise, source-order test. Other raises (pop_out/wlrctl in wire/agents.rs:1479-1488) are unchanged and run in a thread.
- Take over: not implemented; nothing claims exclusivity.
- Approvals: unchanged UI; popup predicate changed (should-fix 3).
- route deletion: clean, no dead callers; coverage note in should-fix 4.
- Timers: none new. Previews: none drawn ("Desk preview unavailable"), so the 12 max / visible-only is vacuous.
- Vocabulary: matches except "Queued" absent (no such state in the store).
- Size: see should-fix 2.

## Files likely to conflict with other PRs
- crates/yantrik-ui/src/control.rs (two small hunks near :764 and :2098; other control PRs add actions at the same spots)
- crates/yantrik-ui/src/main.rs (one `mod control_workroom;` line next to control_agents/control_chat)
- crates/yantrik-ui/src/wire/mod.rs (one `pub mod agents_workroom;` line)
- crates/yantrik-ui-slint/ui/app.slint (export list line 42-43 and open-agent callback about :2911; high risk)
- crates/yantrik-design-tokens/slint/theme.slint (new `mind` colour and `fs-page`; PRs adding tokens nearby will conflict)
- crates/yantrik-ui-slint/ui/agents.slint (rewritten: guaranteed conflict with #583 if it touches it) and wire/agents.rs
- docs/app-control.md (lines about :209-213 and the grade table about :305)
- tests/ui-preview/{preview.slint, src/main.rs, validate.sh, agents_tests.rs, formations_tests.rs, mind_panel_tests.rs} (route removal edits shared lists)
- crates/yantrik-ui/src/control_approvals.rs (pane_agent and its test)

Verdict: MERGE AFTER FIXES
