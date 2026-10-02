# Review: PR #581, Alt+Tab switcher (spec section 4)

Branch `origin/ui/alt-tab-switcher` vs `origin/main` (merge-base diff). Read-only review; nothing was built or run, findings are from reading code.

## Summary
Replaces labwc's Alt+Tab OSD with a shell-drawn card (`alt_tab.slint`), a pure model (`alt_tab.rs`, well tested), and four `shell` actions (`control_switcher.rs`). The model, paging, status line, first-Tab-selects-previous, frozen order, hold_windows on open/commit, and honest grades are good. The PR is honest in its comments that it cannot commit on Alt release and has no previews. But it ships a regression on the core gesture (Alt-release), has no fallback in rc.xml, adds blocking process calls on the UI thread, collapses same-titled windows, and omits Mind View scope. Several spec items are absent.

## BLOCKER

1. **Alt release does not switch; rc.xml removes labwc's working path.** `config/labwc/rc.xml:106-111` (A-Tab / A-S-Tab now only `Execute yos act shell open_switcher`); `control_switcher.rs:12-18`. Before: hold Alt, Tab, release = switch (labwc OSD). After: release does nothing, the card stays up until Enter/click/Escape. Spec "Keys": "Releasing Alt or pressing Return switches". The classic Alt+Tab muscle-memory gesture leaves a card on screen over the desktop and switches nothing. Failure: person taps Alt+Tab, releases, the card persists, keyboard focus is in the shell. Fix: either implement release (labwc `<keybind key="A-Tab" onRelease>` cannot see Alt; use a layer-shell/keyboard-interactive surface per spike #564, or have the shell see Alt key-release while it has focus, which Slint's FocusScope does deliver once the card is focused: handle `key-released` for Key.Alt/Meta and commit), or do not merge until it does. At minimum the shell card should handle Alt key-release since the card holds focus after the first open.

2. **Alt+Tab has no fallback and dies silently.** `rc.xml:106-111`. If the shell is hung/crashed, `yos` fails, or `hold_windows` refuses because an approval card is waiting (`card_watch.rs:117-128`: a keybinding through `yos` is not "granted"), Alt+Tab does nothing at all and the refusal text goes to nobody. Previously labwc cycled windows regardless. Failure: any pending approval card makes Alt+Tab dead. Fix: keep a compositor fallback (e.g. a second path / shell-side watchdog) or have the keybind show the refusal; at minimum document and test the card-waiting case.

3. **Blocking process calls on the UI thread inside a control handler.** `control_switcher.rs:121-122` (`front_title().or_else(windows::in_front)`, `gather()` -> `windows::shell_windows()` -> `compositor_snapshot()`, i.e. `wlrctl toplevel list`), then `:125` `raise_shell()` (restore + focus, each bounded at 1.2 s per `windows.rs:335-343`). Playbook: "No blocking ... process ... call runs on the UI thread or inside a control-action handler. Use the runtime's `answer_later` with a worker". `control_overlays.rs:107` has the same `raise_shell` precedent, but this adds two or three more waits (list, front lookup, restore, focus) per Alt+Tab press, each press being a new `yos` invocation, so a slow wlrctl freezes the desktop on every key press. `commit` correctly uses a thread (`:231`); `open` should too: gather/origin/raise on a worker, publish via `invoke_from_event_loop`, finish with `answer_later`.

## SHOULD-FIX

4. **Windows with identical titles collapse to one cell.** `alt_tab.rs:24-38` (comment at :22 says it is deliberate, "the title is the only handle"). Two terminals/Chromium windows titled the same: one is unreachable from the switcher, and `present(&title)` by title is ambiguous. Spec: windows are individual. Fix: key cells by something unique (wlrctl does not give ids; at minimum show both with a disambiguating suffix and select by app_id+index), or note it as a limitation in the PR description and in `describe`.

5. **Closed windows stay listed.** `alt_tab.rs:40-43`. Spec: "A window that closes is removed". Choosing one logs only a warning (`control_switcher.rs:233`), so the switcher is dismissed and nothing happens. Fix: drop or grey on commit failure and keep the card open, or re-poll off-thread.

6. **Cancel brings a window forward with no hold_windows and is graded safe.** `control_switcher.rs:175-191` (`bring_forward` of origin), `:334` `.risk("safe")`, test at `:375` asserts cancel never holds. Scenario: a mind (or anything) opens the switcher while no card waits, a card then appears, `switcher_cancel` raises the previous app window over the waiting card. Playbook: "An action that brings a window ... over the shell must call hold_windows first". Fix: hold_windows only when origin is an app window and then, if refused, put the switcher away without restoring (as `commit` does). Update the test.

7. **Mind View scope not implemented.** Spec: from the desktop an open Mind View is one window; inside Mind View, Alt+Tab switches that desk's windows with scope label ("Hermes's desk") and a way back; switching is not Take over. Nothing in the diff touches `mind_view` scope; `gather()` uses `shell_windows()` only. Either implement or list as explicitly left out in the PR body.

8. **Missing spec items, undeclared.** Footer shows only app name (`alt_tab.slint:78-89`): no class-if-different, no workspace; plate (`:229-249`) has no "· Workspace N"; windows are not "across all workspaces" by evidence; cells are not the real aspect ratio of a source (previews unavailable, honest, but spec-visible). No `Alt+Shift+Tab`-in-card handling for Backtab beyond key; there is no "visible way back" because Mind View scope is absent. List these in the PR description.

9. **Tokens, not literals.** `alt_tab.slint`: `224px`, `41px`, `40px` (icon, plate), `720px`, `1040px`, `560px/760px`, `12px`, `8px`, `16px`, `32px/24px`, `1.4px/1.5px` thickness, `font-weight: 600` throughout (e.g. :28-32, :123-129, :158-232). Playbook: "no hex colours or magic px in components". Add tokens (switcher cell/card/plate sizes, breakpoints) to `theme.slint`. Colours do use Theme tokens, and radii/type use Theme (verified `r-xl`, `r-lg`, `r-md`, `sp-2`, `fs-caption`, `fs-body`, `fs-micro`, `text-dim`, `border-subtle` exist).

10. **Reuse.** The page arrows are hand-rolled Rectangles with literal "‹" "›" (`alt_tab.slint:206-225`) instead of a kit button/Icon. No focus ring. Check `yantrik-ui-kit` for an icon button.

11. **Test gaps.** No test for the dedup case, closed-window-on-commit, `describe` shape, or card-waiting + `open_switcher` refusal path at runtime (only source-scan). `labwc_alt_tab_opens_the_shells_switcher` locks in the no-fallback binding (finding 2). No `verify-idle` style check that the card draws 0 frames once settled (playbook idle-CPU); PR adds validate.sh scenes but I did not see an idle assertion.

## NIT

12. `control_switcher.rs:106-109`: error says "`<screen>` is not the desktop", but the guard is `bar_is_drawn` (refuses only boot/onboarding/lock/login; works over Files/Settings). Message is misleading; word it "has no status bar yet" or similar. Also `alt_tab.slint` comment says "over every screen" - consistent with that.
13. `control_switcher.rs:312`: `switcher_move` hard-codes 4 columns while the card may draw 2/3, so `up/down` via control differ from keyboard on narrow screens; accept `columns` or read from the UI.
14. `alt_tab.rs:22` doc comment: "windows it has not seen take focus follow..." is garbled.
15. `toplevel_watch::recency()` returns an empty list when the stream is off (`LIVE`), so order silently degrades to listed order; say so in `describe` (`order_source`).
16. `describe` publishes `"previews": "unavailable"` (good, honest). `docs/app-control.md` row: `switcher_commit` documented "standard" while code relies on the unstated default (`risk()` omitted, `control_switcher.rs:321`); state `.risk("standard")` explicitly so it cannot drift.

## Spec conformance checklist
- Card size/opaque/no dim/16px padding/12px gaps/cell sizes/selection 2px white + raised fill (no teal): met (`alt_tab.slint:34-37`, `157-165`).
- Plate 40px/720px: met. "Preview unavailable": met.
- Order frozen at open, first Tab previous: met (`alt_tab.rs:86-93`). Recency uses focus stream only; unfocused windows after.
- Keys: Tab/Backtab/arrows/PgUp/PgDn/Return/Escape: met. Alt release: NOT met (1).
- No window activation for previews: met (no previews).
- Pointer position not overriding selection: met (move events only, `alt_tab.slint:92-96`, tested).
- Paging 8/page, status text, wheel, arrows: met (`alt_tab.rs:182-191`). Columns 3/2 below 760/560: met.
- Scope labels / Mind View: NOT met (7). Class/workspace in footer: NOT met (8).

## Control surface (abuse paths)
- `open_switcher`, `switcher_move`, `switcher_cancel` graded safe; `switcher_commit` default (standard) plus `defers`. hold_windows present in `open` and `commit` (title != desktop). `cancel` is the gap (6).
- `open_switcher` as safe lets a mind repeatedly raise the shell (focus theft) and step the selection; same as the bar panels precedent, acceptable but worth stating. Commit is the only action that moves another window, and it is at least standard and held.
- `present(title)` takes a title that came from the compositor listing, not from the caller, so no caller-chosen window name (verified: `commit` uses only `s.selected()`).
- Synthetic input: the card's FocusScope accepts keys from any source; Enter commits a standard-graded action via a keyboard path a mind with input injection would already have, no new escalation.

## Files likely to conflict with other PRs
- `crates/yantrik-ui-slint/ui/app.slint`: hunks near lines 60 (imports), 76 (export list), 301-303 (`mind-panel-covered`), 447-460 (switcher props), 1609-1610 (desktop bindings removed), 3354-3370 (AltTab instance), 3389-3393 (`changed alt-tab-open`). High conflict risk: shared export line 76 and the overlay stack at the end.
- `crates/yantrik-ui-slint/ui/desktop.slint`: lines 37, 149, 899-908 (WindowSwitcher removed).
- `crates/yantrik-ui-slint/ui/components/taskbar.slint`: line 27 (import). `window_switcher.slint` deleted, `window_item.slint` added.
- `crates/yantrik-ui/src/control.rs`: two 2-line insertions near 963 (describe) and 2082 (actions); high risk of textual conflict with other surface PRs.
- `crates/yantrik-ui/src/main.rs` (mod list ~51), `wire/mod.rs` (~101), `toplevel_watch.rs` (FocusLog struct ~88-160, tests ~419), `config/labwc/rc.xml` (~96-110), `docs/app-control.md` (~310, table rows), `tests/ui-preview/src/main.rs` (~22, ~83), `tests/ui-preview/validate.sh` (~34).
- New files (no conflict): `alt_tab.rs`, `control_switcher.rs`, `alt_tab.slint`, `window_item.slint`, `alt_tab_tests.rs`.

Verdict: DO NOT MERGE
