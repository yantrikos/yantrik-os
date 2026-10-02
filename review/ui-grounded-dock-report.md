# Story 3.0: the grounded dock

Branch `ui/grounded-dock`. This replaces the 40px, full-width, per-window taskbar with the dock of spec §3.

## What a person sees
- A dock centred on the bottom edge, exactly as wide as its buttons (317px for five apps), flush with the screen, 16px round top corners, square bottom corners, `#101417` with a 1px neutral border on the top and sides. 48px tall; 40×40 buttons with 32px app tiles and 4px gaps.
- Order: Apps | pinned and running apps | divider | Yantrik Mind (the only teal tile).
- One button per app, not per window. Running: a static white 12×3px marker. In front: a neutral raised fill. Several windows: a small neutral numeral. A mind waiting on the person: one 6px amber dot on the mind button.
- Past min(880px, window − 32px) the middle pages with 32px "‹ +N" / "› +N" buttons. Icons never shrink. The wheel pages too. The page follows the app that comes to the front.
- Hover for 500ms: the name, 8px above the dock. 250ms on an app with several windows (or Tab focus): the 320px window list with a 56px row per window; a click on a row activates it, and a click on the button opens the list and does not choose for the person. Escape closes it.
- Apps a mind opens are not listed (`windows::shell_windows` already leaves them out). The Mind View window itself is listed, labelled by its own name.

## Exclusive zone
The shell is one fullscreen Slint toplevel, not a layer-shell panel, so the existing mechanism is labwc's `<margin>` in `config/labwc/rc.xml`. I changed `bottom="40"` to `bottom="48"` and `Theme.taskbar-height` to 48px, which every maximised-window calculation in the shell already reads. Result: windows end exactly at the dock's top edge across the whole screen width. I did not move to layer-shell (PR #564's spike); that is a separate change.

## Underneath
- `grounded_dock.slint` (new) replaces `taskbar.slint` (deleted). Reuses `AppTile` (the launcher's tiles), `YWheelArea`, `Icon`.
- `dock_model.rs` (new): pure grouping, ordering (pins first, then running apps in launch order, never focus order) and the page arithmetic, with tests.
- `wire/dock_bar.rs` (new): publishes buttons and window rows from the shell's `window-list` and the pinned list; called from the system poll and on every pin change.
- Tokens in `theme.slint`: `dock-*`, `mind-teal`, `needs-you`. `taskbar-height` 40 to 48.
- The old callbacks keep their names: `open-launcher`, `activate-window`, `open-companion`, `show-desktop`, `window-menu`, `restore-screen`. The taskbar menu actions are unchanged. The Show-desktop corner is now a 12px strip at the screen's bottom-right.
- The minimised-shell-screen entry survives as a dock button.

## Control surface
`describe shell` gains `dock`: `buttons[]` (`app`, `label`, `pinned`, `running`, `windows`, `focused`, `shown`), `page` (`first`, `shown`, `before`, `after`) and `needs_you`. No new action: activating a window goes through the existing `focus_window`, which already calls `card_watch::hold_windows`; the dock's own click path is the unchanged `taskbar_window_clicked` handler. Paging is a view detail with no meaning for a mind.

## Verified
- `verify-dock`, `verify-taskbar-menu`, `verify-apps-button`, `verify-cards-waiting`: all PASS (headless software renderer, built `--profile fast`).
- Screens, looked at: `target/ui-validation/dock-five.png` (5 apps, Notes in front, Terminal ×3), `dock-paged.png` / `dock-paged-2.png` (20 apps, 16 shown, "› +4", then "‹ +4"), `dock-list.png` (window list), `dock-needs-you.png` (amber dot).
- Idle: 0 requested redraws over 1.2s with the dock settled, and again with the window list open.
- `cargo test -p yantrik-ui-kit`: 7 passed, 0 failed.
- `cargo test -p yantrik-ui --bin yantrik-ui`: see the PR description for the count.

## Not verified, or left out
- On a real machine: the labwc margin, the compositor-driven window list, hover and keyboard behaviour with a real pointer. Nothing was run on a VM.
- The wheel pages when the pointer is over some buttons in the preview, but a sweep found it responding only at one position; I did not find out why. It may be a preview-harness artefact.
- The window list has no "Find a window" field (spec: above 12 windows) and no 64×40 previews. labwc does not tell this shell a window's workspace, so the row shows none rather than inventing one.
- `describe shell`'s `dock` is covered by compile and by `dock_model` tests, not by an end-to-end describe call.
- The "mind needs the person" dot uses the shell's pending-card count (`cards-pending`), which covers approvals and questions from all minds.
- Found, not fixed: the Notes app tile is yellow-orange, close to the amber the contract reserves for "needs you"; some other `AppColor` hues are teal, against "no teal tile except a mind's".
