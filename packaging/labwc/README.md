# Yantrik's labwc

Debian's labwc 0.8.3-1 with one patch, so Alt+Tab can be drawn by `yantrik-switcher` while labwc keeps the keys (design: `design/alt-tab-deep-2026-10-18.md`, #867; compositor work: #849).

| File | What it is |
|---|---|
| `labwc-window-switcher-renderer-unstable-v1.xml` | The protocol. The one copy: `crates/yantrik-switcher` generates its bindings from it. |
| `debian-patches/yantrik-window-switcher-renderer.patch` | The labwc change (14 files, ~1,180 lines). |
| `build-deb.sh` | Builds `labwc_0.8.3-1yantrik2_amd64.deb` (`REVISION` in the script; raise it when the patch changes) in a Debian 13 system with deb-src. |
| `test/switcher-probe.c` | A renderer that only logs. Used to test the protocol without the real UI. |

## What the patch does

- `<windowSwitcher renderer="command">` (read when labwc starts; adding or removing it takes a labwc restart): labwc launches that command itself, on a socket it created (`WAYLAND_SOCKET`), and offers `zlabwc_window_switcher_renderer_v1` to that one `wl_client` only (the global filter in `server.c`). No other client can bind it, whatever its pid or name.
- labwc keeps the session: the chord, the order (`get_next_cycle_view`), the selection and the commit when Alt comes up (`end_cycling`). The renderer is told `session_start`, the windows and `windows_done` (again only when the list changes), `selected`, `key` (text keys no keybind took: every binding still runs during a session) and `ended`. It may `ack` a session and ask to `select` a window in it. It can never commit, focus or raise.
- The renderer acks a session once it has drawn it. If it has not within 300 ms (it failed to draw, hung or died), labwc draws its own OSD for that session, exactly as stock labwc does. A session over before the renderer draws (a quick Alt+Tab) shows nothing. A dead renderer is launched again on the next session, at most once every 5 s.
- A stock labwc ignores the `renderer` attribute, so `config/labwc/rc.xml` is the same for both.

## Tested (19 Oct 2026, VM 520, labwc 0.8.3)

- With `test/switcher-probe.c`:
  - only the launched renderer is offered the global; a client started by hand is not;
  - hold and release commits;
  - a quick tap commits;
  - Escape cancels;
  - a typed key is forwarded and `select` is honoured;
  - with no ack, labwc's OSD appears after 200 ms;
  - a killed renderer is relaunched on the next session.
- With `yantrik-switcher`:
  - the first frame is drawn 121 ms into the session;
  - a quick tap draws nothing;
  - Tab moves the stack.

## Not done yet

- Shipping the .deb in the ISO and the update bundle (a CI job on `debian:trixie` runs `build-deb.sh`).
- Upstreaming: a generic version of this hook is to be proposed to labwc (decided 18 Oct, #849).
