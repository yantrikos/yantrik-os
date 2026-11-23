# Yantrik's labwc

Debian's labwc 0.8.3-1 with two patches:

- **window switcher renderer**, so Alt+Tab can be drawn by `yantrik-switcher` while labwc keeps the keys (design: `design/alt-tab-deep-2026-10-18.md`, #867; compositor work: #849);
- **window locator**, so the Y can tell which window the person pointed at and see it (design: `design/y-anywhere-2026-11-22.md`).

| File | What it is |
|---|---|
| `labwc-window-switcher-renderer-unstable-v1.xml` | The switcher protocol. The one copy: `crates/yantrik-switcher` generates its bindings from it. |
| `yantrik-window-locator-unstable-v1.xml` | The locator protocol. The patch carries the same bytes. |
| `debian-patches/series` | The order the patches apply in. `build-deb.sh` fails if a `.patch` here is not listed exactly once. |
| `debian-patches/yantrik-window-switcher-renderer.patch` | The switcher change, and the launch and capture code both patches use (18 files, ~1,400 lines). |
| `debian-patches/yantrik-window-locator.patch` | The locator change, on top of the first (11 files, ~1,100 lines). |
| `build-deb.sh` | Builds `labwc_0.8.3-1yantrik3_amd64.deb` (`REVISION` in the script; raise it when a patch changes) in a Debian 13 system with deb-src. CI runs it for every published build (`iso.yml`), and the bundle ships the .deb. |
| `test/switcher-probe.c` | A renderer that only logs. Used to test the switcher protocol without the real UI. |
| `test/locator-probe.c` | A locator that only logs, driven from a command file. Used to test the locator protocol without the real UI. |

## Shared by both patches

- `launch-client.c`: labwc launches a configured command itself on a socket it created (`WAYLAND_SOCKET`). The `wl_client` on labwc's end is that program by construction, whatever its pid or name, so a global can be offered to it alone (the global filter in `server.c`). Each user keeps its own relaunch policy (at once when it exits, at most once every 5 s).
- `scene-capture.c`: a picture of one window, rendered offscreen from the window's own scene subtree (never what overlaps it, never its decorations), averaged down 2×2 when it shrinks by 2 or more, handed over in a sealed memory file.

Both were moved out of the first patch unchanged when the locator came; every other hunk of that patch is byte-for-byte the reviewed one.

## What the switcher patch does

- `<windowSwitcher renderer="command">` (read when labwc starts; adding or removing it takes a labwc restart): labwc launches that command itself and offers `zlabwc_window_switcher_renderer_v1` to that one client only. No other client can bind it.
- labwc keeps the session: the chord, the order (`get_next_cycle_view`), the selection and the commit when Alt comes up (`end_cycling`). The renderer is told `session_start`, the windows and `windows_done` (again only when the list changes), `selected`, `key` (text keys no keybind took: every binding still runs during a session) and `ended`. It may `ack` a session and ask to `select` a window in it. It can never commit, focus or raise.
- The renderer acks a session it is going to draw. With no ack within 300 ms (the renderer hung before acking, or died), labwc draws its own OSD for that session, exactly as stock labwc does, and ignores a later ack. A renderer that dies after its ack is covered too: labwc falls back when its connection closes, and launches it again at once (at most once every 5 s). labwc does not tell the renderer that it fell back, so `yantrik-switcher` acks when its stack goes up, before the first frame, and leaves a session alone that is too old to ack in time (`crates/yantrik-switcher/src/handover.rs`). Acking after the first frame put both on the screen whenever that frame took longer than the wait. What this gives up: a renderer that is alive after its ack but gets no frame on the screen shows nothing for that session, where labwc's list used to come up; Alt coming up still switches. A session over before the renderer draws (a quick Alt+Tab) shows nothing.
- A stock labwc ignores the `renderer` attribute, so `config/labwc/rc.xml` is the same for both.

## What the locator patch does

- `<yantrik locator="/absolute/program" private="app ids">` (read once, when labwc starts; a later `--reconfigure` does not change it): labwc launches that program itself and offers `zyantrik_window_locator_v1` to that one client only.
- labwc starts it only when (otherwise it logs why and leaves the locator off):
  - every rc.xml it read (`-c`, or the first in the search path, or all of them with `mergeConfig`), and the directory each is in, are root's and writable by no one else, so nothing the person's own programs can write picks the program;
  - the program is an absolute path to a file that is root's alone, in a directory that is root's alone;
  - `private` is set (fail closed: a locator with no private list does not start).
- The person's gesture is checked here, not left to the client:
  - `at(x, y)` is answered with a window only while a pointer button is held and the point is within 8 px of the cursor (the drag);
  - `focused()` only within 2 s after the `YantrikLocate` action ran (bound to Super+Y), once; the action also sends the client `armed`;
  - otherwise the answer is `none` (`no_gesture`).
- `at` names the topmost window drawn at the point (decorations and popups count) on the workspace the person is on. The client's own surfaces are looked through, so its drag overlay does not hide the window under it. Anything else drawn on top there (another client's panel, dock, popup or overlay, even one that lets clicks through; a menu; the lock screen) answers `none`.
- `capture(id, max_w, max_h)`: a picture of the window (16..2048 a side, never larger than the window), only of the window the most recent answer named, within 2 s, once per answer, so it inherits the gesture. Every picture is logged by labwc (app id and size, no title), at the level it writes by default.
- A window whose app id is on `private` is named with an empty title and the `private` flag, and never pictured.
- At most 120 answers and 2 pictures in each one-second window. Nothing is answered while the session is locked, and `YantrikLocate` does nothing then.
- What it is not: labwc offers virtual pointer and keyboard (and screencopy) to every client outside a security context, so a program running as the person can synthesize both gestures. The gesture rule holds the launched client to the person's action; it is not a boundary against code running as the person (#414, #476).

## Tested

### Switcher (19 Oct 2026, VM 520, labwc 0.8.3)

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

### Locator, round 1 (23 Nov 2026, VM 542, nested in the desktop session)

Two `foot` windows, A (red) mostly under B (blue). These ran before the gesture rule existed; the hit test, picture and limit results still hold for the code below.

- a probe started by hand is not offered the global; `at` names B where B overlaps A, A on A's title bar and where only A shows, and `none` on empty desktop;
- A's picture is 100% red and 0% blue, though B covers most of A; a picture asked bigger than the window comes back at the window's size; the file is sealed (`0xf`) and cannot be mapped writable;
- a second picture on the same answer, and one 2.5 s after it, are refused (`not_located`); a third picture in one second is refused (`throttled`); 120 answers a second are given and the rest answered `throttled`;
- a killed probe is launched again within 1.5 s.
- With `test/switcher-probe.c` on the shared code: a session is told and acked and its selection follows; Escape cancels it; with `/tmp/switcher-probe-capture` the picture of A is 240×170, byte mean 111.8 (pure `#c00000`); a killed renderer is launched again.

### Locator, round 2: gesture, trust, private (23 Nov 2026, VM 542, nested)

- Not started, with the reason in labwc's log, when: the rc.xml is the person's; `private` is missing; the program is the person's; the program is not an absolute path.
- Started from a root-owned rc.xml with `private="probe-p, gcr-prompter"` and `W-y` → `YantrikLocate`, with A, B and a green window P (`probe-p`):
  - `focused` with no Super+Y: `no_gesture`. After Super+Y: `armed`, then the window; a second `focused`: `no_gesture`. Super+Y, 2.5 s, `focused`: `no_gesture`;
  - P is named with an empty title and `flags=1`; its picture is refused (`private`);
  - `at` with no button held: `no_gesture`. Button held (virtual pointer) at 600,400: the window at the cursor, and 5 px off; 40 px off: `no_gesture`. After release: `no_gesture`;
  - pictures follow a held-button answer, and each is in labwc's log (`picture of window 2 (app_id probe-b), 300x213`);
  - with the probe's own full-screen overlay up, `at` still names the window; with another client's full-screen overlay that lets clicks through, `at` answers `none`; after it exits, the window again;
  - locked: `at`, `focused` and `capture` are refused (`locked`), and Super+Y sends no `armed`.

## Not done yet

- The locator's client (`yantrik-y`) and, in `config/labwc/rc.xml`, `<yantrik locator="..." private="...">` and `W-y` → `YantrikLocate`: Y Anywhere phase 2. Until then nothing sets `<yantrik locator="">`, so the locator is never started. The `private` list then has to carry the OS's own secret-taking windows (the polkit agent, pinentry, vault unlock, sign-in), and the client has to refuse to locate or capture while private mode is on.
- A machine check of Alt+Tab's hold-and-release commit on a real session (the nested runs could only cancel).
- Upstreaming: a generic version of the switcher hook is to be proposed to labwc (decided 18 Oct, #849).
