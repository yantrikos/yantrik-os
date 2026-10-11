# Shell UI/UX overhaul — audit, benchmark and plan (2026-10-01)

**The feedback.** The feedback was that the desktop is "really really bad" next to Omarchy and the other mainstream desktops. It named three missing basics: a volume control, a Wi‑Fi/network indicator, and a battery indicator when the machine has a battery. It also said "a whole bunch" more was missing.

**The verdict after reading the source and rendering the shell.** The feedback is right, though the reason is not mainly the look:

- **The polish is already decent.**
  - The desktop home, the launcher, Files and Settings look decent in the renders below.
  - The tokens are coherent.
  - The motion discipline is better than most desktops'.
- **What fails is the system furniture**, the part a person touches in the first five minutes:
  - There is no volume control in the bar.
  - The network mark is a 13px dot-sized glyph that opens a three-control card. That card's Wi‑Fi tile turns the radio off.
  - The volume slider writes to ALSA while the machine runs PipeWire.
  - The bar's power and network clicks draw nothing on most screens.
  - There are no OSDs.
  - Half the shortcuts the code advertises are never bound.
- **Underneath sits one structural cause.** The bar and taskbar are an ordinary fullscreen window, not panels. Many of the defects follow from that.

Everything below is cited to `file:line` on the working tree at `fix/one-bar-per-screen`. That tree includes the one-bar change to Files and Settings. It was uncommitted when the audit started and was committed during it, as `114d6c4`. No VM was touched.

The repo's `.gitignore:63` ignores `*.png`. The screenshots therefore need `git add -f` if they are to be kept with this document.

---

## 0. How this was looked at

| Source | What |
|---|---|
| Source read | `crates/yantrik-ui-slint/ui/**`, `crates/yantrik-ui-kit/slint/**`, `crates/yantrik-design-tokens/slint/theme.slint`, `crates/yantrik-ui/src/{wire,control*.rs,windows.rs}`, `crates/yantrik-os/src/{battery,keybinds,observer}.rs`, `config/labwc/*`, `deploy/yantrik-os/{build-debian-iso.sh,cloud-init/user-data.yaml,yantrik-update,yantrik-session}` |
| Renders `h-*.png` | The repo's harness: `tests/ui-preview`. It was built in WSL with `--offline` and `CARGO_TARGET_DIR=/home/yantrik/target-yantrik`, profile `fast`, which took 10m53s. These are the scenes from `validate.sh`. |
| Renders `a-*.png` | The harness has no scene for Quick Settings, the power menu, the lock screen, the window switcher, the clipboard, the command palette or the notification centre. So a **scratch harness outside the repo** composed the *production* components the way `app.slint` composes them, with fixture data: `DesktopScreen` with its overlays open, plus `StatusBar` and `Taskbar`. Nothing in the repo was changed for this. Phase 0 below proposes adding these scenes to `tests/ui-preview` properly. |
| Caveats | All renders use Slint's software renderer with fixture data, so they show layout and paint, not live behaviour. Behaviour claims come from source and are marked *verify live* where only a running session can confirm them. |

Screenshots: `design/ui-audit-2026-10-01/`.

| File | Shows |
|---|---|
| `h-desktop.png`, `h-desktop-light.png`, `h-desktop-compact.png` | Everyday desktop, with the bar and taskbar |
| `h-agent.png` | The *other* desktop home (agent-mode hero), with a different layout |
| `h-mind-panel*.png` | Mind panel open, strip and light variants |
| `h-approval-card.png` | Approval card in the Lens |
| `h-apps-button*.png`, `h-taskbar-menu*.png`, `h-lens-answers*.png` | Launcher, the taskbar's window menu, Lens turns |
| `h-files.png`, `h-settings.png`, `h-notes.png` | App interiors. The scene has no shell bars. |
| `a-shell.png`, `a-shell-laptop.png`, `a-shell-compact.png` | Shell composed with fixture *desktop* and *laptop* readings (Wi‑Fi + battery 64%), at 1280 and 1024 px |
| `a-statusbar-1280.png`, `a-statusbar-1920.png` | The bar alone |
| `a-quick-settings.png`, `a-quick-settings-laptop.png` | Quick Settings open |
| `a-power-menu.png`, `a-window-switcher.png`, `a-clipboard.png`, `a-command-palette.png`, `a-lock.png`, `a-notifications.png` | Each overlay or screen |

---

## 1. Inventory — what the shell has today

### 1.1 Status bar (`components/status_bar.slint`, 756 lines; drawn once in `app.slint:2648`)
- **Left:**
  - YantrikMark (20px) — `:128`.
  - A 6px companion-state dot. It is clickable and opens the Lens — `:134-180`.
  - The word "Yantrik" — `:182`.
  - Red danger chips for "no model" (`:191`) and "minds can't reach" (`:235`).
  - A project pill, shown at ≥1000px — `:275`.
- **Centre:** the AI context lane. It is text only, ≥1200px, and empty in every render — `:296-310`.
- **Right, in order:**
  1. CPU% and MEM text, ≥1100px — `:319-350`.
  2. The free-AI paste chip — `:359`.
  3. The **mind-mode chip** (Ask/Plan/Auto/Bypass/Private) — `:399-445`.
  4. The **active-mind chip**, shown only when it is not the companion — `:454-489`.
  5. The AI privacy/model chip — `:502-545`.
  6. The whisper count — `:548`.
  7. The **unread-notification count**, shown only when it is above 0 — `:573-595`.
  8. "INC" (`:598`) and "**DND**" text badges. These are not clickable — `:617-633`.
  9. The pending-queue count — `:636`.
  10. The **network glyph** and, only when a battery exists, the **battery**. They sit in one TouchArea that toggles Quick Settings — `:661-722`.
  11. Power — `:725-738`.
  12. Clock and date. These are **not clickable** — `:741-753`.
- **Missing:** there is **no volume**, **no Bluetooth**, **no brightness**, no mic/camera indicator, no keyboard layout, no workspace indicator and no recording indicator.
- **Network mark:** `Icons.wifi` or `Icons.network`, 13px, tinted green when online — `:669-678`. It shows no signal strength, no VPN and no "limited" state.
- **Battery:** a hand-drawn 20×10 rectangle plus "%" — `:684-719`. Charging is shown only by the fill colour. There is no bolt and no time remaining.

### 1.2 Taskbar (`components/taskbar.slint`, 328 lines; drawn once in `app.slint:2742`)
- It holds the Apps button, the open windows from `wlrctl` (with a slot for a minimised shell screen), Chat · <mind>, and a 12px Show-desktop strip (`taskbar.slint:8-21`).
- There is no tray, no clock, no workspace switcher and no window previews. That is deliberately minimal (`:15-21`), which is fine; the bar has to carry the rest.

### 1.3 Launcher, Lens and command palette
- **Launcher:** `components/app_grid.slint` (418 lines).
  - It has search, categories, a grid and pins, plus a footer with Lock/Suspend/Restart/Shut down (`:391-411`).
  - Super+Space opens it from anywhere through `yos act shell show_screen screen=launchpad` (`config/labwc/rc.xml:319-323`). It raises the shell (`wire/app_grid.rs:39`).
  - It is solid (`h-apps-button.png`).
- **Lens (the ask bar):** `components/intent_lens.slint` (2407 lines). It opens on Super+K (`rc.xml:222-226`) and Ctrl+K inside the shell (`app.slint:1276`).
- **Command palette:** `components/command_palette.slint`, triggered by Ctrl+Shift+P.
  - It has **no key binding at the compositor**. It is reachable only from the Lens (`wire/lens.rs:382`).
  - Its **render is broken** (`a-command-palette.png`):
    - The input row draws only a centred ">" (`:89`) and "Esc".
    - The category chips float to the row top.
    - The frame has a notch.

### 1.4 Notifications and Do Not Disturb
- **The store:** `services/notifications-service` owns `org.freedesktop.Notifications`, and the shell polls it at 1 Hz (`wire/notifications.rs:1-24`).
- **Toasts:** bottom-right, above the taskbar (`app.slint:3021`; kit `toast_banner.slint:58-63`).
- **Whisper cards:** top-right (`desktop.slint:986`).
- **The history** is a whole screen, `notification_center.slint` (screen 9, `app.slint:2113`). It is not a popover. In the render (`a-notifications.png`):
  - Rows have no app icon.
  - The body is indented differently from the summary.
  - The unread dot shifts the text.
  - There is **no DND switch** on it (grep: none).
- **Ways to change DND:**
  - Settings
  - The command palette (`wire/command_palette.rs:63`)
  - `act shell set_do_not_disturb` (`control.rs:1842`, graded sensitive)
  - A keybind that is never bound (§1.8)
- The bar's DND badge is not clickable.

### 1.5 Quick Settings (`components/quick_settings.slint`, 363 lines)
- **Contents:**
  - One Wi‑Fi tile. Its wireless glyph is hand-drawn from three circles (`:104-136`) instead of the kit's `Icons.wifi`, which the bar uses at `status_bar.slint:674`.
  - Brightness and Volume sliders (`:169-309`).
  - A battery row (`:313-360`).
- **Where it is drawn:** only inside `DesktopScreen` (`desktop.slint:910-925`). The bar's click toggles `quick-settings-open` from every screen (`app.slint:2692`).
  - **On screens 4–35 (Files, Settings, Agents…) it draws nothing.** This is the same class of bug the launcher had (#219), which was fixed for the launcher (`app.slint:2758-2772`).
  - Opening it never calls `raise_shell()`. Compare `wire/app_grid.rs:39` and `wire/lens.rs:425`.
- **Placement:** centred on the screen (`x: (parent.width - 380px)/2`, `:39`), while its trigger is at the far right of the bar (`a-quick-settings*.png`).
- **The Wi‑Fi tile** runs `nmcli radio wifi on|off` (`wire/callbacks.rs:346-361`), while its caption says "Tap to disconnect" (`:160`). It has no network list.
- **The brightness slider:**
  - It is shown on every machine, including the QEMU VM, which has no backlight.
  - It runs `brightnessctl s N%` (`callbacks.rs:365-375`).
  - Its value starts at a hard-coded 80 (`app.slint:462`) and is **never read from the device**.
- **The volume slider:**
  - It runs **`amixer -M set Master`** (`callbacks.rs:378-388`), which is ALSA.
  - The image runs PipeWire (`pipewire-pulse wireplumber`, `build-debian-iso.sh:253`). The labwc media keys use `wpctl` (`rc.xml:246-264`), and so do the mind's tools (`yantrik-companion-tools/src/system.rs:198-226`, `media.rs:109-194`). That makes three code paths and two mixers.
  - Its value starts at a hard-coded 50 (`app.slint:463`) and is **never read**.
- **Visible defects** (`a-quick-settings-laptop.png`):
  - The two slider tracks render at different widths and offsets.
  - The panel is 19% transparent (`:50`), so the desktop greeting shows through behind "Brightness".
  - Several sizes are hard-coded instead of tokens: 68×56, 20px radius, 14px spacing, legacy `font-body`/`font-small`.

### 1.6 Lock screen (`lock.slint`, 276 lines; screen 3, plus the compositor session lock #313)
- **Contents:** clock, date, an amber orb, a greeting, the prompt, the password field, a "○/◉" text glyph to show the password (`:230`) and "Press Enter to unlock" (`a-lock.png`).
- **The background** is `bg-deep` with two glows. It does not use the wallpaper.
- **Not shown:** user avatar, battery, network, notification count, media controls, keyboard layout, power/suspend and accessibility.
- **Idle cost:** the glow animation follows the renderer's budget and spends nothing on the software renderer (`:21-40`). This is good.

### 1.7 Desktop
- There are **two different home designs**:
  - everyday `DesktopHome` (`components/desktop_home.slint`, placeholder "Ask, search, or open anything" `:81`)
  - an agent-mode hero inside `desktop.slint` ("Ask, search, or run anything" `:567`, "Ready when you are" `:826`)
  - Compare `h-desktop.png` with `h-agent.png`: different search fields, different pin tiles, different spacing.
- **Shared elements:**
  - The right-click menu: Terminal/Files/All Apps/Devices/Settings (`desktop.slint:1063-1082`).
  - The morning brief (`:996`).
  - The mind panel at the right edge (`app.slint:2810`, `h-mind-panel.png`).

### 1.8 Window management and keys (`config/labwc/rc.xml`, 378 lines)
- **Windows:** Alt+Tab/Alt+Shift+Tab use labwc's own OSD (`:99-103`), with the colours in `themerc:63-67`. Also bound: Alt+F4, Super+↑/↓/←/→ and F11 (`:105-121`).
- **Snap:**
  - Fourteen named regions on Super+Alt+letter (`:133-176`, `:350-365`).
  - Edge-drag snapping with a 12px range and overlay (`:331-342`).
  - Window-menu snap (`menu.xml`).
  - This is better than most desktops.
- **The shell's own keys:**
  - Super+L lock, Super+E Files, Super+I Settings, Super+D show desktop, Ctrl+Alt+T terminal, Super+K Lens, Super+Space launcher (`:180-226`, `:319`).
  - All of them go through `yos act shell …`. That is the right pattern: a key and a mind take the same path (`:92-95`).
- **Workspaces:**
  - Four exist: Super+1…4, Super+Shift+1…4 and Super+Ctrl+←/→ (`:284-313`, `:367-370`).
  - The shell is on all of them (`:374-376`).
  - **There is no indicator and no overview.** The only feedback is labwc's 600ms popup.
- **Dead bindings.** `crates/yantrik-os/src/keybinds.rs:39-133` declares Super+V clipboard, Super+Shift+Q power menu, Super+Tab switcher, Super+A app grid, Super+D DND, Shift/Ctrl+Print, and `Super_L` → Lens. They are written into `rc.xml` only **if no `rc.xml` exists** (`:262-265`). `yantrik-session:85-87` copies the shipped `rc.xml` at every login, so **none of them is ever bound**. That leaves:
  - the clipboard panel advertising "Super+V to toggle" (`clipboard_panel.slint:220`) for a key that does nothing
  - the shell window switcher ("Triggered by Super+Tab", `window_switcher.slint:1-4`) unreachable from the keyboard
  - the `handle_keybind` arms for clipboard, power menu, app grid and window switcher (`wire/system_poll.rs:496-530`) as dead code. Each is also guarded to screen 1 only (`:497`, `:513`, `:518`, `:523`).
  - **Super+D** meaning "show desktop" in `rc.xml:195` and "toggle DND" in `keybinds.rs:101`.
- **No shortcut cheat sheet** exists anywhere.

### 1.9 System indicators: what exists only as data
| Datum | Where it is read | Where it is shown | `describe shell` |
|---|---|---|---|
| Battery %, charging | UPower D-Bus every 30s (`yantrik-os/src/battery.rs:11,30-48,59-115`) → `system_poll.rs:232-234` | Bar (`status_bar.slint:681`), QS (`quick_settings.slint:314`) | `battery` (`control.rs:898-908`), null when absent ✔ |
| Battery time-to-empty | `battery.rs:114` (`TimeToEmpty`) | **nowhere** | no |
| Network online/medium/label/detail/SSID/IP | network service → `system_poll.rs:760-805` | Bar glyph, QS title, System screen | `network`, `wifi` (`control.rs:883-897`) ✔ |
| Wi‑Fi signal, other SSIDs | **not read** | — | no |
| Volume, mute, output device | **not read** (QS default 50) | slider only | **no** |
| Brightness, backlight presence | **not read** (QS default 80) | slider only | **no** |
| Bluetooth | only the mind's tool (`yantrik-companion-tools/src/bluetooth.rs`, `bluetoothctl`) | **nowhere** | **no** |
| DND | settings | "DND" badge | `do_not_disturb` ✔ |
| CPU/MEM/disk | snapshot | Bar text, System | ✔ |
| Workspaces | labwc only | labwc popup | **no** |
| Mic / camera in use | **not read** | — | **no** |
| Media (MPRIS) | **not read** (grep: no `mpris`/`playerctl` in crates or apps) | — | **no** |
| Keyboard layout | written once by the installer (`installer_rules.rs:235-243`) | — | no |

`act shell` has today: `lock`, `set_do_not_disturb`, `show_screen`, `show_desktop`, `open_app`, `open_lens`, window focus/close/minimise/maximise and others (`control.rs:985-1900`). It has **nothing for volume, brightness, Wi‑Fi, Bluetooth, power, Quick Settings or screenshots.**

### 1.10 OSDs
- **None.** The volume, mute, mic-mute and brightness keys run `wpctl` and `brightnessctl` with no visual response (`rc.xml:246-275`).
- There is no caps-lock or keyboard-layout OSD.

### 1.11 Power menu (`components/power_menu.slint`, 185 lines; `wire/power.rs`)
- It is a centred modal with Lock, Suspend, Restart and Shut Down (`a-power-menu.png`). The launcher footer repeats the same four.
- **Missing:** Log out, Hibernate and confirmation.
- Restart and shutdown fire immediately through `systemctl` (`power.rs:28-35`), with no "two minds are still working" warning.
- It lives only inside `DesktopScreen` (`desktop.slint:928`), so it has the same dead-on-other-screens bug as Quick Settings.

### 1.12 Screenshot and recording
- Print and Super+Shift+S run `grim` (or `grim -g "$(slurp)"`) straight to `~/Pictures` (`rc.xml:234-243`). There is no sound, no toast, no thumbnail, no copy and no editor.
- The shell's `wire/screenshot.rs` (which has toasts and clipboard modes) is reachable only through the dead keybinds.
- **No screen recording.** No recorder is in the image.

### 1.13 Clipboard (`components/clipboard_panel.slint`, `wire/clipboard.rs`)
- History is text only: preview and time-ago (`a-clipboard.png`). The search field shows a "/" glyph as its only affordance.
- No key opens it (§1.8).
- It refreshes through a **200ms repeating timer** that watches a property (`wire/clipboard.rs:22-30`). The launcher uses `changed app-grid-open` instead (`app.slint:393-396`); this should do the same.

### 1.14 Theming and wallpapers
- **What exists:**
  - Dark and light (`ThemeMode`, `theme.slint:17`).
  - Accent presets in the tokens: teal, amber, purple, green (`:28-80`). Settings shows five swatches (`h-settings.png`).
  - `ThemeOverrides` (`:85-97`).
  - Seven wallpapers plus solid and custom (`ui/wallpapers/`, `h-settings.png`).
- **What a theme change does not reach:**
  - labwc title bars and the Alt+Tab OSD: **hard-coded** dark colours in `config/labwc/themerc:38-67`. A light theme still gets dark app frames.
  - foot, GTK, Chromium and the lock screen.
- **There is no "theme" as a single choice.**

### 1.15 Motion
- 129 `animate` sites. Ambient loops respect `ambient-interval-ms`, which is 0 on the software renderer: `lock.slint:21-40`, `status_bar.slint:151-176`.
- Window frames animate geometry at 200ms (`app.slint:2589`). Tokens: `dur-fast` 120 / `dur-normal` 200 / `dur-slow` 300 (`theme.slint:363-365`).
- **Missing:** popovers and overlays appear without any transition, and there is no motion token for an OSD hold or fade.

### 1.16 First run (`onboarding.slint`, 1412 lines)
- **Phases:** orb → greeting → name → interests → location and notifications → AI hardware → AI mode → provider → test → instruction → ready (`:1-10`).
- **It never teaches the keys.** There is no Super+K / Super+Space card and no "learn the keybindings".
- It offers no look (theme or wallpaper) choice and no network step. It assumes a network is already up.

### 1.17 Per-screen chrome consistency
- **Hosted screens that wear the frame's bar *and* their own AppHeader:** screens without `content-has-controls` include Packages 21 (`app.slint:2320`), Devices 27 (`:2396`) and Permissions 28 (`:2458`). Packages, Devices and Permissions use `AppHeader`, so they draw two title rows.
- **Restored, not maximised:** every hosted screen shows the frame's 36px bar above its AppHeader (`window_frame.slint:126`, `:185`). The one-bar change (`114d6c4`) covers Files and Settings when maximised only.
- **Agents and Recipes (34, 35)** use the frame bar and no AppHeader, which is a third look.
- **Two type scales** coexist: legacy `font-*` (`theme.slint:275-282`) and `fs-*` (`:290-310`). Quick Settings uses the legacy one.
- **Two window switchers** (labwc OSD and the shell's) and **two desktop homes** (§1.7).

### 1.18 The structural root cause
The bar and taskbar are not panels. `yantrik-ui` is **one fullscreen toplevel**, kept clear of apps by labwc's `<margin top="32" bottom="40">` workaround (`rc.xml:44-61`) and made omnipresent by a window rule (`:374-376`). The code says so itself (`windows.rs:520-526`). The consequences:
- **Overlays draw behind apps.** Anything the bar opens is drawn inside the shell window, so it is only visible after the shell raises itself (`raise_shell()`). Quick Settings, the power menu and the clipboard never do.
- **Clicking the bar raises the whole desktop over your work** (*verify live*). `rc.xml` defines no `<mouse>` section, so labwc's default (Focus and Raise on click) applies to the shell window.
- **Fullscreen apps, exclusive zones and multi-monitor** depend on the margin hack. The comment at `rc.xml:58-59` already names the proper fix: speak `wlr-layer-shell`.

---

## 2. What the pixels say (the first five minutes)

1. **The bar reads as a developer's status line, not a desktop's** (`a-statusbar-1920.png`). CPU and MEM in letters take the most room, the AI chips come next, and the network mark is about 13px. At 1920px, 1,300px of the bar is empty.
2. **Nothing on the right edge says "sound".** A person who wants to turn it down has nowhere to click.
3. **Quick Settings looks unfinished** (`a-quick-settings-laptop.png`):
   - It drops from the centre while the trigger is at the right.
   - The slider tracks disagree.
   - Desktop text shows through.
   - The Wi‑Fi tile's glyph is a target, not Wi‑Fi.
4. **The power menu is clean but heavy**: a full-screen dim and a centred card for what GNOME and macOS do in one popover.
5. **The lock screen is a dark void with an amber blob** (`a-lock.png`). It shows no wallpaper, no avatar and no state.
6. **The window switcher is three grey boxes with elided text** and no icons (`a-window-switcher.png`). It is unreachable anyway.
7. **The command palette is visibly broken** (`a-command-palette.png`).
8. **The approval card is the product's signature moment, and it is the densest surface on screen** (`h-approval-card.png`): seven provenance lines at 11px before the action.
9. **What already works and should be kept:**
   - The desktop home and launcher typography (Barlow, the 44px hero, good spacing).
   - The taskbar's restraint.
   - The mind panel's information design (`h-mind-panel.png`).
   - Settings' appearance page.

---

## 3. Benchmark — what each gets right in the first five minutes

The versions and specifics below come from web research done for this audit on 2026-10-01, with sources at the end. Current releases are Omarchy 4.0.4, GNOME 51, Plasma 6.7, COSMIC epoch‑1.9, macOS 27 and Windows 11 26H2. Where the request named a component the project has since replaced (Omarchy's Waybar and Mako), both are described.

### Omarchy (Arch + Hyprland)
- **Waybar (3.x), 26px at the top:**
  - **Left:** the Omarchy logo (click opens the Omarchy menu; right-click opens a terminal) and workspaces 1–5 as numbers (active one filled; click switches).
  - **Centre:** the clock (click toggles a date + ISO-week view), plus state chips shown *only while active*: recording (click stops it), idle-lock off, notifications silenced, dictation, update available.
  - **Right:** tray expander, Bluetooth (click opens `bluetui`), network (signal icon, tooltip "SSID (GHz)"; click opens `impala`), audio (click opens `wiremix`, right-click mutes, scroll ±5%), CPU (click opens `btop`), and battery (icon while discharging, tooltip in watts; click opens power profiles).
- **Launcher and menu:**
  - **Walker on Super+Space** finds apps.
  - **The Omarchy menu on Super+Alt+Space** puts the whole system in one tree: Learn, Trigger/Capture, Toggle, Style, Setup, Install, Remove, Update, System. Direct keys jump into a branch: Super+Ctrl+C Capture, Super+Ctrl+O Toggle, Super+Escape System.
  - In 4.0, Super+Space is **one fuzzy palette of apps *and* commands**.
- **Mako notifications:** top-right, 420px, a 2px theme border, 5s timeout. Keys: Super+, dismiss; Super+Shift+, dismiss all; **Super+Ctrl+, DND**; replay last.
- **Themes:**
  - 21 built in. Super+Shift+Ctrl+Space opens the picker.
  - `omarchy-theme-set` changes everything at once: Hyprland borders, the bar, the OSD, terminal, btop, notifications, editor, GTK dark/light, the browser colour, the lock screen and the wallpaper. **This is the single most noticed feature.**
- **Capture:**
  - PrtSc gives a region via slurp, then the `satty` editor; Enter copies and saves.
  - Alt+PrtSc records with gpu-screen-recorder, with or without audio or webcam. A red bar chip shows while recording; click stops it.
- **OSDs:** volume and brightness keys show a themed `swayosd` pill.
- **hyprlock:** blurred wallpaper, one centred input, "FAIL (n)". hypridle runs the screensaver at 150s and locks at 152s.
- **Keyboard first:** Super+K lists every binding. First run posts "Learn Keybindings", "Setup Wi‑Fi" and "Update" as clickable notifications.

### GNOME 47–51
- **Quick Settings (top-right; one click on the network/volume/battery cluster):**
  - **Top row:** battery %, screenshot, Settings, Lock, Power (Suspend/Restart/Power Off/Log Out).
  - **Sliders:** volume (with a chevron to choose the output), mic (only while in use), brightness.
  - **Split pills** (toggle on the left, chevron for a submenu on the right): Wi‑Fi (network list), Wired, Bluetooth (devices), Power Mode, Night Light, Dark Style, Airplane, **DND** (here since 49), Screen record.
- **Activities (Super):** workspaces, search, dash, and type-to-search over apps, settings, files and a calculator.
- **Clock:** notifications, media player, calendar, events, world clocks and weather.
- **OSDs:** a rounded pill at bottom-centre (icon and level) for volume and brightness. Super+Space switches keyboard layout with an OSD.
- **Screenshot:** PrtSc opens an in-shell selection/screen/window UI with a photo/video toggle, and a red timer pill while recording.

### KDE Plasma 6.7
- **System tray applets, each a popover:**
  - **Networks:** list with inline password, QR share.
  - **Bluetooth:** devices with battery levels.
  - **Audio Volume:** a Devices tab and an Applications tab with **a slider per app**.
  - **Power and Battery:** profile slider, plus "block sleep".
  - **Brightness and Colour:** per-screen, Night Light.
  - **Clipboard:** Meta+V, pins, QR.
  - **Notifications:** DND for 1h or until tomorrow.
  - **Disks & Devices:** pops open on USB insert.
  - **KDE Connect.**
- **Elsewhere:** a floating panel, Kickoff on Meta, KRunner on Alt+Space (calculator, units, commands), Overview on Meta+W, and a small OSD pill.

### COSMIC (epoch 1.x)
- **Panel:** Workspaces and Applications on the left, the clock in the centre, and on the right tiling, network, Bluetooth, sound (output and input sliders plus device picker), battery (% plus profiles plus brightness), notifications (history plus DND) and user/power.
- **Keys:** Super opens a launcher with prefixes for calculator, command and path. Super+W opens the workspaces overview. Super+Y toggles auto-tiling.
- **OSD:** bottom-centre.
- **Appearance:** dark/light, accent, roundness, density, frosted glass, and theme import/export.
- **First run:** a setup wizard asks layout, tiling and launcher behaviour.

### macOS (26 Tahoe / 27)
- **Menu bar extras:** Wi‑Fi, Bluetooth, battery, sound, Control Center and the clock. Each is a dropdown, and individual controls can be pinned to the bar.
- **Control Center:** a Wi‑Fi/Bluetooth/AirDrop tile, Focus, display brightness, sound, **Now Playing**, screen mirroring. Editable.
- **Spotlight (Cmd+Space):** Apps, Files, **Actions** (run system and app actions) and **Clipboard** history.
- **Notification Center:** opens from the clock.
- **Volume and brightness HUD:** drawn just under the menu bar, near the control it belongs to.
- **Privacy:** an **orange dot** while the mic is live and a green indicator for the camera. Control Center names the app.
- **Capture:** Cmd+Shift+5 opens a capture toolbar with modes and recording, plus a floating thumbnail to mark up.

### Windows 11 (25H2 / 26H2)
- **Quick Settings (Win+A, or one click on the combined network/volume/battery button):**
  - Tiles: Wi‑Fi and Bluetooth (each with a chevron list), Airplane, Energy saver, Night light, Accessibility, Cast, Hotspot. Editable with the pencil.
  - Brightness and volume sliders, the **per-app mixer** button (Win+Ctrl+V), battery % and a Settings gear.
- **Tray:** an overflow chevron, **mic and camera in-use icons** (hover names the app), and a colour-coded battery.
- **Win+N:** notifications plus the calendar, with an agenda and "Join".
- **Windows and desktops:** Snap layouts (hover maximise, Win+Z), Win+Tab Task View with desktops, Alt+Tab thumbnails.
- **Win+Shift+S:** the snip bar, with recording.
- **Win+V:** clipboard history with pins.
- **OSD:** a bottom-centre pill.

### Table stakes all six share
1. Network, audio, battery and clock are always visible, and one click opens a popover to change them.
2. One quick-toggles surface: Wi‑Fi and Bluetooth with lists, volume with output, brightness, a power profile.
3. A non-interactive OSD for volume and brightness.
4. A one-key launcher that also finds settings and does maths.
5. Toasts, history and **DND within two clicks or one key**.
6. A screenshot key with region selection, a capture feedback thumbnail and recording with a visible stop.
7. Clipboard history, and workspaces with an indicator and overview.
8. Mic and camera privacy indicators.
9. Lock and power within one click of the bar.

### Where the best go further
- Omarchy's one-switch theme.
- The launcher-as-actions (Omarchy 4, Spotlight Actions).
- Super+K cheat sheet discoverability.
- Per-app volume (Plasma, Windows).
- Context pop-ups: Plasma's USB, the macOS HUD next to its control.

---

## 4. Gap analysis

Severity: **P0** means a first-impression failure, **P1** means its absence is noticed within a day, **P2** is polish or delight. Effort: **S** is under a day, **M** is 2–4 days, **L** is a week or more.

| # | Capability | Yantrik today (file:line) | Best-in-class reference | Sev | Effort |
|---|---|---|---|---|---|
| 1 | **Volume indicator in the bar** | none (`status_bar.slint:313-753`) | GNOME/macOS/Windows: speaker icon by level, click → slider + output; Waybar: scroll ±5, right-click mute | **P0** | M |
| 2 | **Volume control actually controls PipeWire** | QS slider → `amixer` (`callbacks.rs:378-388`), never read (`app.slint:463`); keys → `wpctl` (`rc.xml:246-264`) | `wpctl` / WirePlumber everywhere, level read back | **P0** | S |
| 3 | **Bar controls work on every screen and above apps** | QS & power drawn only in `desktop.slint:910-934`; no `raise_shell` | every desktop: the panel is a layer above windows | **P0** | S (hoist + raise), L (layer-shell) |
| 4 | **Network/Wi‑Fi indicator + popover** | 13px glyph, no strength (`status_bar.slint:669-678`); QS tile toggles radio, captioned "disconnect" (`quick_settings.slint:160`, `callbacks.rs:346`) | GNOME split pill + network list; Plasma Networks applet with inline password | **P0** | M |
| 5 | **Battery indicator (when present)** | drawn if UPower answers (`status_bar.slint:681`), but **`upower` is not in the image package lists** (`build-debian-iso.sh:205-260`, `user-data.yaml:38-130`, `yantrik-update:1462`); no sysfs fallback; no time remaining | GNOME %, bolt, "plugged in, not charging"; Windows colour-coded; Waybar wattage tooltip | **P0** | S–M |
| 6 | **OSD: volume, brightness, mic mute, caps lock** | none (`rc.xml:246-275`) | GNOME/Windows/COSMIC bottom-centre pill; macOS HUD under its control | **P0** | M |
| 7 | **Quick Settings panel: toggles and sliders** | 3 controls, centred, defects (`quick_settings.slint:39,50,104-309`), brightness shown with no backlight | GNOME QS / Windows Win+A / macOS Control Center | **P0** | M–L |
| 8 | Bluetooth indicator + tile | none in the shell (only mind tool `bluetooth.rs`) | GNOME pill + device list; Plasma battery levels | P1 | M |
| 9 | Brightness (laptop) | slider always shown, value never read (`app.slint:462`, `callbacks.rs:365`) | hidden without backlight; keys show OSD | P1 | S |
| 10 | Mic / camera in-use | none | macOS orange dot; Windows tray icons naming the app | P1 (privacy-forward OS) | M |
| 11 | Notifications popover + DND | full screen 9 (`app.slint:2113`); no DND switch; DND badge not clickable (`status_bar.slint:617`) | GNOME clock popover + DND pill; Mako Super+Ctrl+, | **P1** | M |
| 12 | Clock → calendar popover | clock is plain `Text` (`status_bar.slint:741`) | GNOME/Windows calendar + agenda; Waybar date toggle | P1 | M |
| 13 | Keyboard layout indicator | none (layout only set at install, `installer_rules.rs:235`) | GNOME Super+Space OSD; tray layout chip | P2 (P1 when >1 layout) | S |
| 14 | Power menu | centred modal; no Log out / Hibernate / confirm; no "minds still working" (`power_menu.slint`, `power.rs:28-35`) | GNOME QS power button; Omarchy System menu on Super+Escape | P1 | S |
| 15 | Workspaces + indicator/overview | 4 workspaces bound (`rc.xml:284-313`), no indicator, no overview | Waybar 1–5 numbers; GNOME Activities; COSMIC Super+W | **P1** | M (indicator) / L (overview) |
| 16 | Launcher | good: Super+Space, categories, pins (`app_grid.slint`) | Omarchy 4 / Spotlight: apps **and** commands, calculator | P2 | M |
| 17 | Command palette | Ctrl+Shift+P only from Lens (`lens.rs:382`), **broken input** (`command_palette.slint:89`) | Omarchy menu / KRunner | P1 | S (fix) / fold into Lens |
| 18 | Lock screen polish | bare (`lock.slint`, `a-lock.png`) | hyprlock blurred wallpaper; GNOME 49 media on lock | P1 | M |
| 19 | Wallpapers & one-switch themes | dark/light + 4 accents + 7 wallpapers; labwc/foot/GTK not themed (`themerc:38-67`) | `omarchy-theme-set` | **P1** | M–L |
| 20 | Typography/spacing/density consistency | two type scales (`theme.slint:275-310`); hard-coded px (`quick_settings.slint`, `status_bar.slint:207,251`); double bars (`app.slint:2320,2396,2458`; `window_frame.slint:126`); two homes (`desktop.slint:567` vs `desktop_home.slint:81`) | COSMIC density/roundness settings; libadwaita consistency | P1 | M |
| 21 | Motion | disciplined; overlays pop with no transition | GNOME 200ms ease; macOS HUD fade | P2 | S |
| 22 | Shortcuts + cheat sheet | good bindings, **dead generator** (`keybinds.rs:39-133,262`), Super+D conflict, no cheat sheet | Omarchy Super+K list; GNOME Settings › Keyboard | **P1** | S–M |
| 23 | Screenshot & recording | grim to file silently (`rc.xml:234-243`), shell path unreachable (`screenshot.rs`), no recording | GNOME in-shell capture UI; macOS ⌘⇧5; Omarchy satty + recording chip | **P1** | M |
| 24 | Clipboard | text history, no key, 200ms poll timer (`clipboard.rs:22-30`) | Win+V pins; macOS Spotlight Clipboard; Klipper | P1 | S–M |
| 25 | Media widget (MPRIS) | none | GNOME clock popover/lock media; macOS Now Playing | P2 | M |
| 26 | First-run experience | long AI-centred onboarding; no keys, no look, no network (`onboarding.slint:1-10`) | Omarchy "Learn Keybindings" notifications; COSMIC initial setup | P1 | M |
| 27 | Window switcher | two: labwc OSD (live) + shell overlay (unreachable, no icons) | Windows Alt+Tab thumbnails; GNOME switcher | P2 | S (delete one) |
| 28 | Panel architecture | fullscreen toplevel + margins (`rc.xml:44-61`, `windows.rs:520`) | every listed desktop: layer-shell panels | P1 (root cause) | L |

### 4.1 Linux data source, control path and what the image ships
| Indicator | Read | Control | Event source (no polling) | On the image? | Absent device |
|---|---|---|---|---|---|
| Volume / mute / output | `wpctl get-volume @DEFAULT_AUDIO_SINK@`, `wpctl status` | `wpctl set-volume -l 1.0 …`, `set-mute`, `set-default <id>` | `pactl subscribe` (sink/server events), one long-lived child | **yes**: `pipewire-pulse wireplumber pulseaudio-utils` (`build-debian-iso.sh:253`, `user-data.yaml:59-61`) | No sink: speaker-slash icon, popover says "No output device". The VM has a virtual sink. |
| Mic in use / mic mute | `pactl list source-outputs` (client app names) | `wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle` | same `pactl subscribe` (source-output) | yes | No source: hidden |
| Network / Wi‑Fi | NetworkManager D-Bus (`org.freedesktop.NetworkManager`, `AccessPoint.Strength`); `nmcli -t -f … dev wifi list` | `nmcli radio wifi on|off`, `nmcli dev wifi connect <ssid> [password]` | NM `PropertiesChanged` / `StateChanged` signals (zbus already used: `yantrik-os/src/network.rs`) | **yes**: `network-manager wpasupplicant` (`build-debian-iso.sh:225-229`, `user-data.yaml:124`) | No Wi‑Fi device: the tile is hidden and the bar shows the wired/offline mark |
| Battery | UPower `DisplayDevice` (`battery.rs`) and fallback `/sys/class/power_supply/BAT*/{capacity,status}` | n/a | UPower `PropertiesChanged` (today: 30s poll) | **NO**: `upower` is not listed in `build-debian-iso.sh`, `user-data.yaml` or `yantrik-update` `REQUIRED_PACKAGES` (`:1462`). It may arrive only as an apt Recommends: *verify on a laptop image*. | Hidden (already correct: `battery.rs:78-99`) |
| Power profile | `powerprofilesctl get` / D-Bus `net.hadess.PowerProfiles` | `powerprofilesctl set balanced|power-saver|performance` | D-Bus signal | **NO**: needs `power-profiles-daemon` | Hidden |
| Brightness | `/sys/class/backlight/*/{brightness,max_brightness}` | `brightnessctl set N%` (or logind `org.freedesktop.login1.Session.SetBrightness`, no setuid) | inotify on the sysfs file | **yes**: `brightnessctl` (`build-debian-iso.sh:253`, `user-data.yaml:114`) | **No backlight (QEMU): hide the slider** (today it is shown) |
| Bluetooth | BlueZ D-Bus (`org.bluez.Adapter1.Powered`, `Device1.Connected`, `Battery1`), or `bluetoothctl` | `bluetoothctl power on|off`, `connect <mac>`; `rfkill` | BlueZ `InterfacesAdded` / `PropertiesChanged` | **yes**: `bluez` (`:253`, `user-data.yaml:116`) | No adapter (`/sys/class/bluetooth` empty, which is the case on QEMU): tile and indicator hidden |
| Camera in use | PipeWire video nodes in `pw-dump`, or `/dev/video*` held open | n/a | `pw-mon` / poll only while a capture is announced | pipewire yes | No `/dev/video*`: never shown |
| Keyboard layout | `~/.config/labwc/environment` `XKB_DEFAULT_LAYOUT` (`installer_rules.rs:235`) | rewrite and `labwc --reconfigure` | file change | labwc yes | Hidden with a single layout |
| Media (MPRIS) | session D-Bus `org.mpris.MediaPlayer2.*` | `PlayPause`, `Next`, `Previous` | `PropertiesChanged` | dbus yes; **the Music app publishes no MPRIS** | Hidden with no player |
| Recording | — | `wf-recorder -g "$(slurp)"` | process lifetime | **NO**: needs `wf-recorder` | — |
| Night light | — | `wlsunset -T/-t` (wlr-gamma-control) | — | **NO**: needs `wlsunset` | Hidden without gamma control |

**Package changes** (all three lists, kept in step by the existing parity check, `build-debian-iso.sh:250`): add `upower power-profiles-daemon wf-recorder`, plus `wlsunset` for P2. Add `upower` and `power-profiles-daemon` to `yantrik-update` `REQUIRED_PACKAGES` so machines that already exist gain them.

### 4.2 Hardware reality
The main test machine is QEMU, with no battery, no backlight, no Bluetooth adapter and a virtual audio sink. The rules:
- **Every indicator, tile and slider decides its own presence from the device, not from configuration.** Use a three-state reading `{absent, unknown, present}`. Draw nothing when absent, and a dimmed placeholder only while unknown at boot.
- **The harness renders the VM shape and the laptop shape for every phase.** The `a-shell*.png` and `a-quick-settings*.png` pairs are the baseline to beat.
- **No stub data.** `brightness-level: 80` and `volume-level: 50` (`app.slint:462-463`) are exactly the shape of lie the project's own rules forbid. They go.

### 4.3 Control-surface parity (project rule: what a pointer can do, a mind can ask for)
Every new control is published on `describe shell` / `act shell` in the same PR as its pixels. Each action answers by **reading back** the system, as `set_do_not_disturb` reads its file back (`control.rs:1848-1855`), never by echoing the request.

| `describe shell` field | `act shell` action | Grade | Why that grade |
|---|---|---|---|
| `audio {available, volume, muted, output, outputs[]}` | `set_volume level=|step=`, `set_mute on=`, `set_output id=` | safe | reversible, local, the person hears it instantly |
| `mic {available, muted, in_use_by[]}` | `set_mic_mute on=` | **sensitive** to un-mute | unmuting a mic is a privacy act |
| `brightness {available, percent}` (null on QEMU) | `set_brightness level=|step=` | safe | |
| `network.wifi {radio, ssid, signal, networks[]}` | `set_wifi on=`, `connect_wifi ssid= password?` | **sensitive** | turning the radio off cuts the person, and every mind, off |
| `bluetooth {available, powered, devices[]}` | `set_bluetooth on=`, `connect_bluetooth id=` | sensitive | pairs and connects devices |
| `battery {percent, charging, time_to_empty}`, `power_profile` | `set_power_profile name=` | safe | |
| `quick_settings {open}`, `clock_popover {open}`, `power_menu {open}` | `open_quick_settings`, `close_quick_settings`, `open_clock` … | safe | describes what is on screen, like `launcher.open` (`control.rs:928`) |
| `workspaces {count, active}` | `go_to_workspace n=` | safe | |
| `recording {active, since}` | `screenshot mode=`, `start_recording`, `stop_recording` | **sensitive** (stop: safe) | reads the person's screen |
| `theme {name, dark, accent, wallpaper}` | `set_theme name=` | sensitive | a lasting change to the machine, the same reasoning as DND (`control.rs:1836-1841`) |
| — | `power action=suspend|restart|shutdown|logout` | **sensitive, deferred** | answers with which minds are still working before acting |

Keys in `rc.xml` call these same actions through `yos`, so one path serves the key, the pointer and the mind (`rc.xml:92-95`).

---

## 5. Design direction — what "10× better" looks like, and why it isn't Omarchy

Omarchy wins on **coherence and speed**: one theme changes everything, every action has a key, nothing animates for show. Yantrik should take that discipline and nothing of the look. The look should come from what only this OS has: **minds working beside a person, with the person always knowing who is acting, how freely, and what is waiting for them.**

1. **The bar is a sentence about the machine. Read it left to right: *who's here → what time it is → how the machine is.***
   - **Left: the minds.** The mark, then one chip per active mind with a thin progress ring. An approval waiting turns that mind's chip amber, with one pulse when it arrives and no loop. Clicking a chip opens its pane. No other desktop has this zone, so it becomes the signature.
   - **Centre: the clock.** One click opens **Today**: calendar, notifications, now-playing, and a *"While you were away"* digest of what minds finished. That is the morning brief, moved to where a person already looks.
   - **Right: the machine.** Volume, network, Bluetooth, battery, then **the mind-mode chip** (Ask/Plan/Auto/Private, already red in bypass), then power. CPU and MEM move into the System popover and the mind panel's MACHINE section. On a laptop they are not first-impression information.
   - **Quiet by default.** An indicator draws only when it has something to say. Recording, mic in use, DND and bypass show up as state chips while they are true, as in Waybar, and are gone otherwise.
2. **One popover grammar for everything the bar opens.** It is anchored under its indicator, 360px, `glass-raised`, `r-xl`, with a 120ms fade and 8px drop. Quick Settings, Today, network, sound, power and the minds panel use the same kit `YPopover`, so the shell feels like one object. The approval card uses the same grammar too: it drops from the mind chip that asked.
3. **Quick Settings has tiles nobody else can have.** Alongside Wi‑Fi, Bluetooth, DND, Night light, Dark style and Power mode, the first row holds **Mind mode** (split: toggle Ask↔Plan, chevron for the full menu), **Private** (the Mind is off), and **Mind View** (show or hide the minds' desk). The things that make this OS different sit one click from the bar, next to Wi‑Fi, and they are honest about what they do.
4. **Colour means something.**
   - Teal (accent) is a mind.
   - Amber is "needs you".
   - Red is "it is not asking you" (bypass) or a failure.
   - Everything else is neutral glass.
   - This is already in the tokens (`tint-accent`, `tint-amber`, `color-danger-dim`). The overhaul makes it a rule, and the cheat sheet explains it in one line.
5. **Stillness is the brand.** The software renderer forced motion discipline (`status_bar.slint:151-176`). Keep it and say it: things move only when their state changes, 120–200ms, never on a loop. A Yantrik desktop at rest is a photograph, and a person sees that as calm and confident.
6. **Themes are places.** Each of the seven wallpapers becomes a theme: Serenity, First Light, Nightfall, Aurora, Sunset, Ocean, Nebula. Picking one sets the wallpaper, palette, accent and dark/light. It also writes labwc's themerc, foot's colours, GTK's preference and the lock screen. One choice changes the whole machine, the Omarchy lesson, but in Yantrik's own visual language: calm landscapes and glass, not terminal palettes.
7. **The Lens is the menu.** Omarchy needs a menu tree because its launcher only finds apps. Yantrik's Lens already classifies deterministic intents. Typing "volume 30", "wifi off", "dark", "screenshot" or "record" runs the action at once (the same `act shell` path, with the same grades), and anything else is a question for the mind. Super+K is one door for apps, commands, settings and questions. Super+/ shows the keys.
8. **Approvals read like a sentence, then the proof.** The card leads with *"pi wants to start the Council on 'ship 0.4 Friday'"* and two buttons. The seven provenance lines move under "Why you can trust this" (`h-approval-card.png`). The trust model stays intact and becomes legible.

---

## 6. Plan — phased, small PRs, each one ships something visible

Ground rules for every PR:
- **Kit first.** Use one shared `YIndicator`, `YPopover`, `YSlider`, `YToggleTile` and `YOsd` in `crates/yantrik-ui-kit/slint/`, with re-export shims in `ui/components/` the way `y_button.slint` is done. No copies.
- **Tokens, not literals.** Extend `theme.slint`, for example:
  - `bar-icon-size: 16px`
  - `popover-width: 360px`
  - `popover-radius: r-xl`
  - `slider-track-h: 6px`, `slider-thumb: 18px`
  - `tile-h: 56px`
  - `osd-w: 280px`
  - `dur-osd-hold: 1500ms`
  - `ease-standard`
- **Idle CPU:** no repeating Slint timers. Data arrives from Rust on events (`pactl subscribe`, NM, UPower and BlueZ signals, inotify). An OSD hides with one single-shot timer. Animate only while something changes.
- **Small files:** one Slint file per indicator, popover or tile family, one `wire/*.rs` per backend, one `yantrik-os/src/*.rs` per system service. The mind's tools reuse the same module; no second `wpctl` wrapper.
- **Parity:** every PR adds its `describe`/`act` fields from §4.3, with read-back answers and tests in `control.rs`'s existing style.
- **Verification:** each PR adds a `tests/ui-preview` scene. Render the *VM shape* (no battery, backlight or Bluetooth) and the *laptop shape*, assert absent devices draw nothing, and assert redraws = 0 after the OSD/popover settles (the `verify-idle` pattern, `main.rs:122-145`). Attach before and after screenshots against `design/ui-audit-2026-10-01/a-*.png`.

### Phase 0 — Make the shell honest and reachable (P0, 2 PRs, S)
- **PR 0.1 — "The bar's buttons work everywhere."**
  - Move `QuickSettings`, `PowerMenu` and `ClipboardPanel` out of `desktop.slint:910-983` into app-level overlays drawn after the screens. Put them in a new `ui/components/shell_overlays.slint`, instantiated in `app.slint` beside the StatusBar.
  - Call `windows::raise_shell()` when each opens, using a `changed …-open` hook as `app-grid-opened` does (`app.slint:393-396`, `wire/app_grid.rs:39`).
  - Replace the clipboard's 200ms poll (`wire/clipboard.rs:22-30`) with the same `changed` hook.
  - Remove the `screen == 1` guards in `handle_keybind` (`system_poll.rs:497-523`).
  - Add `quick_settings.open` and `power_menu.open` to `describe`, and the actions `open_quick_settings`, `close_quick_settings`, `open_power_menu` (safe).
  - Verify: new scene `verify-bar-overlays`. Click the network and power indicators on Files, Settings and Agents and assert each overlay is drawn. Render `quick-settings-from-files.png`.
- **PR 0.2 — "No fake numbers, one mixer."**
  - Add a new `crates/yantrik-os/src/audio.rs`: a `wpctl` read/set plus a `pactl subscribe` watcher thread emitting `SystemEvent::AudioChanged`.
  - The shell's `wire/audio.rs` publishes `volume-level` and `muted` from it.
  - Delete the `amixer` path (`callbacks.rs:377-388`, `dep_check.rs:17`).
  - Point `yantrik-companion-tools/src/system.rs:198-226` and `media.rs` at the same module.
  - Add `wire/backlight.rs`: read `/sys/class/backlight`, publish `brightness-available` and `brightness-level`, and hide the slider when absent.
  - Delete the `80`/`50` defaults (`app.slint:462-463`).
  - Parity: `audio`, `brightness`, `set_volume`, `set_mute`, `set_brightness`.
  - Verify: unit tests on the `wpctl` output parser, and the VM-shape render shows no brightness slider.

### Phase 1 — The bar says the basics (P0, 4 PRs)
- **PR 1.1 — Kit: `YIndicator`, `YPopover`, `YSlider`, `YToggleTile`** (`crates/yantrik-ui-kit/slint/{indicator,popover,slider,toggle_tile}.slint`), plus the tokens.
  - `YSlider`: keyboard (←/→, PgUp/PgDn, Home/End), a step, and a value readout. Fixes the track geometry bug seen in `a-quick-settings*.png`.
  - `YPopover`: an `anchor-x` input clamped to the screen, Esc/backdrop close and a 120ms fade.
  - Verify: `verify-kit-controls` dispatches keys and pointer drags, as `verify-controls` does today.
- **PR 1.2 — Volume indicator and Sound popover.**
  - New `ui/components/bar/sound_indicator.slint` and `popovers/sound_popover.slint`.
  - The icon follows the level (off/low/mid/high/muted). Scroll changes the volume ±5, middle-click mutes, click opens the popover: slider, mute, an output list (`wpctl status` sinks) and "Sound settings".
  - Parity: `set_output`.
- **PR 1.3 — Network indicator and popover.**
  - Signal strength in 4 bars for Wi‑Fi, wired, VPN, "connected, no internet" and offline states, with SSID and IP in the tooltip.
  - The popover lists Wi‑Fi networks with signal and lock icons. Connect asks for the password inline, as Plasma does, and "Network settings" deep-links to Settings › Network.
  - Data: extend `yantrik-os/src/network.rs` (NM D-Bus, already zbus) with access points.
  - Parity: `network.wifi`, `set_wifi`, `connect_wifi` (sensitive).
- **PR 1.4 — Battery and power profile.**
  - Add `upower power-profiles-daemon` to `build-debian-iso.sh`, `user-data.yaml` and `yantrik-update` `REQUIRED_PACKAGES`.
  - Add a sysfs fallback in `battery.rs` and switch it from the 30s poll to UPower signals.
  - The indicator shows a bolt when charging, %, and time remaining in the tooltip (`TimeToEmpty`, `battery.rs:114`). The popover holds the power-profile toggle tile plus brightness when a backlight exists.
  - Parity: `battery.time_to_empty`, `power_profile`, `set_power_profile`.
  - Verify: the laptop and VM shapes, with the battery hidden on the VM.

### Phase 2 — OSDs and Quick Settings, rebuilt (P0, 3 PRs)
- **PR 2.1 — `YOsd` and the shell OSD.**
  - A pill above the taskbar at bottom-centre: icon, level bar and %, a 120ms fade and a 1.5s hold from one single-shot timer.
  - The labwc media keys change to `yos act shell volume step=+5` and the like (`rc.xml:246-275`). The shell changes the level *and* shows it, and a mind changing the volume shows the same OSD, so the person sees it happen.
  - Caps lock: bind `Caps_Lock` with `yos act shell osd what=caps`, reading `/sys/class/leds/*::capslock/brightness`.
  - Verify: `verify-osd-idle` shows the OSD, then asserts 0 redraws after 2s.
- **PR 2.2 — Quick Settings v2.**
  - Rewrite `quick_settings.slint` as a right-anchored `YPopover` built from `YToggleTile` and `YSlider`.
  - **Row 1 (the minds):** Mind mode (split), Private, Mind View.
  - **Row 2:** Wi‑Fi (split, opens the network list), Bluetooth (split, P1.5), DND, Dark style, Power mode.
  - **Sliders:** volume (chevron to output), brightness (only with a backlight).
  - **Footer:** battery %, Screenshot, Settings, Lock, Power.
  - Parity: reuse the existing actions, with no new ones.
  - Verify: `verify-quick-settings` covers keyboard navigation, the VM and laptop shapes, and a 360px width at 800/1280/1920.
- **PR 2.3 — Bluetooth.**
  - `yantrik-os/src/bluetooth.rs` (BlueZ D-Bus) with an indicator, tile and device list. Everything is hidden with no adapter.
  - The mind's `bluetooth.rs` tool moves onto it.

### Phase 3 — Today: clock, calendar, notifications, DND (P1, 2 PRs)
- **PR 3.1 — Clock → Today popover.**
  - Month grid (reuse the Calendar app's grid component; do not copy it), today's events, the five newest notifications with their actions, a **DND switch**, "While you were away" from the morning-brief data, and "All notifications".
  - The bar's DND badge becomes a moon `YIndicator` that toggles.
  - Parity: `clock_popover.open`, `open_clock`.
- **PR 3.2 — Notification polish.**
  - Fix row alignment and add app icons in `notification_center.slint`.
  - Toasts get the same card as the popover rows.
  - Super+N opens Today, and Super+Ctrl+N toggles DND (`rc.xml`).

### Phase 4 — Keyboard first (P1, 3 PRs)
- **PR 4.1 — One source of keys.**
  - Delete the dead generator (`keybinds.rs:39-133`, `ensure_labwc_config` `:252-300`, `app_context.rs:259`) and the D-Bus keybind daemon. `rc.xml` is the one source.
  - Add bindings through `yos`:
    - Super+V clipboard
    - Super+A Quick Settings
    - Super+N Today
    - Super+Escape power menu
    - Super+/ cheat sheet
    - Super+Shift+V Mind View
  - Fix the Super+D conflict. A test, like the existing Super+K test in `lens.rs`, fails if any hint names a key that `rc.xml` does not bind.
- **PR 4.2 — Cheat sheet (Super+/).**
  - A generated overlay grouped as Windows, Snap, Workspaces, Shell, Minds and Capture.
  - It is built from `rc.xml` by `build.rs`, so it cannot drift.
- **PR 4.3 — Workspace indicator.**
  - Four dots at the left of the bar: active filled, occupied outlined, click to switch.
  - Spike first: labwc's `ext-workspace-v1` support on the Debian labwc version. If it is absent, route Super+1…4 through `yos act shell go_to_workspace` so the shell knows the active one.
  - Retire the unreachable shell `WindowSwitcher` (`window_switcher.slint`) and keep labwc's Alt+Tab, themed from tokens (Phase 6).

### Phase 5 — Capture and clipboard (P1, 2 PRs)
- **PR 5.1 — Screenshot and record.**
  - Print and Super+Shift+S call `yos act shell screenshot mode=…` instead of raw `grim` (`rc.xml:234-243`), reusing `wire/screenshot.rs`.
  - A thumbnail toast offers Copy, Open and Show in Files.
  - Super+Shift+R starts `wf-recorder` (package added in 1.4) on a slurp region. A red **recording chip** sits in the bar until it is clicked to stop.
  - Parity: `recording`, `screenshot`, `start_recording`, `stop_recording` (sensitive).
- **PR 5.2 — Clipboard.**
  - Super+V, image entries (`wl-paste --list-types`), pin, clear, and "paused while a secret is pasted" (already in FreeAi).

### Phase 6 — Look: themes, lock, consistency (P1, 4 PRs)
- **PR 6.1 — Themes are places.**
  - `crates/yantrik-design-tokens/themes/*.toml` holds palette, accent, wallpaper and dark flag.
  - `wire/theme.rs` applies a theme to `ThemeOverrides` and writes `themerc` (the labwc frame and OSD), `foot.ini` colours, GTK `color-scheme` and the lock wallpaper, then runs `labwc --reconfigure`.
  - Settings › Appearance shows theme cards.
  - Parity: `theme`, `set_theme` (sensitive).
- **PR 6.2 — Lock screen.**
  - The wallpaper pre-blurred once when chosen (no live blur on the software renderer), avatar and name, clock, date.
  - Battery, network and "3 notifications" (count only), media controls, keyboard layout, and Suspend/Restart/Shut down.
  - The show-password toggle uses the kit eye icon instead of "○/◉".
- **PR 6.3 — Power menu as a popover.**
  - Anchored to the bar icon, adding Log out and Hibernate (when logind `CanHibernate`).
  - Restart and Shut down confirm with a 10s countdown and say *"2 minds are working: finish first / stop them"* (data from the mind panel).
  - Parity: `power action=…` (sensitive, deferred).
- **PR 6.4 — Consistency sweep.**
  - Set `content-has-controls` on screens 21, 27 and 28. Fix the restored-window double bar (`window_frame.slint:126,185`).
  - Make one desktop home: fold the agent-mode hero (`desktop.slint:520-880`) into `DesktopHome` with a mode flag.
  - Retire the legacy `font-*` tokens (`theme.slint:275-282`). Replace the literal px in `status_bar.slint` and `quick_settings.slint`.
  - Fix the command palette input (`command_palette.slint:80-130`), or fold it into the Lens (direction §5.7).

### Phase 7 — Delight and privacy (P2, 3 PRs)
- **PR 7.1 — Mic and camera in-use chip.**
  - Orange, naming the app, and naming **which mind** when the voice capture is the companion's. This is privacy made visible for an OS where software listens on the person's behalf.
- **PR 7.2 — Media.**
  - MPRIS in Today and on the lock screen. The Music app publishes `org.mpris.MediaPlayer2`.
- **PR 7.3 — Lens actions.**
  - "volume 30", "wifi off", "dark", "record", "theme nightfall" run as deterministic intents through `act shell` with the same grades.
  - First run gains a **Look** step (theme card picker) and a **Keys** card, and posts an Omarchy-style "Learn the keys" notification after onboarding (`onboarding.slint`).

### Architecture track — panels become panels (P1 root cause, L; in parallel from Phase 1)
- **Spike, then adopt:**
  - Run the status bar, taskbar and their popovers as `wlr-layer-shell` surfaces. The bar and taskbar sit on `top` with exclusive zones of 32 and 40. Popovers, OSD and approval cards sit on `overlay`. The desktop sits on `background`.
  - Drive them with a Slint custom `Platform` over smithay-client-toolkit that renders with the software renderer into `wl_shm` buffers. This is the same `MinimalSoftwareWindow` mechanism the preview harness already uses (`tests/ui-preview/src/main.rs:34-54`).
- **Benefits:**
  - The `<margin>` workaround goes (`rc.xml:61`).
  - Clicking the bar no longer raises the desktop over apps.
  - Popovers appear above any app without `raise_shell()`.
  - Fullscreen apps cover the bar correctly, and a second monitor gets its own bar.
- **The Phase 0–2 components are written as self-contained components with no `DesktopScreen` coupling,** so they move into layer surfaces without being rewritten.

### Order and size
| Phase | PRs | Ships | Effort |
|---|---|---|---|
| 0 | 2 | bar buttons work everywhere; real volume/brightness | S + S |
| 1 | 4 | volume, network, battery indicators + popovers | S, M, M, M |
| 2 | 3 | OSDs, Quick Settings v2, Bluetooth | M, M–L, M |
| 3 | 2 | Today popover, DND, notifications polish | M, S |
| 4 | 3 | keys, cheat sheet, workspace dots | S, S, M |
| 5 | 2 | screenshot UI, recording chip, clipboard | M, S |
| 6 | 4 | themes, lock, power popover, consistency | M–L, M, S, M |
| 7 | 3 | mic/camera chip, media, Lens actions + first run | M, M, M |
| Arch | spike + 2 | layer-shell panels | L |

---

## 7. Blocked, unverified, open

- **Live behaviour was not checked.** No VM was used (520 is in use). The click-raises-desktop claim (§1.18) and whether `upower` arrives as an apt Recommends need a live session or a laptop image.
- **The harness has no scenes for these overlays.** The `a-*.png` renders came from a scratch composition outside the repo. PR 0.1 adds real scenes.
- **The benchmark is not first-hand.** Versions and specifics in §3 come from web research on 2026-10-01 and were not checked on hardware.
- **Decisions for Pranab:**
  - CPU and MEM leave the bar (moving to the System popover and the mind panel).
  - Themes are named for the wallpapers.
  - The command palette folds into the Lens.
  - The shell's own window switcher is retired.
  - The layer-shell track starts now.

### Sources (benchmark)
- **Omarchy:** repo `basecamp/omarchy` (`dev`: waybar `config.jsonc`, `bin/omarchy-menu`, `omarchy-theme-set`, mako `core.ini`, `hyprlock.conf`, `hypridle.conf`); release notes https://github.com/basecamp/omarchy/releases/tag/v4.0.0; manual https://learn.omacom.io/2/the-omarchy-manual/53/hotkeys
- **GNOME:** https://release.gnome.org/48/ · https://release.gnome.org/49/ · https://www.omgubuntu.co.uk/2026/09/gnome-51-released
- **KDE Plasma:** https://kde.org/announcements/plasma/6/6.5.0/ · https://kde.org/announcements/plasma/6/6.7.0/
- **COSMIC:** https://github.com/pop-os/cosmic-epoch/releases · https://github.com/pop-os/cosmic-osd
- **macOS:** https://sixcolors.com/post/2025/09/macos-26-tahoe-review-power-under-glass/ · https://www.macrumors.com/how-to/do-more-with-spotlight-in-macos-tahoe/
- **Windows 11:** https://pureinfotech.com/windows-11-26h2-features/ · https://pureinfotech.com/windows-11-new-volume-mixer-quick-settings/
