## 1. Confirmation against revision 1

This checks the supplied document, not repository implementation. A promised build/test gate is not evidence that it passed.

### B1–B10

**B1 — PARTLY.** The architectural invariant is repaired, but lifecycle step 8 unconditionally constructs a primary window. Headless command processing also needs a wake mechanism independent of Slint.

Replace lifecycle steps 5 and 8 with:

> **5. Execution mode:** Resolve the initial window set independently of `Lifetime`. With no windows, construct no component and select no graphical backend; use a backend-independent command pump and wake mechanism. Initialize the graphical backend before constructing the first window.  
> **8. Initial windows:** Construct only the requested windows; apply theme, chrome and shortcuts, wire callbacks, and render each. An empty initial window set is valid.

**B2 — PARTLY.** The bridge and adapter contract are present. The required two-crate release compilation remains a future gate, not supplied evidence.

Replace the §2.3 gate bullet with:

> **Approval gate:** The adapter design remains provisional until passing release builds from two separate generated UI crates are attached as evidence. Neither production adoption nor adapter approval precedes that proof.

The build evidence itself is still required; wording cannot substitute for it.

**B3 — RESOLVED.** Refusal invariance, staged effects, revisions, separate continuations and registration checks are specified. Queue admission creates a related inconsistency addressed under B6.

**B4 — RESOLVED.** Invocation identity, settlement modes, terminal outcomes, provenance association and invocation-specific replies are specified.

**B5 — RESOLVED.** Admission authority, bound approval, dequeue-time checks and proposal provenance are repaired. One factual qualification appears below.

**B6 — PARTLY.** Bounded queues, fairness, panic isolation, scoped replacement and honest worker shutdown are present. Missing: how effect admission avoids reporting refusal after mutation; how terminal outcomes survive queue saturation; and whether persistence flushing can defeat bounded shutdown.

Replace the “Bounded” paragraph and the lifecycle instruction “flush the persistence lane” with:

> **Bounded admission:** Reserve execution capacity before applying a command whose acceptance depends on that capacity. Capacity rejection leaves the model unchanged. Failures discovered after commit are operation failures, not command refusals. Job completion, cancellation and terminal settlement use reserved bounded control capacity, so saturation cannot lose an accepted invocation’s terminal outcome. The UI thread never blocks waiting for queue space.

> **Persistence shutdown:** Flush only within the shutdown grace deadline. At expiry, exit without an unbounded join or flush; unfinished writes are not reported as durable. Recovery uses the last durably committed revision.

**B7 — PARTLY.** Ordering, desired versus committed state, recovery and conflicts are repaired. “Content hash on change” still misses same-size edits whose metadata is unchanged.

Replace the fingerprint sentence with:

> Polling fingerprints file contents as well as identity and metadata; content checks occur independently of metadata changes. It handles replacement and deletion and suppresses self-write echoes by committed fingerprint.

**B8 — RESOLVED.** Close/exit commands, save-before-close, live-loop checkpoints, mutation-free cleanup and bounded logout are present.

**B9 — RESOLVED.** Argument acquisition, focus/shortcut precedence and stable identities are repaired. The illustrative callback has a separate Rust error below.

**B10 — RESOLVED.** Profile-keyed identity, release validation, readiness, acknowledged request-ID handoff, startup unwind and single launch application are specified.

### “Decide now”

- **Large lists — PARTLY.** Retention and incremental updates are present; bounded paging and cursor validity are not defined. Replace the final sentence of §8’s Lists bullet with:

  > Paging has explicit row and byte limits and supports cancellation. Surface cursors bind to the query and committed revision; a cursor from another revision is rejected as stale and requires a restart.

- **Undo — RESOLVED.**
- **Desktop integration — RESOLVED.**
- **Launch/deep links — PARTLY.** Explicit profile targeting is missing. Replace §10’s launch bullet with:

  > **Deep links and launch:** `Launch` is typed and versioned, with explicit profile and optional window targets, files or intent arguments. Validate all fields as untrusted input. Reject mismatched instance targets; deep-linked intents enter the authorized intent path.

- **Data upgrades — PARTLY.** Versioning and tests exist, but migration ownership and crash behavior remain implicit. Replace §6’s opening app-data sentences with:

  > Apps own application-data and draft schemas and migrations; the framework owns ordered durable replacement and recovery mechanics. Migration preserves the last valid source until the upgraded destination commits durably. Interrupted migration resumes idempotently or enters protected recovery. Future versions are refused without overwriting them.

- **Accessibility/localisation — RESOLVED.**
- **Privacy/telemetry — PARTLY.** “Have a stated retention” does not state one. Replace that clause with:

  > Problem records expire after 30 days and have a 10 MiB per-profile cap, evicting the oldest first. Users can delete them immediately.

- **Testing — RESOLVED.** Injected paths and manually scheduled execution are present.

### Migration points

All are **RESOLVED as plan requirements**, not verified implementation:

1. Mechanical extraction, compatibility tests, dependency-cycle check.
2. Minimal runner, two-crate release proof, bounded queue, scheduled executor.
3. Invocation/provenance/settlement with real-dispatch tests.
4. Ordered persistence, recovery and lifecycle.
5. Typed services and rail.
6. Architecture checks with the first migrated feature.
7. Weather parity before a separate redesign.
8. Every specified pre-Notes Weather failure test.
9. Notes as an acceptance gate, including all specified scenarios.
10. Large-list fixture before Calendar/Email.
11. Real socket/credential/approval integration.
12. Compatibility removal only after migration and the support interval.

The unresolved B2 evidence requirement remains despite its correctly placed migration gate.

## 2. New errors in revision 2

**1. The selection callback cannot use `?` while returning a plain command.** Replace it with:

```rust
ui.on_select_place(cx.cmd1(|id: SharedString| {
    Ok(places::Command::Select(PlaceId::parse(&id)?).into())
}));
```

Add:

> `cmd1` accepts fallible converters returning `Result<A::Command, Refused>`.

**2. The `SO_PEERCRED` explanation is factually too broad.** Linux peer credentials include PID, so same-UID processes can be distinguished; that does not authenticate application identity.

Replace the first identity sentence with:

> Peer credentials provide transport evidence, including PID, UID and GID on Linux; they do not by themselves authenticate an application identity, including among processes sharing a UID.

## 3. Verdict: REVISE

Most repairs are adopted correctly. Remaining blockers are the missing adapter proof, contradictory headless lifecycle, and incomplete queue/shutdown guarantees. The replacements above close the remaining document gaps without reopening settled architecture.
