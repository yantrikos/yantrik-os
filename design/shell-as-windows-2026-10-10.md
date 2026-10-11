# The shell as windows, not one fullscreen window (#733, 2026-10-10)

Pranab, 10 Oct: "When we click a few things, other windows are gone. That is also not a good experience."

## Why it happens

yantrik-ui is **one fullscreen toplevel** under labwc. It draws:
- the status bar, the dock and the desktop;
- the Lens;
- every shell screen: Settings, Files, Agents, Memory and the rest, selected by `current-screen`.

Everything that shows one of these calls `windows::raise_shell()`: `show_screen`, `open_lens`, an approval card. That raises the fullscreen window over **every** app. Nothing closed; everything got covered.

`config/labwc/rc.xml` already says the panel should really be a layer-shell surface, and keeps a `<margin top="36" bottom="48">` as a workaround. Slint's winit backend (1.17, `unstable-winit-030`) can't make layer-shell surfaces, but it can make **more than one window**. The shell already does: `AgentWindow` (agents.slint ~1292, the agents pane popped out) and `OsdWindow`.

## The shape

The person's desktop is made of windows that each behave like what they are:

| Window | What it is | labwc rule (`windowRule identifier=…`) |
|---|---|---|
| `yantrik-desktop` | the wallpaper and desktop home, fullscreen | always on bottom; skip the taskbar and the switcher; no decoration |
| `yantrik-bar` | the 36px status bar | always on top; fixed at the top edge, full width; skip the taskbar and the switcher; no decoration (the existing `<margin top>` keeps windows clear of it) |
| `yantrik-dock` | the dock, sized to its icons | always on top; fixed at the bottom centre; skip the taskbar and the switcher; no decoration (`<margin bottom>` as now) |
| `yantrik-lens` | the Lens, 440px on the right | always on top while open; no decoration; skip the switcher. It opens **beside** the person's work and covers only its own strip |
| `yantrik-settings`, `yantrik-files`, `yantrik-agents`, `yantrik-memory`… | each shell screen | **ordinary windows** with decorations: in the dock and Alt+Tab, minimisable and resizable, raised alone |

Approval cards appear in the Lens window (raised alone), never by raising a fullscreen surface.

## Phases

Each phase ships on its own and is tested on 520.

1. **The Lens becomes its own window.** This is the most frequent "everything vanished" moment: opening the chat. `open_lens` shows the Lens window instead of raising the shell. Cards raise it. The person's apps stay where they were.
2. **The shell's screens become windows,** one at a time, Settings first, then Files, Agents and Memory. `show_screen(n)` opens or raises that window. The `current-screen` switch inside the fullscreen window goes away for each screen as it moves. The `app.describe` / `app.act` surfaces keep their names, so minds see no change.
3. **The bar and dock become their own small windows,** and the remaining fullscreen window becomes `yantrik-desktop`, always on bottom. After this phase nothing in the shell can cover an app by being raised.
4. **Clean-up:** remove `raise_shell` from every path that only needed it because the shell was one window. Remove the `<margin>` note's "workaround" wording if labwc's rules now hold the space.

## Risks to check in phase 1

- labwc 0.8.3 window-rule actions for always-on-top, always-on-bottom and positioning (`man labwc-config`). Check them before relying on them.
- Focus: the Lens taking keyboard focus when it opens, and giving it back on close (`toplevel_watch` knows the previous window).
- Mind View routing (`mind_view::route_now`) and the window registry treat the new shell windows as the shell's own, never as apps a mind opened.
- Memory: a Slint window per screen on the software renderer. Measure on the 2012 Mac mini.
