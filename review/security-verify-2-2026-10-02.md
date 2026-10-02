# Security verification 2: PR #582 and PR #583 (2 Oct 2026)

Method: original review compared with the branch as it is now (`git diff origin/main...origin/<branch>`), each finding read in the code. The fix reports were not trusted. No code was changed.
Tests: `cargo test -p yantrik-ui-kit --lib` was run on the #583 branch: 15 passed, 0 failed. `cargo test -p yantrik-companion-tools --lib` could not be built in this sandbox (missing `speech-dispatcher` headers, a system dependency). The `yantrik-ui` tests and the Slint preview were not built (heavy). Everything on #582 below is code reading; its tests were not run here.

---
## PR #582 `fix/mind-apps-in-mind-view`

Merges cleanly into current main (`git merge-tree`, no conflicts). The branch is not a descendant of the latest main (main has moved since).

| Finding | Status | Evidence |
|---|---|---|
| B1 DISPLAY inherited by a mind's app | **VERIFIED FIXED** | `wire/dock.rs:1164-1196` `launch_command` applies `seat.display()` after `session_env()`. `MindDisplay::mind_view` clears DISPLAY, WAYLAND_DISPLAY, WAYLAND_SOCKET and XAUTHORITY (`yantrik-companion-tools/src/browser.rs:29,43`) and then sets only Mind View's own. Without Xwayland, `Seat::env` pins GDK_BACKEND and QT_QPA_PLATFORM to wayland only (`mind_view.rs:111-128`). Blender in Mind View is refused when `seat.x11` is None (`dock.rs:1128-1141`). Tests: `dock.rs:1805`, `mind_view.rs:902,918`. |
| B2 "already running" spawns on the person's desktop | **VERIFIED FIXED** | `mind_view.rs:204-226` `route` returns `spawn: false`. `dock.rs:1112-1116` returns before any process is started. `control.rs:1118` `already_open_for_mind` adds an honest "not launched again, not raised" note. Test `dock.rs:1830`. |
| B3 other mind launch paths | **VERIFIED FIXED** | `desktop.rs:49-52`: `open_url` no longer runs `xdg-open`; it calls `browser::open_url_for_mind`, which navigates the companion's own browser or launches it through `display_for_mind`. `display_for_mind` (`mind_view.rs:570`) returns Err when Mind View is down and no longer falls back to the person's display. The browser only attaches to a non-foreign CDP browser (`browser.rs:533-553`). `LaunchBrowser` applies `MindDisplay`, and headless clears the person's variables (`browser.rs:764-779`). The `xdg-open` in `open_url.rs:31` and `lens.rs:186,252` is person-only: nothing in `control*.rs` reaches `lens_result_selected`. |
| S1 `lists_app` substring match | **VERIFIED FIXED** | `mind_view.rs:698-721`: matches the declared app id or exact title via `windows::toplevel_entry`, never words inside a title, and excludes the agent viewer (`foot` titled `... terminal`). |
| S2 error text asserted "nothing opened on the desktop" | **VERIFIED FIXED** | `mind_landing.rs:124-160`. The desktop sentence appears only on `Miss::Refused`. A timeout says "may still be starting" and does not name the desktop. A refusal is not shown as "exited with" (`REFUSED` prefix, `mind_landing.rs:20-22,91-98`). |
| S3 singleton browsers | **VERIFIED FIXED** (Chromium and Firefox only) | `dock.rs:1198-1226` `singleton_isolation`: separate `--user-data-dir` / `--no-remote --profile` for a mind. Other single-instance apps are not covered (see new issue N1). |
| S4 spoof / wipe of the viewer log | **VERIFIED FIXED** | `mind_agent_terminal.rs:60-120`: `Cleaner` drops C0 controls, ESC two-byte, OSC/DCS and all CSI except SGR. The command line is caret-escaped (per the fixes report; tests present). |
| S5 `files_open` answers "opened" falsely | **VERIFIED FIXED** | `control_files.rs:133-178`: `.defers()`; for a mind it waits for the window through `mind_landing::probe_for/answer` on `answer_later`, and a refusal is returned as an error. |
| S6 Mind View failure permanent | **VERIFIED FIXED** | `mind_view.rs:271-283`: `unavailable` is retried after `RETRY_AFTER`. |
| S7 `agent_run` blocked / raised over a card | **VERIFIED FIXED** | `control_agent_terminal.rs:~536`: `show_later` runs on its own worker. `mind_agent_terminal.rs:302` calls `hold_windows("agent_run")` before `ensure()`; a source-scan test pins the order (`:571-573`). |
| S8 unbounded log | **PARTIAL** | The per-command cap, a single append handle and 0600 are claimed in the report. The log is not deleted when an agent ends (acknowledged in the report), so output, secrets included, stays on the runtime tmpfs until the next oversized `begin()`. Low. |
| S9 id collisions / stale state | **VERIFIED FIXED** (per code and tests listed) | Hash suffix for ids that sanitise alike, "(not started)" on failure, `abandon` on a refused start (`control_agent_terminal.rs:516-524`). |
| S10 tests | **PARTIAL** | The B1, B2, S1, S3, S4 and S6 behaviours now have behavioural tests. Still source scans for the `answer_later` wiring. The held-act-replay `requester_now` case (review path 21) has no test; it is low risk because `open_app` is graded standard, so no replay is involved. |
| N1-N3 | Fixed. N4, N7, N8: not fixed, not security relevant. N5, N6: documented in `docs/app-control.md`. |

Core requirement check:
- No DISPLAY, WAYLAND_DISPLAY, WAYLAND_SOCKET or XAUTHORITY of the person's session reaches a mind launch: **met** (the person's own setting `minds_open_in_mind_view` off sends a mind to the desktop by design; it is pointer-only, `control_approvals.rs:1476`).
- No "already running" spawn on the desktop: **met** for apps the shell registry knows about.
- Every mind launch path goes through Mind View: **met**. Catalogue, route apps, Browser, Blender, `files_open`, `open_with`, the viewer, `open_url` and the companion browser all go through `spawn_launch`/`launch_command`/`display_for_mind`.
- `open_app` answers truthfully: **met**. It waits off the UI thread and reports the place, a refusal, a failure or a timeout, and does not assert where a window went.

### New issues
- **N1 (medium, follow-up, not a blocker):** `DBUS_SESSION_BUS_ADDRESS` and the XDG vars stay in a mind's launch environment. A D-Bus-activatable single-instance GTK/Qt app (Nautilus, gedit, Evince style) that the person already has open, and that the shell registry does not know about (so `is_running` is false), could hand the request to the person's process over the session bus and draw on the person's desktop. Only Chromium and Firefox are isolated. Suggest a private D-Bus (`dbus-run-session`) or an env scrub for Mind View launches, plus `--gapplication-*` handling.
- **N2 (low):** `cdp_port_is_the_persons` fails open ("taken as ours") when `/proc` cmdlines cannot be read (`browser.rs:531-541`), so in another pid namespace the companion could attach to the person's debugging browser on 9222.
- **N3 (low):** The `Launch::Blender` pre-check in `dock.rs:~925` still tests the person's `$DISPLAY`. For a mind it only decides refuse-or-continue, and the real gate is `seat.x11`; but a person with no DISPLAY cannot have a mind open Blender in Mind View.
- **N4 (info):** `agent_run` is a shell, graded sensitive: a mind's command that sets WAYLAND_DISPLAY itself is outside this PR's launcher guarantees. Unchanged and pre-existing.
- **N5 (info):** The yantrik-ui run in the fixes report has 1 failing test (`harness_install::...`) attributed to a sandbox shell profile; not confirmed on main.

**Verdict: MERGE.** All three BLOCKERs (B1, B2, B3) are fixed in code and the requirement holds. Ticket N1 (D-Bus activation) as a follow-up.

---
## PR #583 `ui/colour-system`

The branch is not rebased on current main: merging into main conflicts in `tests/ui-preview/validate.sh` (mechanical, append-only scene lines). Main has also rewritten `intent_lens.slint` (chat v2, #580) but the approval card region merged without conflict. After rebase, re-run the kit scan tests, because they read `intent_lens.slint`.

| Finding | Status | Evidence |
|---|---|---|
| 1 BLOCKER: AT-SPI default action pressed Allow once | **VERIFIED FIXED** | `y_button.slint:41-42`: `accessible-enabled` and `accessible-action-default` are both gated on `!root.pointer-only`. The FocusScope is `enabled: ... && !root.pointer-only` (`:67`), and the TouchArea does not focus a pointer-only button (`:84`). `clicked()` has exactly 3 callers (default action, key, pointer), pinned by a test. |
| 2 Test guarded a hand-built pair | **VERIFIED FIXED** | New `tests/ui-preview/src/approval_tests.rs:run_pointer_only` drives the real IntentLens card (Tab, Backtab, Enter, Return, Space x12, then a click on each) and is registered in `validate.sh`. Kit source scan `the_real_approval_card_keeps_pointer_only_on_both_buttons` (`yantrik-ui-kit/src/lib.rs`) fails if either button loses `pointer-only: true` or the card gains `forward-focus`, `init =>`, `.focus()`, `key-pressed` or `accessible-action`. The rendered test was not run here (heavy). The scan was run: pass. |
| 3 Settings swatch / names | **VERIFIED FIXED** | Swatches read `AccentPreset.swatch-*` tokens; "Soft blue"; the saved id stays "cyan"; test `the_accent_swatches_come_from_the_presets_not_from_hex` passes. |
| 4 Teal-for-minds | **PARTIAL** | The thinking dot (`agents.slint`) and the companion mark (`intent_lens.slint`) are `Theme.cyan`, tested. Other `Theme.accent` uses on mind surfaces were judged chrome by the author; not audited beyond that. Not security relevant. |
| 5 Email flagged state | **VERIFIED FIXED** | `variant: is-flagged ? 0 : 1`; test passes. |
| 6 Blast radius / split | **NOT FIXED** | Left to the author (recorded as open). Process only. |
| 7 Unverified claims | **PARTIAL** | The report now lists what was run. Preview scenes were reported as run, but only the kit tests were confirmed here. `preview.slint` variant 2 call sites moved to 1 (test `no_one_asks_for_the_retired_button_kind` passes). |
| 8 Behind main | **NOT FIXED** | Not a descendant of current main; conflict in `validate.sh` (see above). The report's "rebased" held for the main of the time, not the current one. |
| 9 AgentProposalCard Apply | **VERIFIED FIXED** | `agent_proposal.slint:~177` `pointer-only: true`; test passes. |
| 10 dead variant 2 | **VERIFIED FIXED** | No `variant == 2` in `y_button.slint`. |
| 11 | Documented in the `pointer-only` comment. |
| 16 Empty Trash | **VERIFIED FIXED** | Destructive variant; test passes. |
| 12-15, 17 | Not changed; spec calls or pre-existing. |

Core requirement check:
- "Allow once"/"Approve once" cannot be pressed by the AT-SPI default action, Tab, Enter, Return, Space or autofocus: **met** (`y_button.slint:41-42,67,84`; `intent_lens.slint:687-701`: no focus, key or accessible-action code on the card). A pointer click (including synthetic pointer events) still answers, exactly as before; that is the spec.
- The test guards the REAL card: **met** (rendered scene `verify-approval-pointer-only` plus the source scan on `intent_lens.slint`; the scan passes). The rendered scene was not run in this sandbox.
- card_watch's press guard is intact: **met**. `control_approvals.rs:1323` and `:1348` still call `card_watch::just_raised()` first, and no Rust file under `crates/yantrik-ui` differs from main on this branch for these handlers (the `crates/yantrik-ui` diff in `origin/main` vs HEAD is main's newer work, not this PR's).

### New issues
- **N1 (low):** Not rebased on current main; `validate.sh` conflicts. Rebase, then re-run the kit scan tests and `verify-approval-pointer-only`, since main rewrote the file that holds the card.
- **N2 (low):** `accessible-enabled: false` hides the Allow once and Deny buttons from a screen reader's actionable list. This is consistent with the spec, but it means a screen-reader user cannot answer an approval at all. Decide and document it.
- No new keyboard, focus or default-action paths found.

**Verdict: MERGE** after a mechanical rebase onto current main and a re-run of the kit tests and the approval preview scene.
