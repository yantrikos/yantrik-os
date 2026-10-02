# Review of PR #578 — keyboard cheat sheet (Super+/), branch `ui/keyboard-cheat-sheet`

Method: `git diff origin/main...origin/ui/keyboard-cheat-sheet` read in full, against `docs/agents/playbook.md`.
Not run: the `yantrik-ui` tests and the Slint build (heavy, and a git dependency needed a network fetch). Every finding below is from reading the code. The parser's unit tests were read, not executed.

## Verdict: MERGE (the SHOULD-FIX items can land in this PR or a follow-up)

Nothing here blocks. The parser is safe, the sheet is generated from the shipped file, and the control surface matches the playbook.

## Playbook checks
| Check | Result |
|---|---|
| One concern | Yes. The `rc_keys.rs` change makes the cheat sheet's parser the single rc.xml keybind reader, which is reuse, not scope creep. |
| Reuse | Yes. The old `bound_keys` and `canonical` in `rc_keys.rs` are removed, not copied. |
| Tokens | Colours are tokens (`panel-solid`, `keycap-fill`, `keycap-border` added to the theme). There are many px literals, see NIT 2. |
| Timers | None in `cheat_sheet.slint`. |
| Off the UI thread | No process, D-Bus or network call. rc.xml is `include_str!`. |
| Parity | `open_cheat_sheet` and `close_cheat_sheet` are `safe`. `describe shell` has `cheat_sheet`. Tests were updated. |
| `hold_windows` | `open_cheat_sheet` calls `hold_windows("open_cheat_sheet")` (`control_overlays.rs`). It is added to the source-scan list in `card_watch.rs`. The sheet also goes through `shell-overlay-opened`, which raises the shell. |
| True copy | I compared all 54 `@help` texts with the command or action each binding runs. All match. |

## Parser safety (`cheat_sheet.rs` `keybinds` / `help_in`)
- It is a hand scan of an `include_str!` constant. It never reads a user file, so there is no path or injection surface.
- A `<keybind` inside a comment is not a binding (tested). A second comment between `@help` and the keybind cuts the tie (tested). A malformed `@help` panics. That is acceptable for a shipped constant, because tests read it, and `shipped()` is only reached from the wire code.
- An unknown modifier panics. The old `bound_keys` did the same, but it ran only in tests. `keybinds` is now also reachable from the running shell (`wire/cheat_sheet.rs` → `shipped()`). The test `the_sheet_lists_exactly_the_keys_rc_xml_binds` guards the constant, so it cannot fire in a build that passed tests.

## Findings

### SHOULD-FIX 1 — the sheet shows the embedded rc.xml, not the one labwc is running
`crates/yantrik-ui/src/cheat_sheet.rs:25` (`SHIPPED_RC = include_str!(...)`), used via `shipped()` at `wire/cheat_sheet.rs:32`.
Failure scenario: a person or an installer edits `~/.config/labwc/rc.xml` (or `/etc/xdg/labwc`), or the packaged rc.xml is newer or older than the shell binary. The sheet then lists keys that do not work, or omits ones that do. The module doc and the action description both say "it lists exactly the keys that work".
Fix: either state the limit honestly (the description would say "the keys the desktop ships with") or read the installed file at open time, on a worker, falling back to the embedded copy. The reads would stay behind the same parser.

### NIT 1 — the sheet is re-parsed on every keystroke
`wire/cheat_sheet.rs:31-34` (`show` calls `shipped()` each time). That is about 54 rows, so it is cheap, but it is needless work on the UI thread while typing. Cache it in a `OnceLock`.

### NIT 2 — literal px and magic numbers in `cheat_sheet.slint`
Examples: lines 22-23 (`24px`, `14px`), 70-72 (`640px`, `600px`, `80px`), `34px` row height, `10px` paddings, `drop-shadow-blur: 32px`. The playbook asks for tokens. `Theme.sp-*`, `r-xl` and `fs-*` are used where they exist, so this is partial. Move the panel width and row height to tokens.

### NIT 3 — spelling
`W-A-c` is "Centre the window", but the rest of the sheet says "Maximize". Pick one spelling.

### NIT 4 — fixed `viewport-height` of rows × 34px
The row list assumes a one-line description. The longest text, "Show the desktop, then bring the windows back", fits at 640px. A longer `@help` would be clipped. Elide, or measure.

## Verified OK
- The `W-slash` binding runs `yos act shell open_cheat_sheet`. `every_yos_action_rc_xml_runs_is_published_by_the_shell` still covers it.
- The sheet cannot be opened while an approval card waits unless it is the person's own Allow call. `hold_windows` refuses a mind there, and the same applies to the keybind (as for the other panels).
- No new D-Bus, process or network call.
