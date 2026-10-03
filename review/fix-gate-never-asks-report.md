# fix/gate-never-asks — report

**Security-sensitive:** approvals and the permission gate (`approvals.rs`, `control_approvals.rs`, `control.rs`), and the `yos-mcp` bridge.

## What happened
The Mind gate (`tests/arena/harness_arena.py`) runs scored tasks on a machine where a person is logged in. A task's `shell.agent_run` needed approval and put a card in their live session. Nobody answered; it expired after ~110 s. A gate must never put anything in front of the person.

## 1. Approvals-off mode for test runs (`never_ask.rs`)
- **Enforcement point.** `approvals::request` is the only door that makes cards (the shell's `request_approval` and the recipe executor's `hand_off` both use it). It now asks `never_ask::refusal()` first and returns `refused: this action needs the person's approval, and approvals are off during a test run, …`. Nothing is stored, drawn, notified or sent to the phone, because all of that happens after the request is accepted.
- **Fail-closed.** It can only turn an "ask" into a "refuse". It never reaches `Decision::Run`, and a source-scan test (`it_can_only_refuse`) keeps `never_ask.rs` from naming grant/loosen functions. Actions that need no approval, grades, the mode table and the machine ceiling are untouched.
- **Who may switch it.** `shell.set_approvals_off_for_test state=on|off [minutes=N]`, graded `safe` like `set_mind_mode`.
  - ON: any caller. It only removes power, and its length is capped.
  - OFF: refused to the mind account, to any agent token and to any process an attached mind started. Otherwise only the shell's own account, root, or no socket call at all (a click) may do it (`may_switch_off`). The refusal says who can.
- **Expires by itself.** It is a deadline, derived from the clock on each read, never persisted. Default 30 min, max 240 min, 0 means 1 min. A restart clears it, and re-asking can only move the deadline within the cap from now.
- **`describe shell`** publishes `approvals_off_for_test: {on, expires_in_secs, means}`.
- **Arena.** `harness_arena.py` wraps `main()` in `ApprovalsOff` (on before the run, 90 min deadline). It turns it off in `finally` (error, `KeyboardInterrupt`, `SIGTERM` converted to a normal exit). If the shell cannot confirm it is on, the run does not start. `SIGKILL` can't be caught, so the shell's own deadline covers it.

### Review questions
- OFF refuses descendants of an attached mind (`requester_now() != Person`). A human running the arena from a mind's terminal could therefore turn it ON but not OFF, and would wait for the deadline. That is the safe side of the trade; say if you want it looser.
- ON is open to any socket caller by design. A mind can use it to make its own approvals-needing actions refuse for up to 4 h (a self-inflicted denial of service). It cannot gain permission that way.

## 2. Unknown action names list the real ones (`deploy/yantrik-os/yos-mcp`)
`guard_act` wrapped every ungradeable action in "how the OS grades it could not be read: …". When the app's `describe` answers and lacks the action, it now says `editor has no action new_note; its actions are: new, set_content, save_as, …. Nothing was run — call one of those`, so the model can correct itself. The "app not open", "describe would not answer" and unreadable-grade paths are unchanged.

## Tests
- `cargo test -p yantrik-ui --bin yantrik-ui`: 1161 passed, 1 failed (`harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`: untouched code, picks up this sandbox's `nvm` output; not from this change). New Rust tests: `never_ask::tests` (refuses and says why; expires; capped; a mind/other account cannot switch it off; can only refuse), and `approvals_off_for_a_test_run_refuses_and_raises_no_card` (store holds no card; without the flag a card is raised).
- Python: `tests/arena/test_approvals_off.py` (on during, off after; off after error, interrupt and SIGTERM; refuses to start on a desktop that can't turn it on). `yos-mcp-selftest.py` case 16c (`editor.new_note` / `new_file` refusals list the actions).
