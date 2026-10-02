# Review of PR #582: a mind's apps and commands appear in Mind View, and open_app says whether the window appeared

Branch `origin/fix/mind-apps-in-mind-view` (2 commits, 8 files, +670/-24). Review only: nothing was modified, and no tests were run.
The author's report says the full `cargo test -p yantrik-ui --bin yantrik-ui` could not be built (OOM). I did not build either, because the `yantrik-ui-slint` build is too heavy. Everything below comes from reading code with `git show` / `git archive` of the PR branch.

## Summary

The PR does three things:

1. `open_app` now waits up to 8 s, off the UI thread through `answer_later`, for the window to appear, and answers with where it is or an error.
2. A mind's launch is refused, not sent to the person's desktop, when Mind View is down (`mind_view::route`, `wire/dock.rs::spawn_launch`).
3. `agent_run` mirrors the command and its PTY bytes into a log and opens a `foot` viewer on Mind View's display.

The direction is right and the threading is mostly right: `answer_later` is used, the wlrctl call has a 2 s timeout, and the viewer's environment strips DISPLAY. But as a security fix it is incomplete:

- A mind's launch can still land on the person's desktop through the inherited `DISPLAY`, through the "already running on the desktop" route (a second process is spawned there), and through singleton browsers.
- The two other mind launch paths, the companion's browser tool and its `open_url` tool, still act on the person's desktop.
- `open_app` can still answer falsely in both directions. The error text asserts "Nothing was opened on the person's desktop" without checking, and `lists_app` is a loose substring match that gives both false positives and false negatives.
- Mind-originated `files_open` now fails silently where it used to fall back.
- The viewer mirror lets a mind spoof or wipe what the person sees on "the actual desk".
- The tests do not cover the real wiring, and nothing tests `DISPLAY` for launched apps.

## Paths examined

| # | Path (mind or synthetic input) | Status on PR branch |
|---|---|---|
| 1 | `shell.open_app` -> `invoke_launch_app` -> `spawn_launch` -> `route_now` -> Mind View (`control.rs:1066-1135`, `wire/dock.rs:1106-1140`) | Covered. Routes to Mind View and answers after a wait. See B1, S1, S2, S3 for holes. |
| 2 | Same path, Mind View down (`ensure()` Err) | Closed: launch refused and a failure recorded (`dock.rs:1129-1138`). Side effect: S5, and the answer text in S2. |
| 3 | `launch()` environment for the Mind View seat (`dock.rs:1146-1170`) | **Not closed.** The person's `DISPLAY` is inherited when the seat has no X11. See B1. |
| 4 | `open_app` when the app is "already running on the desktop" (`mind_view.rs::route`, `open_on_desktop`) | **Not closed.** A second process is spawned on the person's desktop. See B2. |
| 5 | `open_app` for Screen/Settings/Launchpad routes (`control.rs:1082-1110`) | Not covered. The shell switches the person's own screen and raises the shell. Graded `standard`. By design but unmentioned. See N6. |
| 6 | `open_app browser` / `blender` / catalogue apps, singleton apps | Partly covered. Singleton forwarding to the person's Chromium/Firefox is not handled (S3). The blender DISPLAY gate (`dock.rs:~925`) reads the person's `$DISPLAY` (B1). |
| 7 | `agent_run` (control_agent_terminal.rs) | Runs in the agent PTY with `env_clear` and no display, so it is closed for GUI launches except where the command itself sets `WAYLAND_DISPLAY`. The new mirror viewer is on Mind View's display (the test asserts it). See S4, S6, S7, S8, S9. |
| 8 | `agent_job` / `agent_input` / `agent_kill` | Unchanged. Output mirrored through `on_output`. |
| 9 | Terminal app `run` / `send_input` (app surface) | Unchanged. Types into whatever Terminal window is open. If the person's own Terminal is open (path 4), the mind drives the person's shell. Disclosed in the answer's `where`, graded by the app. See N5. |
| 10 | `files_open` -> `open_with::launch` -> `spawn_app_with_args` -> `spawn_launch` | Routed through the same `route_now`, but it now fails silently when Mind View is down while answering `opened`. See S5. |
| 11 | `wire/files.rs` "Open Terminal here" | Person's UI action. Not mind-reachable. |
| 12 | Companion tool `browser.rs` LaunchBrowser, `display_for_mind` (`mind_view.rs:536`) | **Not closed** (acknowledged "not fixed" in the author's report). Falls back to the person's `WAYLAND_DISPLAY`. Headless uses `wayland-0` (fine, draws nothing). The CDP port 9222 is shared with the person's Chromium. See B3. |
| 13 | Companion tool `open_url` (`yantrik-companion-tools/src/desktop.rs:49`) | **Not closed.** Runs `xdg-open` in the shell's own environment and opens the person's default browser ("outside session"). Graded `Standard`. See B3. |
| 14 | Companion `focus_window` / `close_window` / `list_windows` (via `shell.focus_window` etc.) | By design acts on the person's windows. Graded on the shell. Not changed. |
| 15 | `send_message` -> chat -> `invoke_send_message` | Refused for agents. Mind-door callers reach chat, not the Lens launcher. `LensAction::Launch` (`wire/lens.rs:240`, plain `Command::new`, person's env) is not reachable from it as far as I could read. Unverified end to end. |
| 16 | `open_lens`, `show_screen`, `show_app` | Show the shell or raise windows on the person's desktop. By design, unchanged. |
| 17 | Dock / pins / app grid / notification actions (`wire/app_grid.rs`, `wire/notifications.rs:346,430`) | Person's clicks, with no caller, so the route is Person. `pin_app` (sensitive) does not launch. Not mind-launchable, except that a mind could post a notification whose action opens an app, which needs the person's click. |
| 18 | MCP (`yos-mcp`, `crates/yantrik-mcp`) and the mind door | Funnel into the same socket handlers, so paths 1-10 apply. `requester_now` classifies by `mind_account`/token/pid. |
| 19 | Deep links / `open_apart` / `open_url::open` | Reachable only from person's UI (`wire/lens.rs:268`, `wire/free_ai/mod.rs:139,152`). Not mind-reachable. |
| 20 | `start_service` (sensitive) | Systemd user services. Not a window. Unchanged. |
| 21 | Held-act replay after the person's Allow (`call_was_granted`) | If a replayed act runs without the mind's `caller()` in scope, `requester_now` returns Person and the app opens on the desktop. Not verified. Needs a test (S10). |

## Findings

### BLOCKER

**B1. A mind's app can still draw on the person's desktop: `DISPLAY` is inherited. (`wire/dock.rs:1146-1170`, `mind_view.rs:111-117`)**
`launch()` takes the shell's full environment and overlays only `Seat::env()`. `Seat::env()` adds `DISPLAY` only when Mind View has its own Xwayland (`seat.x11 = Some`). `session_env()` additionally sets `DISPLAY` to the person's `x_display()` when it is not set. The nested labwc is started with `env_remove("DISPLAY")`, and the new viewer strips `DISPLAY` when `seat.x11.is_none()` (`mind_agent_terminal.rs:72-75`). `launch()` has no equivalent strip.

- Scenario: Mind View runs with no Xwayland (the first line of `seat_script` / `parse_seat` allows `x11 = None`). A mind opens an X11-only app. Debian's Blender is X11-only, as the code's own comment says, and `blender_display` even reads the person's `$DISPLAY`. Electron/xcb apps behave the same. The child inherits the person's `DISPLAY` and appears on the person's desktop. `GDK_BACKEND=wayland,x11` and `QT_QPA_PLATFORM=wayland;xcb` also fall back to X11 when the Wayland socket is gone.
- Related TOCTOU: `ensure()` returns a seat, the person closes Mind View (or it dies), and the spawn finds `WAYLAND_DISPLAY=wayland-1` dead. GTK/Qt then fall back to the person's X display. Not a window in the wrong place for pure-Wayland apps, but wrong for everything else.
- Fix: in `launch()`, when `seat.is_some()`, `env_remove("DISPLAY")` and `env_remove("WAYLAND_SOCKET")` before applying `seat.env()`. Also set `GDK_BACKEND`/`QT_QPA_PLATFORM` consistently with what Mind View offers, and re-check the seat's socket exists immediately before spawn. For Blender in Mind View, require `seat.x11`, not the person's `$DISPLAY`. Add a test that builds the `Command` (extract an env builder) for `seat.x11 == None` and asserts `DISPLAY` is absent.

**B2. "Already running on the desktop" spawns a new process on the person's desktop for a mind. (`mind_view.rs:204-226`, `wire/dock.rs:1111-1115`)**
`route()` returns `mind_view: None` when `is_running(app_id) && !is_mind_view_app`. `spawn_launch` then calls `launch(..., None seat, raise=false)`, a real spawn with the person's environment. The comment assumes every app is single-instance. That holds only for the shell's own Slint apps. For catalogue apps, `foot`, `chromium` etc., the second process opens a new window on the person's desktop, on a mind's request.
- Scenario: the person has `foot` or Firefox open. The mind calls `open_app foot`. A new terminal appears on the desktop. The answer then reads "window appeared, where: the person's desktop, note: already open ... used where it is, not raised", which is false because a second window was created.
- Fix: for a mind, do not spawn at all in this case. Answer from the registry ("already open on the person's desktop; not launched, not raised") or launch in Mind View. If the app is a singleton, the answer may say that handover can occur. A test must pin that `route` returns a no-spawn outcome, not `mind_view: None`.

**B3. Other mind launch paths still act on the person's desktop and are not covered or mentioned. (`yantrik-companion-tools/src/desktop.rs:49`, `browser.rs:~650-690`, `mind_view.rs:536-546`)**
The PR title and the report say "a mind's apps never go to the person's desktop", but:
- The companion `open_url` runs `xdg-open` directly and opens the person's default browser. It is graded `Standard`.
- `display_for_mind()` still falls back to the person's display when Mind View is down or the setting is off. The author lists this as "not fixed".
- The companion browser attaches to whatever answers CDP port 9222 (`is_browser_running()`), which is the person's own Chromium if it is running with debugging, so the mind then drives the person's tabs.

For a PR whose stated goal is "a mind must never act on the person's desktop", these leave the same hole open one tool away. At minimum the PR description must say so and a follow-up must be ticketed. Preferably fix `display_for_mind` in the same PR (return an error, not the person's display, as with `spawn_launch`) and route `open_url` through the same decision.

### SHOULD-FIX

**S1. `lists_app` is a substring match and gives false positives and false negatives. (`mind_view.rs` `lists_app`, `mind_landing.rs:83-90`)**
- It checks `line.contains(id) || line.contains(display_name)` over lowercase `wlrctl toplevel list` lines.
- False positive: the new viewer window is titled `<agent> terminal`, so after any `agent_run` the id `terminal` matches it and `open_app terminal` answers "window appeared in Mind View" even if the Terminal app never drew. Similarly `files`, `editor`, `notes` match any title containing those words.
- False negative: for third-party apps the id (`browser`, `org.mozilla.firefox`, `blender`) does not appear in lines like `chromium: New Tab - Chromium`. The mind then gets an error after 8 s while the window is there, retries, and the mind is told "no window".
- The author admits the match against real labwc output is unverified.
- Fix: match on the structured `app_id` field (`wlrctl toplevel list` prints `app_id: title`) against the app's declared `StartupWMClass`/binary/app id mapping, and exclude the agent viewer. Add tests with real-shaped lines for both cases.

**S2. The error text asserts "Nothing was opened on the person's desktop" without checking. (`mind_landing.rs:120-123`)**
`answer()` always appends this sentence on `Err`. It is false when the setting is off (apps go to the desktop by design), for the person's own calls, for the B2 route, for singleton forwarding (S3), and for slow starts: a cold Chromium that appears at 9 s on the person's desktop is reported as "no window; nothing opened on the desktop". `Seen::Failed` also prints `exited with {status}` where `status` is the free text "Mind View is not available (...)" for the new refusal, giving "exited with Mind View is not available ... after 0 ms".
- Fix: say only what was observed ("no window was listed in Mind View within 8 s"), and add the desktop sentence only on the refusal path. Pass the failure text through as its own variant. Consider a longer budget for cold starts, or return `running: true`/"still starting" instead of an error after 8 s.

**S3. Singleton apps launched in Mind View can open on the person's desktop. (`wire/dock.rs launch`)**
Chromium and Firefox use the profile's singleton. With the person's own browser running under the same profile, the Mind View launch hands over to the person's process and a window appears on the person's desktop. The registry might not know it is running (B2's `is_running` is registry-based). The mind then gets "no window appeared; nothing opened on the person's desktop" (S2), which is wrong.
- Fix: launch third-party singleton browsers with a separate profile/`--user-data-dir` when started for a mind, or detect an existing process and refuse.

**S4. The mirrored terminal can be spoofed and wiped by the mind it is supposed to show. (`mind_agent_terminal.rs:111`, `:96-122`)**
The command is written raw to the log: `\x1b[1;36m$ {command}\x1b[0m\r\n`. A command containing `\r`, `\x1b[2K`, `\x1b[3J`, `\x1bc` or OSC sequences makes the Mind View terminal show something other than what ran. Output bytes are raw PTY bytes, so `clear`, `\x1bc` or `printf '\e[3J'` also erases the history in the person's view while the Agents card keeps it. Mind View is the spec's "actual desk" (the person's oversight surface).
- Fix: render the command line with control characters escaped (`^[`, `\r`), and filter or neutralise destructive sequences in the viewer (or show it through `cat -v`-like or a vt100-rendered snapshot). Add a test with `\r`/ESC in the command.

**S5. Silent failure for other mind-originated launches: `files_open` and others. (`control_files.rs:133-156`, `wire/dock.rs:1129`)**
With Mind View unavailable the launch is refused in a worker and recorded, but `files_open` answers `{"opened": name, "app": app, ...}` immediately. Before this PR the file opened on the desktop. Now a mind is told "opened" while nothing happened. This is exactly the lie the PR fixes for `open_app`.
- Fix: share the wait/answer helper (`mind_landing`) with `files_open`, or have it answer `launching` plus `failed_launches` honestly. Add a test.

**S6. A transient Mind View start failure disables minds' apps for the whole session. (`mind_view.rs` `ensure`: `state.unavailable`, "Cleared only by a restart")**
Previously the desktop fallback masked this; now a slow first start (`START_BUDGET` 5 s on a loaded VM) permanently refuses every mind launch, including `agent_run`'s viewer, until a shell restart. Fix: retry with backoff (for example after 30 s) or after the person changes the setting.

**S7. `agent_run` blocks its answer behind `show()` and can raise Mind View over a waiting approval card. (`control_agent_terminal.rs:518-524`, `mind_agent_terminal.rs:145-190`)**
- `show()` runs before `jobs().job(wait)`: `ensure()` (up to 5 s), `foot` start, and a 4 s window wait. The command is not delayed, but the answer is, by up to about 9 s on every command when the title never appears in the list, and `wait` starts counting after that.
- Starting Mind View maps a window on the person's desktop (minimised afterwards by `step_aside_when_shown`) without `crate::card_watch::hold_windows("agent_run")`. The playbook's rule is that an action that brings a window over the shell must call it first, and a source-scan test enforces it for `open_app`. Fix: run `show` on its own worker (the answer's `shown_in` can then say "starting"), and call `hold_windows` before `ensure()` or skip the viewer while a card is waiting.

**S8. The log is unbounded during one command and is written inefficiently. (`mind_agent_terminal.rs:21-23,96-132`)**
`LOG_CAP` is checked only in `begin()`. One long command (`yes`, a build log) can fill the `$XDG_RUNTIME_DIR` tmpfs. `mirror()` opens the file, chmods and writes on every PTY chunk on the job's reader thread. Fix: keep one append handle per agent, cap bytes per run, and set mode 0600 at creation with `OpenOptions::mode`. Output is retained on disk (including secrets printed by commands) until the next oversized `begin()`; delete the log when the agent ends.

**S9. Agent-id collisions and stale state. (`mind_agent_terminal.rs:42-49,112,118`)**
`file_stem` maps `pi.1` and `pi_1` to the same file and window title, so two agents' commands interleave in one viewer and the title probe can match the other agent's window. A `logs` entry is never removed, so after the setting is turned off `mirror()` keeps writing for that agent while `begin()` returns None and the answer says "nowhere on screen". `begin()` also writes the `$ command` line before `jobs().start()`, leaving a dangling line if start is refused. The `shown_note` for the `None` case blames the setting even when the file could not be written.

**S10. Tests do not cover the real wiring and would not have caught B1, B2 or S1.**
- `the_mind_path_launches_with_mind_views_display...` is a source scan (`!worker.contains("None, true)")`). It would miss a `launch(..., None, false)` or any rewording, and it never builds a command or checks `DISPLAY`.
- `mind_landing` tests cover only the pure wait loop and `answer()`. `probe_for`, `lists_app`, the handler's use of `answer_later`, and the `open_app` wait gating on `answer.get("launching")` are untested.
- There is no test for: a mind's launch when the app is running on the desktop (B2); `requester_now` at the point `route_now` runs after a held-act replay (path 21); `describe shell` `your_windows_are` and `agent_terminal` text; `agent_run`'s answer fields; `files_open` with Mind View down.
- The existing source-scan `opening_the_launcher_raises_the_shell...` and `showing_and_reading_stay_below_the_line` still apply. They should be extended with a scan that `open_app`'s handler calls `answer_later` and that no process call sits outside the closure.
- Add a test that the Mind View `Seat` environment never includes the person's `DISPLAY`, and a launch-env builder unit test.

### NIT

- N1. `mind_view.rs` (`ensure`): the warning still reads "minds' apps open on the desktop"; the module comment in `dock.rs` says the same. Both are now false.
- N2. `for_describe()` `your_windows_are`: "in Mind View, a window of its own on the person's desktop; never on their desk" contradicts itself. Say "inside Mind View (one window on the person's screen), not as windows of their own".
- N3. `mind_landing::BUDGET` 8 s and the `STEP` constant (250 ms) are duplicated as literals in `control.rs:1129-1130`; `STEP` is unused.
- N4. `launcher_id_in` re-implements the id the dock registers; it works for the routes and catalogue today but is a second derivation to keep in step with `Resolved::Catalogue::id`.
- N5. The "drives the person's own window" case for a running Terminal (path 9) is honest in the answer's `where` but worth saying in the report and spec.
- N6. `open_app` of a shell screen (files, settings, launchpad) still switches the person's screen and raises the shell for a mind. The report states "no action ... changed", which is true, but the claim "never on the person's desk" is conditional. Mention it.
- N7. The inline fallback `.or_else(|wait| wait())` blocks 8 s if the handler is ever called directly on the UI thread (tests only today).
- N8. `agent_run`'s description string has a long line; match nearby style. `review/fix-mind-apps-report.md` adds a file under `review/` that no other change does; check that directory is expected.

### Checked and fine

- No grade changed. `open_app` stays `standard` and `agent_run` stays `sensitive`. `hold_windows("open_app")` is still called before the launch.
- No new command-injection surface: the viewer argv is fixed (`foot -T <sanitised title> -e tail -n +1 -F <path>`), the path comes from a sanitised stem, and `foot` is started with no shell.
- `answer_later` is used for the wait. The wlrctl call is on its own thread with a 2 s `recv_timeout`. The closure takes everything it needs by move and does not read `caller()` in the worker.
- The viewer's environment keeps the person's display out (`WAYLAND_DISPLAY` set to Mind View, `DISPLAY` removed without X11, `WAYLAND_SOCKET` removed).
- `route_now` runs on the UI thread inside `invoke_launch_app`, so `caller()` is in scope. The `Requester` classification (mind account first) is sound.
- The setting `minds_open_in_mind_view` is pointer-only (`control_approvals.rs:1476`), so a mind cannot turn Mind View off to reach the desktop.

## Files likely to conflict with other PRs

- `crates/yantrik-ui/src/control.rs` (`open_app` handler region): `ui/agents-workroom`, `ui/alt-tab-switcher`, `ui/grounded-dock` all touch it.
- `crates/yantrik-ui/src/main.rs` (the `mod` list): all four UI PRs add modules.
- `crates/yantrik-ui/src/wire/dock.rs` and `wire/dock_bar.rs` (`ui/grounded-dock`): the launch helper and dock wiring.
- `crates/yantrik-ui/src/mind_view.rs`: the Alt+Tab spec's "Hermes's desk" scope will want the same display/window-list helpers.
- `crates/yantrik-ui/src/control_agent_terminal.rs`: the workroom PR reads `agent_run` output and Agents cards.
- `docs/app-control.md`: two PRs edit it. This PR changes `describe shell` fields and `agent_run`'s answer but does not update it.

Verdict: MERGE AFTER FIXES — B1, B2 and B3 (or an explicit scoped-out statement for B3) must be fixed or disclosed first, together with S1, S2 and S5 so that `open_app` and `files_open` answer the truth.
