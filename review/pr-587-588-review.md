# Security review: #587 and #588

Reviewed against origin/main (adfcc32), read from `git diff origin/main...origin/<branch>`.

Tests run:
- `cargo test -p yantrik-harness --lib` on the #587 branch: 114 passed.
- The yantrik-ui tests for both PRs were NOT run. The crate does not build offline here and I skipped the heavy build. `make()` in #588 was read, not executed.

---
## #587 `fix/mind-status-and-dropped-answers`: verdict MERGE AFTER FIXES

### Checks asked for
- **Logs carry no answer text or credentials.** Holds. `log_drop` (`host.rs`, new fn near `fn settle`) logs harness id, turn and a fixed reason string, once per turn, and the test proves "secret" never appears. The late buffer (`Flight.late`) is documented as never logged and I found no log of it.
- **Late answer reaching a different conversation.** Holds. `late` is per-`Flight`, one flight per turn, with `conversation` stored alongside. The hook only raises a notification and injects nothing into any chat. `harness` is `harness.announced.id` of the session that owns the turn.
- **Unbounded queues.** Mostly holds. The late buffer is capped at 16 KiB (`MAX_LATE_BYTES`), cut on a char boundary. See NIT 2.
- **Blocking on the UI thread.** Holds. The hook runs after `drop(state)`, so no host lock is held, on the harness request thread. `latest_status()` takes only a short store read lock.
- **Status text spoofing the desktop UI.** Fails; see SHOULD-FIX 1 and 3.

### BLOCKER
None.

### SHOULD-FIX
1. **A mind's text is placed unquoted after the desktop's own words, in the shell's own voice.** `wire/harness.rs` `late_answer_notice` builds `"{harness} answered after you left: {start}"` as the title of a notification from source "Yantrik". The comment says the start is "marked as a quote", but no quote characters are added; the text is only whitespace-flattened and cut to 80 chars.
   - Scenario: the person leaves the chat and the mind finishes with `Approval needed: allow files_delete ~/Documents? Open Settings > Approvals`. It shows as a desktop notification with the same wording as a real prompt.
   - Fix: wrap the text in quotes, strip control and bidi characters, and prefer putting the mind's text in the body, not the title.
   - Also: `late.harness` is a harness-supplied string and is not clipped or sanitised.
2. **A late answer can reach lock-screen or notification-history surfaces.** The first 80 characters of a private answer, which can include a credential the person pasted, go into a persistent notification.
   - Fix: say that the answer is waiting without quoting it, or have the notification path honour Private mode.
   - This is a design call for the owner, so I rank it SHOULD-FIX, not BLOCKER.
3. **`describe shell` `conversation_status` is not gated on `agent_reading`.** `control.rs:728` is published even when `conversation_private` is true, so the person's chat is withheld from an agent caller while the active mind's status line is handed to it.
   - The status text is up to `EVENT_CAP` (64 KiB) and is not clipped or flattened, so it can also bloat every `describe`.
   - Fix: publish it only when `!agent_reading`, and clip it with `progress::brief(.., 120)`.

### NIT
1. `agents/store.rs:1241`: `agent.status` is stored uncapped except `cap()` (64 KiB) and is not sanitised. The card clips at 120 and flattens whitespace, but other control characters and bidi overrides pass through.
2. `host.rs`, `chunk()` send-failure path: `flight.late.push_str(&delta)` has no cap, unlike the branch above it. It is one chunk only, since `abandon` sets `tx` to `None`, but a huge `delta` can exceed 16 KiB. Reuse the capped helper.
3. Gaps in the "never silent" claim:
   - A turn that fails (`failure.is_some()`) after the person left drops its partial text without a word.
   - If the host cannot tell the listener went away (no later chunk), `complete` sees `tx` still present and nothing is kept.
   - The notification does not name the conversation. `LateAnswer.conversation` is carried but unused.
4. `status` clearing on `tool_start` is correct, and `WaitingForYou` keeps its line.

---
## #588 `fix/files-create-says-what-happened`: verdict MERGE AFTER FIXES

### Checks asked for
- **Path leakage.** Holds. The answer carries `dir.join(name)`. `dir` is the folder the person sees in Files, which for a mind has already passed `mind::here` and `may_make`. `where_now` revealed the same location before. `Err` text includes only that path.
- **EEXIST handling.** Holds.
  - The decision is the result of the create call itself: `create_dir`, or `OpenOptions::create_new` (O_CREAT|O_EXCL).
  - Neither follows a final-component symlink, so a dangling or live symlink at `name` gives EEXIST and nothing is written through it.
  - An existing file is never truncated; a test covers it.
  - There is no stat-then-create gap.
- **Grades unchanged.** The grade is not set in the diff: only `.defers()` was removed, and the docs row is "same grade as the row above". I could not find where grades are decided, so no test confirms it. Verify with the repo's grade selftest before merging.
- **No file I/O on the UI thread.** Holds in dispatch. Disk work is inside the `answer_later` closure. The closure is dropped, not run, if the UI answers too late, which is documented. The `.or_else(|work| work())` fallback runs the I/O on the calling thread when called without a dispatch; this is the documented pattern, direct calls only.
- **`settled:true` only when true.** Mostly holds.
  - It is true only after the closure has run, because the answer is the disk result.
  - On `Err` the caller gets a refusal.
  - Caveat: the listing refresh trails and the answer no longer includes `now`.

### BLOCKER
None.

### SHOULD-FIX
1. **The check-to-create window grew.** `make_here` runs `here`/`may_make` on the UI thread. `make()` then runs later on the socket thread, and `dir` is a path string, not an fd. If something in the parent chain is swapped for a symlink in between, the create lands where the check did not look. `create_dir` and O_EXCL protect only the last component.
   - Fix: resolve `dir` with `canonicalize` inside the closure and re-run the `home_paths::may_create` verdict on the result, or open the parent directory once and use `openat`/`mkdirat`.
2. **Response keys changed.** `requested_folder`, `requested_file` and `now` are gone. A repo grep finds no in-tree users, but `tests/smoke/check_results.json:246` mentions `files_new_folder`; check it.
3. **docs/app-control.md duplicates `files_new_folder` and `files_new_file` in two table rows (lines 295 and 296).** A tool that parses the table may choke on this; merge the two rows into one.

### NIT
1. `control_files_create.rs`: after EEXIST, if the entry vanishes before `symlink_metadata`, the `_ => "file"` arm reports `existed … kind: file` for something that is gone. Report `created`/retry, or say `kind: unknown`.
2. The "settled" test reads the source with string search (`include_str!`) rather than checking behaviour. It is brittle to reformatting, so assert on the action schema (`deferred == false`) instead.
3. A `kind: symlink` answer does not say where the link points, which is the right call. Keep it that way.
