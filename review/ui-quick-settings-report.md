# Quick Settings v2 and the power popover — report

Branch `ui/quick-settings-v2`, from origin/main (rebased onto #592, the render-01 top bar).
Security review: power actions (`power_sleep`, `power_off`) and the power popover's confirmation.

Plan PR 2.2 and the popover half of PR 6.3.

## What a person sees
- **Quick Settings** hangs from the settings mark at the right of the bar, 360px wide, on the
  charcoal #151A1E panel. Tiles are #1E252B with a 1px #2C343B border; a lit tile is the soft-blue
  accent with dark text.
  - *Minds*: **Mind mode** (split: the body steps Ask ↔ Plan, the chevron opens the full mode
    menu), **Private**, **Mind View** (full width, says where a mind's apps open).
  - *Machine*: **Wi-Fi** (split: radio switch / network list) or **Wired** on a VM, **Do Not
    Disturb**, **Dark style**, **Power mode**, **Bluetooth** only when an adapter is reported.
  - Volume and brightness sliders; the speaker icon mutes. Brightness only with a backlight.
  - Footer: battery % (only with a battery), Screenshot, Settings, Lock, Power.
- **Power popover** (from the footer's Power, Super+Escape or the palette): Lock, Log out,
  Suspend, Hibernate (only if logind says `CanHibernate` is "yes"), Restart, Shut down.
  Lock, Suspend and Hibernate act at once. Log out, Restart and Shut down show "Restart now?",
  a **red** button and "Expires in 10 s · nothing happens if it expires". With minds working it
  says "2 minds are working. Shut down stops them…" and offers **Finish first** / **Stop them and
  shut down**. Focus starts on the safe button, so Enter on arrival cancels.
- Keyboard: arrows move between controls, Space or Enter presses, Esc closes. A split tile's
  chevron is a separate stop (Tab). A slider keeps the arrows for its own level; Tab leaves it.

## Hidden when absent (nothing is invented)
| Machine | Not drawn |
|---|---|
| VM (wired, no battery, no backlight, no power-profiles-daemon) | battery text, brightness, Power mode, Wi-Fi (shows Wired instead) |
| No audio server | volume row |
| No Bluetooth adapter | Bluetooth tile (`bluetooth-present` is false and nothing sets it yet: PR 2.3 wires it) |
| logind says no | Hibernate |
| No minds working | the "N minds are working" sentence |

## What changed underneath
- `quick_settings.slint` rewritten as a `YPopover`; new `quick_settings_parts.slint` (a slot
  wrapper per kit control so the arrow keys have a cursor). `power_menu.slint` rewritten as a
  popover. `shell_overlays.slint` and `app.slint` wire them to the app's existing state and
  callbacks (mode, Private, Mind View, DND, dark, power profile, volume, brightness, lock).
- Kit (additive): `YToggleTile.right-opens-details` and `focused`; `focus-body()`, `YSlider`
  and `YIndicator` `focused` plus a focus function; `YButton.focus-button()`; `YPopover`
  `key-unhandled` hook. Four icons (`contrast`, `bluetooth`, `screenshot`, `logout`).
- **`YPopover` is now `Theme.panel-solid` (#151A1E), not `bg-elevated` (#2C3545).** This changes
  the network and battery popovers too, to the colour the tile tokens say they sit on.
- `yantrik-os/src/login1.rs`: `CanHibernate`, 2 s bus timeout, off the UI thread.
- `control_power.rs`, `wire/power.rs`, `wire/screenshot.rs`, `mind_panel.rs` (`Working.minds`,
  distinct minds at work).

## Control surface
- New: `power_sleep how=suspend|hibernate` — **sensitive**, deferred. `power_off
  how=logout|restart|shutdown [even_if_minds_working]` — **dangerous**, deferred; refuses
  while minds are working unless told to stop them, and says how many. Both answer "logind
  accepted it", never "it is off". Hibernate is refused where logind does not offer it.
- `describe shell` › `power`: `hibernate_offered`, `minds_working`.
- Existing and reused: `lock` (safe), `set_do_not_disturb`, `set_power_profile`, `set_volume`,
  `set_brightness`, `set_wifi`, `set_mind_mode` (tightens only), `open_/close_quick_settings`,
  `open_/close_power_menu` (hold_windows, unchanged).
- Not on the control surface, by design: Private and Mind View (pointer-only choices), Dark style.
- Neither new action raises a window, so neither calls `hold_windows`; a source test says so.

## Verified
- `cargo test -p yantrik-ui-kit`: 15 passed.
- `cargo test -p yantrik-ui --bin yantrik-ui`: 1187 passed, 1 failed (`harness_install`, below,
  environmental), 1 ignored. Includes the new `control_power` tests (5), the `mind_panel` minds
  count, and the rewritten `wire::network` tile test.
- `cargo test -p yantrik-os login1`: 1 passed.
- `verify-quick-settings` (new, replaces `verify-qs-levels`): PASS. It checks the VM and laptop
  shapes (pixels: off tile #1E252B, lit tile = accent, panel #151A1E), 360px at 800/1280/1920,
  tile body vs chevron as separate targets, arrows/Space/Enter/Tab/Esc, the speaker mute, the
  footer, 0 redraws once settled, the power list, Lock acting at once, Enter-on-arrival cancels,
  Tab+Enter on the red button performs once, 0 and 2 minds working, the countdown ticking, and an
  expired confirmation performing nothing.
- Also PASS on the rebased tree: `verify-network`, `verify-kit-controls`, `verify-colour-system`,
  `verify-battery`, `verify-mind-panel`, `verify-cards-waiting`.
- Screenshots: `review/ui-quick-settings-shots/`.

## Not verified / found, not fixed
- **Fail identically on origin/main** (built and run from a clean worktree): `verify-bar-overlays`
  (lock-screen "no panel" drift check, 81106 px), `verify-osd` (45% fill), `verify-approval-card`
  (session row). Not touched here.
- `harness_install::a_coloured_installer_reaches_the_row_as_plain_text` fails in this sandbox
  (a login shell prints "nvm"); the file is untouched.
- Not run: the real shell on a machine. `systemctl`/`loginctl` calls, `CanHibernate`, grim and
  the Screenshot button are not exercised by the preview. `loginctl terminate-session` needs
  `XDG_SESSION_ID`; not tried on the labwc image.
- Not run: `cargo build --workspace --locked`, `cargo test --workspace`, the selftests.
- The volume row has no chevron: there is no outputs list to open (no sound popover or Settings
  category exists), and a chevron to nowhere is worse than none. Bluetooth has the same status.
- The bar no longer has a power mark (#592), so the popover is right-anchored like Quick Settings
  instead of hanging under a power icon. The launcher footer's Restart/Shut down still act
  at once (`app_grid.slint` calls `power-action`); routing them through the popover is a follow-up.
- Log out also confirms (the brief named only Restart and Shut down): it ends every program too.
- "Mind mode" body toggles Ask ↔ Plan only. Auto and the bypasses stay behind the menu's own
  confirmations.
- Light-mode values for the new surfaces were not looked at.
