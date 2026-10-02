# PR #582 security review: fixes (2 Oct 2026)

Rebased on `origin/main` (no conflicts). Grades, approvals and `hold_windows` are unchanged; no new action.

| Finding | Fix | Test |
|---|---|---|
| B1 DISPLAY inherited | Every mind launch clears `DISPLAY`, `WAYLAND_DISPLAY`, `WAYLAND_SOCKET`, `XAUTHORITY`, then sets only Mind View's display; Wayland-only toolkit env without Xwayland (`Seat::display`, `launch_command`). Blender in Mind View needs Mind View's own Xwayland. | `a_minds_app_never_inherits_the_persons_display`, `a_minds_launch_command_never_carries_the_persons_display`, viewer test |
| B2 duplicate on desktop | `Route.spawn=false`: nothing is started; the answer says not launched, not raised. | `a_minds_launch_of_an_app_open_on_the_desktop_starts_no_process`, route test |
| B3 other paths | `open_url` goes through the companion's browser in Mind View (no `xdg-open`); `display_for_mind` returns an error when Mind View is down or no shell is present; browsers clear the same variables; the companion does not attach to a non-companion browser on port 9222. | companion `mind_display_tests` |
| S1 | Window match reads declared app id / exact title, excludes the agent viewer. | three `lists_app` tests |
| S2 | Errors say only what was observed; "not on the desktop" only on a refusal; refusal not shown as "exited with". | `mind_landing` tests |
| S3 | A mind's chromium/firefox gets its own profile. | `a_browser_for_a_mind_gets_a_profile_of_its_own` |
| S4 | Command line caret-escaped; output reduced to text and SGR colour. | cleaner and spoof tests |
| S5 | `files_open` for a mind waits for the window and reports a refusal. | source scan test |
| S6 | Failed Mind View start retried after 30 s. | `a_failed_start_is_tried_again_after_a_while` |
| S7 | Viewer opens on its own worker; `hold_windows("agent_run")` before Mind View starts. | `the_viewer_is_held_while_a_card_waits...` |
| S8 | Per-command 1 MiB cap, one append handle, mode 0600 at creation. Log is not deleted when an agent ends (no hook for it). | cap and mode tests |
| S9 | Ids that sanitise alike get a hash suffix; start failure writes "(not started)"; setting off stops mirroring at the next command. | stem and abandon tests |
| S10, N1-N3 | Source scans for `answer_later`; wording and constants fixed. | `open_app_and_files_open_wait_for_the_window_off_the_ui_thread` |

Not fixed: N4 (second id derivation), N5/N6 (documented in `docs/app-control.md`), N7, N8. Singleton handover of a browser started under a profile the registry cannot see is covered only by the separate profile.
Not verified: nothing was run on a real labwc, so window matching against live `wlrctl` output is untested.

## Tests
- `cargo test -p yantrik-ui --bin yantrik-ui`: 1029 passed, 1 failed, 1 ignored. The failure, `harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`, output `nvm\n✓ uv ready`, comes from this sandbox's shell profile printing `nvm`; it is in a file this branch does not touch. I did not run it on `main`.
- `cargo test -p yantrik-companion-tools --lib`: see the final message.
- Built with `CARGO_BUILD_JOBS` 1-2, `CARGO_PROFILE_DEV_DEBUG=0`, 12 GB swapfile; no OOM.
