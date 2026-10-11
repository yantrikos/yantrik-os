# Mind apps and commands in Mind View (2 Oct 2026)

Security review: changes where a mind's apps and commands run.

## What was found

1. `open_app` answered `launching` (read by the os_act wrapper as `accepted: True`) the moment the
   dock handed the launch to a worker. For a mind that worker first starts Mind View, then spawns
   the app on `wayland-1`. If the spawn failed, the app died, or Mind View was down, the answer had
   long been sent. Nothing read the window back.
2. When Mind View could not start, `spawn_launch` fell back to the person's desktop. That is the
   opposite of the design ("apps a mind opens appear in Mind View, never on the person's desk").
3. `agent_run` ran in the shell's own PTY (`yantrik-agent-terminal`) with a built environment that
   has no `WAYLAND_DISPLAY`, by design. Its output went to the Agents screen card only, never to
   Mind View, so Mind View stayed empty while a mind worked. The Terminal app's `run` types into
   the app's own shell, so it is visible only if that window is open in Mind View, which (1) hid.
4. `describe shell` did not say where a mind's windows are.

The routing itself (`mind_view::route_now`, `Seat::env`) was already right and is on by default
(`minds_open_in_mind_view: true`); no setting needed flipping.

## What changed

- `mind_landing.rs` (new): `open_app` now waits, off the UI thread via `answer_later`, up to 8 s for
  the window: Mind View's compositor (`wlrctl toplevel list` on its display) for an app launched
  there, the desktop's window list otherwise, or a recorded launch failure. It answers
  `window: appeared, where: Mind View` or an error saying no window appeared and that nothing was
  opened on the person's desktop. The probe is injectable (`wait_for_window`).
- `mind_view::route`: a mind's launch no longer falls back to the person's desktop when Mind View is
  down; `wire/dock.rs` records the refusal as a launch failure so the answer says why.
- `mind_agent_terminal.rs` (new): `agent_run` for a mind mirrors the command line and its PTY bytes
  to `$XDG_RUNTIME_DIR/yantrik/agent-terminal/<agent>.log` (0600) and opens a `foot` titled
  `<agent> terminal` on Mind View's display running `tail -F` on it. The command still runs once, in
  the agent terminal. The answer carries `shown_in` and, when it is not shown, why.
- `describe shell` `mind_view` gains `your_windows_are` and `agent_terminal`; `agent_run`'s
  description says where output is shown.

## Security notes

- No action, grade, approval or argument added or changed. `open_app` keeps
  `crate::card_watch::hold_windows("open_app")?` before the launch.
- The mirrored log is under the 0700 shell socket directory, file mode 0600, named from a sanitised
  agent id. The viewer environment strips `WAYLAND_SOCKET`, `SLINT_FULLSCREEN` and, when Mind View
  has no Xwayland, the person's `DISPLAY`.
- Behaviour change to review: a mind's app launch now fails when Mind View is unavailable instead of
  opening on the person's desktop.

## Tests

`cargo check -p yantrik-ui --bin yantrik-ui --tests` passes. `cargo test -p yantrik-ui --bin yantrik-ui` could NOT be run: building `yantrik-ui-slint` was killed by the OOM killer twice (also with `-j1`, no debuginfo). The four `mind_landing` tests were compiled and run standalone (4 passed); the others compile but were not run. New: `mind_landing::tests` (including
`open_app_answers_the_truth_when_no_window_appears`), `mind_agent_terminal::tests` (viewer on Mind
View's display only), `mind_view::tests::the_mind_path_launches_with_mind_views_display_and_never_the_persons`,
and the updated route test.

## Not done / not verified

- Nothing was run on a real labwc: whether `wlrctl toplevel list` on Mind View's display includes the
  Slint apps by id or title, and whether `foot` draws there, is unverified.
- Not fixed: `mind_view::display_for_mind` (the companion's browser tools) still falls back to the
  person's display when Mind View is down.
- The Terminal app's `run` is unchanged; it types into the app's shell and is visible when the
  Terminal is open in Mind View, which `open_app` now confirms.
