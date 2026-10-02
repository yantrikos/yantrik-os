# Security review: PR #574 (feat/hermes-uses-yantrikdb)

Scope: `git diff origin/main...origin/feat/hermes-uses-yantrikdb` (22 files, ~1,860 lines). No code changed.
Tests: `python -m pytest harnesses/tests harnesses/hermes/tests -q` -> 274 passed, 4 skipped. The PR only adds tests (no deleted or weakened assertions). Rust tests were not run.
Not reviewable here: `mind-memory-mcp` and `yantrikdb-hermes-plugin@5c1abc5` (separate repos, outside session scope). Claims that depend on them are marked UNVERIFIED.

## Overall verdict: MERGE AFTER FIXES

No CRITICAL issues. The credential path is well designed (per-turn, in-process, no env/argv/log). The remaining problems are allowlist erosion after install, revocation that never reaches the Hermes process, and a few fail-open edges.

## Verdicts on the five previous findings

| # | Finding | Verdict |
|---|---------|---------|
| 1 | Credential in os.environ (HIGH) | **FIXED** in this tree. See F5 for defense-in-depth gaps; the loopback/unix URL check lives in the plugin (UNVERIFIED). |
| 2 | Toolset denylist | **PARTIAL.** An allowlist is written and verified with Hermes's own resolver at install time only. It is not re-enforced after `hermes update`, a plugin install, or a person's config edit (F1). |
| 3 | Door impersonation | **PARTIAL / out of tree.** `YANTRIK_PERSON_UID` now comes only from the unit or passwd, never the settings file, and fails closed with 0 or 2+ accounts. Ordinary-grant impersonation remains the known issue. Nothing in this tree hands out sensitive grants (see note below). |
| 4 | Token fallback picks the wrong credential | **PARTIAL / UNVERIFIED.** This tree registers strictly per `session_key` and passes `None` to revoke. Whether the plugin has a "single credential" fallback is in the other repo. The adapter's multi-path session-key derivation (F6) can make the key diverge from the gateway's. |
| 5 | Unpinned plugin deps | **PARTIAL.** Hash-locked deps plus a commit-pinned plugin with `--no-deps`. Build-time deps of the git sdist are still resolved unpinned (F7). |

Sensitive-grant note: `harness_memory::with_install_grant` sets only `recall_ordinary`, `remember` and `believe`. It never sets `recall_health`, `recall_finance` or `household`, and the control-surface `install_harness` path is test-guarded against granting. I found no path in this tree that issues sensitive grants. Residual: `with_install_grant` keeps any sensitive grants already present for that id. See F9.

## Findings

### HIGH

**F1. Toolset allowlist erodes after install** (`harnesses/lib/install/hermes_config.py:98-113`, `hermes.sh:133,142`)
The allowlist and `known_plugin_toolsets` are written once, during install. Nothing re-applies or re-checks them afterwards.
Scenario: the person later runs `hermes update` or `hermes plugins install X` (or a skill installs a plugin). X's toolset is not in `known_plugin_toolsets.yantrik`, so Hermes's resolver turns it on by default for the `yantrik` platform. If X provides a shell or file toolset, that is the original bypass of every desktop approval, with no signal to the person. `hermes update` may also rewrite config defaults.
Fix: run `hermes_config.py apply` then `check` as `ExecStartPre=` on the gateway unit (a drop-in) so every gateway start re-applies the allowlist and refuses to start on failure. Optionally add a periodic `check`.

**F2. Credential revocation never reaches the Hermes process or the memory server from this tree** (`crates/yantrik-harness/src/host.rs:1122`, `adapter.py`)
`Host::revoke_memory_credentials` has no non-test caller in the tree, and no `memory.revoked` is ever sent. There is also no uninstall path that removes the grant. The only revocation is the next-poll check in `host.rs:1640-1650`, which withdraws the credential only when a turn for that harness is next polled.
Scenario: the person takes away Hermes's memory (or Hermes is stopped or uninstalled). The credential already registered in the gateway process stays in the plugin registry until the next turn. If the memory server has not been told the digest is revoked, it keeps working until it asks the shell. A stopped adapter calls `_forget_memory()` only on a clean disconnect, not on a kill.
Fix: call `revoke_memory_credentials` when grants change and send `memory.revoked` to the memory server. Add an uninstall action that removes the grant and revokes. Give credentials a short TTL.

### MEDIUM

**F3. Grants and the gateway stay live after a failed final check** (`hermes.sh:136-143`)
`check` runs after `systemctl --user restart hermes-gateway`. If `check` fails, the script exits non-zero, but the gateway is already running with the plugin enabled and possibly Hermes's own terminal on the desktop platform. The grant is correctly not written, but the dangerous exposure the allowlist exists to prevent is live.
Fix: run `check` before starting the gateway, and `systemctl --user stop hermes-gateway` on any later failure.

**F4. `_carry_memory` fails open on unexpected provider errors** (`adapter.py` `_carry_memory`, `except Exception`)
On a non-`ValueError` exception from `set_desktop_credential`, the function logs and returns without clearing. A previously registered credential for that session stays, so a turn after a revoke can still present the old credential.
Fix: on any exception, call the clear function for that session (or `register(session_key, None, None)`) in a nested try, and set `_carried_memory = None`.

**F5. Adapter does not itself validate `memory_url`** (`desktop.py` `carry_memory`)
The URL from the desktop assignment is passed on as is. The off-host rejection (loopback `http://` or `unix:/abs`) is enforced only by the plugin (UNVERIFIED, separate repo). A plugin bug or version drift would send the bearer credential off-host. The URL comes from the desktop host, which is trusted, so this is defense in depth only.
Fix: validate in `carry_memory` and refuse (revoke) otherwise. Parse with `urllib.parse`, require scheme `http` and host in {127.0.0.1, ::1, localhost}, or `unix:` with an absolute path.

**F6. Session-key derivation has four fallbacks and a single constant chat id** (`adapter.py` `_gateway_session_key`, `_carry_memory` called before the busy-session branch)
A derived key that differs from the gateway's makes the credential unreachable (fail closed). But if the plugin has any fallback to a lone credential, one wrong key would make a different session present it (F finding 4). Every desktop turn also shares one `CHAT_ID`, so the credential of a message that arrives while a turn is running overwrites the running turn's. This is harmless with one agent per chat, but breaks per-agent credential isolation if the desktop ever multiplexes agents over this adapter.
Fix: assert the key equals the one the gateway used (log once on mismatch); do not register while the turn is only queued as busy; confirm no lone-credential fallback in the plugin.

**F7. Lock does not cover build-time dependencies** (`hermes.sh:69-77`, `yantrikdb-hermes-plugin.lock`)
The plugin is installed from a git URL with `--no-deps`, which builds an sdist. Its `build-system.requires` (setuptools/wheel, etc.) are resolved from PyPI unpinned at build time, and are not covered by the hashes. A hijacked build backend runs code as the person at install. `yantrikdb==0.23.1` wheels vs sdist is also not constrained (`--only-binary` is absent), so an sdist build can run arbitrary setup code.
Fix: `--only-binary :all:` for the locked deps; build the plugin wheel from the pinned commit with `--no-build-isolation` against a pinned, hash-locked build env, or vendor a built wheel with a hash.

### LOW

**F8. `yantrik_os` MCP server is not verified** (`hermes_config.py:110-112`, `problems()`)
If `mcp_servers.yantrik_os` already exists, it is kept as is, including any `command`. `check` does not verify the command is `/opt/yantrik/bin/yos-mcp`. A pre-existing entry could run something else as the "desktop tools" server. Fix: verify or overwrite the command and flag a difference.

**F9. `with_install_grant` merges into an existing entry by manifest id** (`harness_memory.rs:44-52`)
A manifest from a writable harness root whose `id` equals another mind's id (for example `pi`) would, on the person's Install click, add ordinary grants to that mind. Only ordinary grants, and only on a click. Fix: refuse ids that already have grants unless the entry was created by this harness's own manifest, and ensure harness roots are not user-writable.

**F10. Grant issued even if `YANTRIKDB_MODE` is not `yantrik`** (`hermes.sh:116-120`)
If the person's `~/.hermes/.env` sets another mode, the script only prints a note, yet the grant is written on success. The credential is then unused (harmless), but the grant is broader than the effective setup. Fix: fail or skip the grant.

**F11. `YANTRIK_ALLOW_ALL_USERS=true` relies on the 0700 socket directory** (`hermes.sh:126`)
If `YANTRIK_HARNESS` points at the dev TCP path, any local user can message Hermes with no pairing. Fix: only set it for the unix socket, and document the TCP path as dev-only.

**F12. Concurrent writes to `config.yaml`** (`hermes_config.py:116-136`)
Read, modify, write, rename with no lock. If the gateway or a `hermes config set` writes in between, the update is lost (the final `check` mitigates at install time only; see F1). Fix: lock or re-check after write.

**F13. `delegation` is in the allowlist** (`hermes_config.py:37`)
Sub-agents get their own toolsets; whether Hermes's delegate defaults include `terminal`/`file` is UNVERIFIED. `check` resolves only the platform, not delegate children. Fix: set `delegation.toolsets` explicitly, or drop `delegation`.

## Not found
- Credential in logs, argv, env, files or errors: none (adapter logs only the exception type; Rust logs only the harness id).
- Grant on a failed install: none (`install_then` runs the grant only on exit 0, with tests).
- Shell injection in `hermes.sh`: none (constants only in `env_default`; quoting is correct).
- Weakened tests: none.
