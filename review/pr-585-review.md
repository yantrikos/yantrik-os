# Review of PR #585: the grounded dock (`ui/grounded-dock`)

Reviewed statically with `git diff origin/main...origin/ui/grounded-dock` and `git show` of full files. Nothing was built or run, and no PNGs were inspected. Line numbers are on the PR branch. `gd.slint` means `crates/yantrik-ui-slint/ui/components/grounded_dock.slint`.

## Summary

The PR is well scoped. It adds one new component, a pure `dock_model.rs` with five tests, and a `wire/dock_bar.rs` publisher. `taskbar.slint` is deleted, and a grep of the branch finds no remaining import or `include_str!` of it (only comments and design docs mention it). `rc.xml` and `Theme.taskbar-height` both move to 48px, and `app.slint` is a small wiring change.

What is right:
- Grounded and centred: the bar is `y = parent.height - 48`, centred, with a clipped rounded rectangle giving round top corners and square bottom corners (gd:331-345).
- Size and timers: the 4px padding and gaps, 40px buttons and 6px dot match the spec. Timers are single-shot (500ms, 250ms, 200ms) and each stops itself through its `running` binding.
- Mind apps: they stay off the dock, because `windows::shell_windows` already drops `mind_view::app_pids()`.
- Control surface and security: the one new surface is a read-only `dock` field in `describe`. No new action exists, so there is no grading or `hold_windows` gap. The existing `focus_window` path is unchanged.
- No new blocking work: `dock_bar::publish` runs on the UI thread inside the existing poll.

Problems:
- One likely build or CI regression: the `#[allow(dead_code)]` moves off `mod windows` (BLOCKER).
- One spec deviation: 32px icons where the spec says 24px.
- A hover-list bug.
- Partial wheel paging.
- Several untested claims.

## BLOCKER

### B1. `#[allow(dead_code)]` now guards the wrong module
**Where:** `crates/yantrik-ui/src/main.rs:140-144`
```
// NOTE: #[allow(dead_code)] required to avoid rustc 1.93.1 ICE in check_mod_deathness.
#[allow(dead_code)]
mod dock_model;
mod windows;
```
**Failure:** The PR inserted `mod dock_model;` between the attribute and `mod windows;`. The attribute, which the comment says is needed to avoid a compiler ICE, now covers `dock_model`, and `windows` loses it. Either `windows` warns or fails on dead code (fatal under `-D warnings`), or it reintroduces the `check_mod_deathness` ICE. This is a build or CI breaker that `cargo check` on the author's machine could hide.
**Fix:** Put `mod dock_model;` above the NOTE comment (and, if wanted, give it its own `#[allow(dead_code)]` for unused items). Keep the attribute directly on `mod windows;`. Then run `cargo build --workspace --locked`.

## SHOULD-FIX

### S1. Dock icons are 32px; the spec says 24px
**Where:** `crates/yantrik-design-tokens/slint/theme.slint` (`dock-icon: 32px;` with the comment "spec allows 24-36px"), used throughout gd (`size: Theme.dock-icon`). `review/ui-grounded-dock-report.md` repeats the claim ("32px app tiles").
**Failure:** Spec §3 says "40×40 buttons with 24px icons and 4px gaps". I found no "24-36" range in `design/` or `docs/`. The report states a deviation as if the spec allowed it. The mind tile is also 32px.
**Fix:** Set the token to 24px, or get the owner to amend the spec first and cite it. Either way, do not leave a comment claiming the spec allows it.

### S2. A title change kills the hover label and the hover list
**Where:** gd:~239-243 (`changed buttons => { over-id = ""; label-shown = false; }`) combined with `dock_bar::publish` replacing the model whenever any button field changes. `title` is one of those fields, and `dock_bar.rs:128` replaces the model on any change.
**Failure:** The system poll runs about every 3s. A browser tab or terminal title changes often, which replaces the model. The pointer is resting on a button, so `over-id` is cleared. The label disappears, and a hover-opened window list closes 200ms later (the close timer runs when `over-id == ""` and nothing is held). A person reading the list sees it vanish under a still pointer.
**Fix:** Clear hover state only when the button the pointer is on no longer exists, for example by checking `over-id` against the new `buttons`. Or keep the hover state keyed by `app-id` and re-resolve it after the update. Add a preview-test step that replaces the model while hovering.

### S3. The wheel pages only over the app buttons
**Where:** `YWheelArea` is used only inside `DockBtn` (gd:~160). The pager buttons (gd:~390, ~447), the divider, the gaps and the bar body have no wheel handling. The PR's own report says "a sweep found it responding only at one position; I did not find out why".
**Failure:** The spec says the scroll wheel works over the dock. The author is not sure it works at all, so this is shipped unverified.
**Fix:** Put one `YWheelArea` under the whole `bar`, or wrap the middle layout and both pagers with one. Add a `verify-dock` step that wheels at three positions, and don't merge with an unexplained "responds at one position".

### S4. The compositor margin and the token can drift, and nothing tests them
**Where:** `config/labwc/rc.xml:61` (`bottom="48"`) and `theme.slint` (`taskbar-height: 48px`). The coupling is a comment only.
**Failure:** The exact 48px exclusive zone across the whole width is what keeps maximised windows off the dock. Change one number and windows slide under the dock, or leave a gap, with no failing test. The wallpaper strip beside the dock is correct only while they match.
**Fix:** Add a unit test in `yantrik-ui` that parses `config/labwc/rc.xml`'s `<margin bottom=...>` and the `taskbar-height` token and asserts they are equal (the repo already has source-scan tests, so follow that pattern). Do this before #581 changes `rc.xml`.

### S5. The window list is not reachable from the keyboard
**Where:** gd:~620-660. Rows are a `TouchArea` plus `accessible-action-default`, with no focus scope or arrow keys.
**Failure:** The spec says the list appears "on keyboard focus". It does open, because `attended` includes keyboard focus. But a Tab-only user cannot move into it or choose a row, and Escape only closes it.
**Fix:** Handle Down/Up/Return in the button's `FocusScope` while the list is up, or make the rows focusable. If that is out of scope, say so in the PR description under "left out".

### S6. Spec items left out and not tracked
**Where:** `review/ui-grounded-dock-report.md` ("Not verified, or left out").
**Failure:**
- The window list has no 64×40 previews and no "Find a window" field above 12 windows. The report admits both.
- It also lacks the "Running" section in Apps. This is not in the PR, so please note it or file a follow-up.
- Alt+Tab reveal is only on the next poll, because `dock_bar::publish` computes the reveal from the 3s-poll `window_list`, not from the switcher (`dock_bar.rs:116-126`). There is no Alt+Tab code in this PR, so only the dock side exists. Please state that plainly, because the report says the page follows the app that comes to the front.
**Fix:** List these in the PR description as left out. Tag the reveal as poll-driven.

### S7. No test covers mind-apps-off-the-dock, `dock_bar`, or `describe`'s `dock`
**Where:** `crates/yantrik-ui/src/dock_model.rs` tests (5 of them) and the absence of any `#[cfg(test)]` in `wire/dock_bar.rs`.
**Failure:**
- The mind-app exclusion comes entirely from `windows::shell_windows`, with no test pinning it for the dock. A future change to the window list could put a mind's apps on the person's dock.
- `describe shell`'s `dock` is covered by "compile and `dock_model` tests", by the author's own admission.
- The Mind View label fallback (`dock_bar.rs:65-72`) is untested.
**Fix:** Add one `dock_bar` or `dock_model` test with a window list holding a mind-view window plus app windows. Add a describe-shape test for `dock` (keys `buttons`, `page`, `needs_you`). `dock_model` itself, covering grouping, launch order and paging, is good.

## NIT

- **N1. Hardcoded English where Tr exists.** gd:~395, ~452 ("Earlier apps", "Later apps"), gd:~513 (`mind-label: "Yantrik Mind"`) and `" windows"` (gd:~612). The old taskbar's `chat-with` (active harness name) and `companion-count` were dropped from `app.slint`, so the mind button no longer names the active mind. Spec wording for the label is "Yantrik Mind", which is fine, but go through `Tr`.
- **N2. "1 windows".** `list-count` is captured when the list opens (gd:~265). If windows close while it is open, the header can read "1 windows" and the panel height is stale. Bind it to the live count and use singular/plural.
- **N3. Literals in gd.** Magic px in `grounded_dock.slint`: `14px`/`7px` numeral plate, `font-size: 11px`, `28px`/`20px` label, `40px` list head, `12px` show-desktop strip, `8px`/`16px` divider inset, `16px` check, `1px`/`2px` offsets, `0.26`/`0.6` ratios. The playbook says no magic px. Add tokens (`dock-label-h`, `dock-list-head`, `dock-numeral`, `dock-desk-strip`) and use `fs-*` for the 11px text.
- **N4. Token naming.** The old dock tokens (`dock-height`, `dock-icon-size`, `dock-item-size`) sit beside the new `dock-icon`, `dock-button` and others in `theme.slint`. Confusing. Delete the old ones if unused, or note which are dead. The Theme comment about delays "written out in dock.slint" should say `grounded_dock.slint`.
- **N5. Orphaned comment in `app.slint:457-466`.** The new `dock-*` properties were inserted mid-comment, between the "taskbar menu / drawn at the end of this file" text and `taskbar-menu-open`. Move them above the comment.
- **N6. Launch order is "first seen".** `LaunchOrder::observe` takes newcomers in compositor order (front first), so two apps launched within one 3s poll can swap. Fine for stability, but the doc says "launch order". Say "first seen".
- **N7. Row click on the front window minimises it.** `activate-window(w.title)` goes to `taskbar_window_clicked`, which puts away the window in front. Clicking the current row (the one with the check) in the list therefore hides it. Probably unintended for a list; route rows to `switch_to` semantics.
- **N8. Colour conflicts noted by the author.** Notes' tile is amber-adjacent and some `AppColor` hues are teal. Both break "amber only for needs you / teal only for minds". Not this PR's concern, but file an issue.
- **N9. Report vague on tests.** `review/ui-grounded-dock-report.md` says "see the PR description for the count" for `cargo test -p yantrik-ui --bin yantrik-ui`. The playbook wants the exact command and pass count, so put it in the report.

## Merge-bar checklist (docs/agents/playbook.md)

| Check | Result |
|---|---|
| Grounded, centred, bottom-flush, round top corners | OK |
| 48px / 40px buttons / 4px gaps | OK |
| 24px icons | FAIL (32px, S1) |
| Exact 48px exclusive zone across the whole width | rc.xml and token OK; untested coupling (S4) |
| Paging with "› +N" and 32px pager buttons | OK |
| Wheel paging | partial (S3) |
| Stable order | OK (N6 aside) |
| Alt+Tab reveals page | poll-driven only (S6) |
| Window list 250ms / 320px / ≤400px / 56px rows | OK |
| Hover delay timers single-shot | OK; hover state wiped by title change (S2) |
| Mind apps off the person's dock | OK by `shell_windows`, untested (S7) |
| Amber dot for mind requests | OK (source is `cards_pending`, all minds) |
| `hold_windows` where a window is raised | No new raising control action; pointer path only. OK |
| Control fields graded honestly | `describe` field is read-only. OK; no action added, which is reasonable for a view-only detail |
| No blocking calls on the UI thread | OK |
| Tokens not literals | Partial (N3) |
| Reuse (AppTile, YWheelArea, Icon; taskbar.slint removed) | OK; no remaining importers, tests updated |
| One concern | OK |
| `dock_model.rs` tests | Good, five tests; add the S7 ones |
| True copy | Mostly; N1, N2 |
| Report present | Yes; N9 |

## Files likely to conflict with other PRs

- `config/labwc/rc.xml` (#581): the `<margin bottom="48">` and the comment block (line ~52-61). Merge whichever lands second by hand and re-check the number against `Theme.taskbar-height`.
- `crates/yantrik-ui-slint/ui/app.slint` (#581): the import line, the `export { ... DockButton, DockWindow ... }` list, the new `dock-*` properties near `taskbar-menu-open`, and the `Taskbar {` to `GroundedDock {` block at ~2830-2870.
- `crates/yantrik-ui-slint/ui/components/taskbar.slint` (#581, #583): this PR deletes the file, so any PR that edits it will have a modify/delete conflict. The #583 changes must be re-applied in `grounded_dock.slint` or dropped.
- `crates/yantrik-design-tokens/slint/theme.slint` (#583, #584): the +38 lines sit in the chrome-size block around lines 401-445. Expect textual conflicts if they add tokens at the same spot, and a clash if they also touch `taskbar-height`.
- Also: `crates/yantrik-ui/src/main.rs` (the `mod` list, see B1), `crates/yantrik-ui/src/wire/mod.rs`, `wire/pins.rs` (two visibility changes plus a `publish` call), `wire/system_poll.rs`, and the `tests/ui-preview` files (`main.rs`, `validate.sh`, `preview.slint`).

Verdict: MERGE AFTER FIXES
