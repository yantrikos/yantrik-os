# Independent security verification, 2026-10-02

Method: for each PR, the original review was compared with `git diff origin/main...origin/<branch>` and the code on the branch as it is now. The fix reports were not trusted.
Static review only. No VM, no secrets.

Tests run:
- `cargo test -p yantrik-os` on `ui/network-indicator`: 81 passed, 0 failed (lib), the rest ignored.
- `python -m pytest harnesses/tests harnesses/hermes/tests -q` on `feat/hermes-uses-yantrikdb`: 301 passed, 4 skipped.
- `node deploy/live/gate-models/gen-models-list.js --check` on `live/gate-nim`: matches.
- NOT run: anything in `yantrik-ui` or `tests/ui-preview`. Their build needs speechd and a full Slint build, which this sandbox lacks.
  So the PR #573 handler code and the PR #580 Rust and Slint changes are verified by reading only. The authors' own fix reports say the same.

---

## PR #573 (ui/network-indicator): MERGE, on condition that CI compiles and runs the yantrik-ui tests

| Finding | Status | Evidence |
|---|---|---|
| H1 grade below the repo's | VERIFIED FIXED | `control_network.rs:98-100` `grade_for` returns `dangerous` for `disconnect_network` and `set_wifi` whatever the reading (the snapshot argument is ignored). Both actions are declared `.risk("dangerous")` (`:258`, `:293`). Tests pin the match with the companion tools. |
| H2 stale grade, no re-check | VERIFIED FIXED | `grade_still_holds` (`:115-131`) is called in the handler on `latest()` with the stale flag (`:269`, `:298`). It is called again inside the `answer_later` work, on `network::read_fresh()` (`:273`, `:300`), before the D-Bus act. A stale or unreadable state refuses. The grade is a constant today, so the check can only refuse (fail-closed). |
| M1 `connect()` could create for a mind | VERIFIED FIXED | `yantrik-os/src/network.rs:712` `permitted(by_person, &decided)`, called at `:786` on the fresh reading, before any D-Bus write. |
| M2 unbounded threads, connections, joins | VERIFIED FIXED | Shared `BUS` connection (`:583`). Join slot taken before the thread is spawned (`begin_join` `:148`; `wire/network.rs:213`, spawn at `:232`). Mind joins spaced by `MIND_JOIN_SPACING`. `MUTATING` mutex serialises disconnect and radio (`:620`, `:677`). |
| M3 mind can disconnect a wired link | VERIFIED FIXED | The control surface calls `disconnect(false)` (`control_network.rs:302`). `choose_disconnect_target` refuses wired. The person's popover passes `true` (`wire/network.rs:157`). |
| M4 listener exit freezes the picture | VERIFIED FIXED | The listener loops with doubling backoff capped by `RESUBSCRIBE_CAP` and logs (`network.rs:260-296`). The monitor polls every 15 s when it is down. A failed read sets `STALE`, and `describe` reports `network.stale` (`control_network.rs:175`). |
| L3 attribution | VERIFIED FIXED | `control_network.rs:215` `requester_now()`. |
| L4 inline D-Bus fallback | VERIFIED FIXED | `answer_later(work)....map_err(\|_\| NOT_A_DISPATCH)` (`:280`, `:310`). |
| L5 poll timeout read as failure | VERIFIED FIXED | `join_step` (`network.rs:732`): `None` keeps waiting until the cap. |
| L1, L2, L6, L7, L8, L9 | NOT FIXED (deliberately left) | The report says so. L2 matters most: an open evil-twin joined by a person autoconnects. It is not a mind-exploitable path. Track it. |

New issues:
1. LOW. The person-only popover callbacks for scan, radio and disconnect (`wire/network.rs:139-159`) still spawn one OS thread per click. They are serialised by `MUTATING` but not bounded. The trigger is a person's click, not a mind.
2. LOW. `tracing::info!(%ssid, %why ...)` (`wire/network.rs:235-237`) logs the SSID and the error text. No password, and a source-scan test forbids `secret`, `password` or `request` in log lines. Passes.
3. INFO. The handler and worker edits in `yantrik-ui` are uncompiled and untested here and by the author. A CI run is required.

No credential in logs. No grade lowered; both grades went up. No test weakened.

---

## PR #574 (feat/hermes-uses-yantrikdb): MERGE, with follow-ups F2 (trigger path), F7, F9, F13

| Finding | Status | Evidence |
|---|---|---|
| F1 allowlist erodes | VERIFIED FIXED (with a design limit) | `guard.py:start/check`. The adapter runs `hermes_config.reassert` on gateway start (`adapter.py:149`) and `guard.check` on every attach (`:189`). It refuses to attach when the file was widened, and a missing checker is a refusal (`guard.py:60-62`). Limit: the gateway has started by the time the adapter runs. The guard protects the desktop attach, not Hermes's other platforms. |
| F2 revocation never reaches Hermes | PARTIAL | Fixed: `host.rs:1125-1131` sets `memory_revoked`. A withdrawn grant at poll time does the same (`:1656`). The next poll carries it once (`:1703`). The adapter handles it (`adapter.py:242-245`, `_revoke_memory` `:416`). Not fixed: `harness_memory::take_away` and `wire::harness::take_memory_away` (`wire/harness.rs:355`) have no caller, so there is no button, uninstall or action that triggers them. Revocation works only when the grant store changes some other way. Credentials have no TTL. |
| F3 live gateway after failed check | VERIFIED FIXED | `hermes.sh:177-192`. `hermes_check` runs before `gateway install` and again after the start. Each failure runs `stop_gateway` then `fail`. |
| F4 `_carry_memory` fails open | VERIFIED FIXED | `adapter.py:398-409`. On any exception it clears that session's key, then everything if that fails. |
| F5 `memory_url` not validated | VERIFIED FIXED | `desktop.py:65-84` `valid_memory_url`. It parses with urlsplit and requires `http` with a loopback host and no userinfo, or `unix:/abs` with no `..`. |
| F6 session-key derivation | VERIFIED FIXED | One path (`desktop.py:144`). A mismatch with the gateway's key gives no memory and logs once (`adapter.py:_gateway_session_key`). A busy or queued turn does not register (`:326`). |
| F7 build-time deps unpinned | NOT FIXED | `hermes.sh:98-103` still runs `pip install --no-deps "$YANTRIKDB_PLUGIN"` with no `--only-binary` and no `--no-build-isolation`. |
| F8 `yantrik_os` MCP entry | VERIFIED FIXED | `hermes_config.py:42` and `:111` always write the entry, and `check` fails on any other. |
| F9 `with_install_grant` merges by id | NOT FIXED | Not changed. Only ordinary grants, only on a click. |
| F10 grant when mode is not yantrik | VERIFIED FIXED | `hermes.sh:147-152` fails with no grant. |
| F11 allow-all on the TCP path | VERIFIED FIXED | `hermes.sh:162` turns it on only for off, `unix:*` or `/*`. |
| F12 config.yaml write race | NOT FIXED (mitigated by the start and attach checks) | |
| F13 `delegation` in the allowlist | NOT FIXED (unverifiable here) | |

New issues:
1. LOW. `take_memory_away` is dead code (see F2). Wire it to a graded control-surface action in a follow-up with its own review.
2. LOW. A tests file was edited: `test_the_desktops_mcp_server_is_added_when_missing_and_theirs_is_kept` now expects an existing `yantrik_os` entry to be replaced. This follows from F8 and is not a weakening. All 301 pass.

No credential in logs, argv or env. The adapter logs only exception types.

---

## PR #577 (live/gate-nim): MERGE

| Finding | Status | Evidence |
|---|---|---|
| MEDIUM-1 NIM key a hard requirement | VERIFIED FIXED | `setup-models.sh:36-50`, `:86-88`. A missing provider keeps the gate's key or gets an `unconfigured` stub. The route answers 502. A trap restores on failure (`:78`). |
| MEDIUM-2 Mind-side names | PARTIAL (unverifiable from the repo) | `mind-env.sh` has a comment saying the names were checked in the Mind's code. The Mind's code is not in this repo. The catalogue id is still `nvidia-nim` (`catalogue.rs:208`). The names in the files did not change from the reviewed version, only the comment. Check on the live VM that NIM traffic goes to the gate before making `nim:` the primary brain. The route is dormant until then. |
| LOW-1 malformed key silently ignored | VERIFIED FIXED | `setup-models.sh:24-31` exits 1, naming the variable and not the value, before anything changes. |
| LOW-2 `NGC_API_KEY` alias | VERIFIED FIXED | `pick-keys.py` alias list has no NGC. |
| LOW-3 `NVIDIA_API_KEY` name | NOT FIXED (explained) | The name is the one the Mind reads. The value is only the instance key. |
| LOW-4 backup and rollback | VERIFIED FIXED | The backup is removed after a good reload (`:135`). A failed reload is an error (`:133`). A trap restores on any failure (`:78`). `mind-env.sh` keeps its first backup. |

New `/models` locations (`models-list.conf`, generated by `gen-models-list.js`, installed by `setup-models.sh:96-106`):
- The same instance-key check as the chat routes: `if ($http_authorization != "Bearer @INSTANCE_KEY@") { return 401; }`. It runs before anything else.
- `location =` exact match only. `limit_except GET { deny all; }` allows GET and HEAD.
- Static `return 200` JSON. There is no `proxy_pass`, no `js_content` and no provider auth include, so no upstream and no provider key.
- The JSON comes from the RULES in `live_models.js`, with only the allowed model ids. `--check` passes.
- The instance key is substituted on the gate by awk, after a hex-only check (`:99`). The files are written under `umask 077`.
- Rollback covers `models-list.conf`.

New issues:
1. LOW. The `/models` routes have no `limit_req`. They cost no upstream and are authenticated, so this is acceptable.
2. INFO. `pick-keys.py` still treats `NVIDIA_API_KEY` as an alias for `NIM_KEY`. A file holding the instance key under that name would now be rejected by the format check, exit 1. That fails safe.

---

## PR #580 (ui/chat-v2): MERGE

Approval security is unchanged:
- No autofocus, no Enter. `ApprovalCard` has no `FocusScope`, `key-pressed`, `forward-focus`, `focus()` or `init` (`intent_lens.slint`, ApprovalCard block). The only new focus call is `composer.focus-input()`, on the text field.
- Press guard: `card_watch.rs` has no diff at all.
- `control_approvals.rs`: `row_for` gains `decided_at` and `session` (display only), plus the fields in three test constructors. `approvals.rs` adds the two `Card` fields. No decision, grade or guard logic changed. The existing tests were not weakened.
- Button wiring: `allow`, `allow-session` and `deny` still map one to one to `approval-allow*` and `approval-deny` (`intent_lens.slint:~2142-2144`). `view-action` only navigates.

| Finding | Status | Evidence |
|---|---|---|
| SHOULD-FIX 1 resolved line | VERIFIED FIXED (by reading) | The line reads "Approved by you · time · action (this session or once)", from `Card.session`, set from `record.session`. |
| SHOULD-FIX 2 sensitive vs standard | VERIFIED FIXED (by reading) | The edge and heading follow the grade. A test exists in `lens_card_layout.rs`. |
| SHOULD-FIX 3 card moves | PARTIAL | The block moved to directly under the header, above the transcript, strip and composer (`intent_lens.slint:~2126`). The height-limit uses the worst case for the composer and strip (`:2150-2152`). This removes the reviewed strip and composer shift. It is not a fixed top edge. `Store::cards` orders decided records first and the pending card last (`approvals.rs`, `cards()`), and the block also holds one 32px row per decided record. A record added or expiring above the card moves it by about 32px. The `height-limit` term `32px * approvals.length` also changes with it. |
| NIT 1-5 | Left alone | As the report says. |

Press-then-shift race: no path to a wrong approval. `clicked` needs press and release in the same `TouchArea`, and the model is rebuilt on change, so a shift can only drop a click. Approve and Decline share a row. The residual is a possible missed click when a record is added or expires above the card.

New issues:
1. LOW. The residual record-row shift above. Pin the pending card above the records, or reserve the records' height.
2. INFO. The preview checks (`chat_tests.rs` 1b, 1c) were written but never built or run, so the rendered behaviour is unverified.

No credential, grade or test regression found.

---

## Verdicts
- #573: MERGE (all HIGH and MEDIUM fixed; require CI compile and test of yantrik-ui first)
- #574: MERGE (F2 PARTIAL: no trigger path for take-away; F7, F9, F12 and F13 NOT FIXED; follow-ups)
- #577: MERGE (MEDIUM-2 is PARTIAL and unverifiable from this repo; dormant until the primary brain is switched)
- #580: MERGE (SHOULD-FIX 3 PARTIAL, only a possible missed click)
