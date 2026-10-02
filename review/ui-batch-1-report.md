# UI batch 1: integration report

Branch `ui/batch-1`, from `origin/main` (after #573, #586, #577, #578, #579, #580, #582). Merged with `--no-ff` in this order: #583, #585, #581, #584.

## Conflicts and resolutions

### #583 colour system
- `intent_lens.slint`: kept Chat v2's (#580) imports, `ChatHeader` and its copy ("Decline", "Approve once"). Put those two approval buttons on the pointer-only `YButton` (variant 1 and 0), so #583's guards hold: no focus, no Enter or Space, no AT-SPI default action. The kit source-scan tests now look for the new labels. The amber "needs you" outline on Approve once is replaced by the primary `YButton` look the spec asks for.
- `validate.sh`: kept both the `verify-chat` and `verify-approval-pointer-only` scenes.

### #585 grounded dock
- `app.slint`: kept main's export list, added `DockButton` and `DockWindow`.
- `taskbar.slint`: accepted the deletion. #583's one-line edit (active marker `accent` to `text-primary`) already holds in the dock, whose running marker is neutral white, so nothing was carried.

### #581 Alt+Tab
- `theme.slint`: kept main's OSD tokens (240x64 pill) and added the `sw-*` switcher tokens.
- `app.slint`: export list gains `SwitcherCell`; `mind-panel-covered` includes both `network-open` and `alt-tab-open`.
- `control.rs`: kept `control_levels_actions`, `control_network` and `control_switcher`.
- `tests/ui-preview/src/main.rs`, `validate.sh`: kept `verify-dock` and `verify-alt-tab`.
- `taskbar.slint`: stays deleted. `window_switcher.slint`: deleted by #581; #583's token swap has no counterpart in `alt_tab.slint` (no cyan or accent left).

### #584 Agents workroom
- `theme.slint`: kept all main tokens, added `wr-*`.
- `agents.slint`: took #584's, re-added Chat v2's `lens-cards`, `lens-strip`, `lens-live`, `lens-waiting`, `lens-identity` and the `WorkCardData` import, re-applied #583's working-dot colour (`accent` to `cyan`). #583's Close/Cancel `variant: 2` to `1` edits have no target (those buttons are gone).
- `wire/agents.rs`: took #584's workroom publishing; kept Chat v2's `publish_lens`, `work_card` and the call.
- `wire/mod.rs`: kept `lens_card_layout`, `lens_work`, `agents_workroom`.
- `control_approvals.rs`: #584's shared test `card()` helper, given the `decided_at` and `session` fields main added to `Card`.
- ui-preview `main.rs`, `validate.sh`: kept `verify-chat` and `verify-approval-pointer-only`. `verify-agents-route` and `route_tests.rs` go with #584's removal of the Overview route.

## Security properties checked by reading
- Approval buttons: `pointer-only: true` on both, no focus, key or accessible-action in the card, and the kit scan tests enforce it.
- #584's `pane_agent_of` popup rule is kept; `card_watch` and `hold_windows` code is untouched by any conflict.
- #582's launch routing is untouched (no conflict touched it).

## Notes for review
- #583 turned `Theme.accent` blue. #584's workroom still uses `Theme.accent` for links and two highlights (agents.slint); the spec wants teal for minds. Not changed here (one concern per PR).
- The `review/pr-58*` author reports from the source branches came in with the merges and should not land on main.

## Tests (run on the integrated branch; CARGO_BUILD_JOBS=1, CARGO_PROFILE_DEV_DEBUG=0, CARGO_INCREMENTAL=0, 12 GB swapfile)
- `cargo test -p yantrik-ui-kit`: 15 passed, 0 failed (includes `an_idle_window_stops_drawing::*` and the `consent_buttons_are_pointer_only` source scans). One test needed a fix after the merge: it looked for the mind-colour mark in `intent_lens.slint`, but Chat v2 moved the header to `chat_chrome.slint`.
- `cargo test -p yantrik-ui --bin yantrik-ui`: 1155 passed, 1 failed, 1 ignored. The failure is `harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`: the log has an extra leading `nvm` line, which this sandbox's shell profile prints. `harness_install.rs` is not changed by this batch; I did not run the test on plain main to confirm it fails there too.
- Not run: `tests/ui-preview` scenes (`validate.sh`, including `verify-chat`, `verify-approval-pointer-only`, `verify-dock`, `verify-alt-tab`) and `cargo test --workspace`. No rendered PNGs were looked at.
- Sandbox setup needed: apt packages libspeechd-dev, libclang-dev, libasound2-dev and other `-dev` libraries.
