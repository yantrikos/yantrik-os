## Verdict: REVISE

The command/model direction is sound. The document is **not yet a durable framework contract**: authorization across deferred work, invocation completion, shutdown, persistence ordering and window lifetime remain underspecified. Weather could pass while all six remain broken.

This review is against the supplied design, not an independent verification of the repository or Slint build.

## 1. Blocking problems

### B1. “One window” is an architectural dead end — §§2, 2.1, 2.2

Notes will need document windows; Email will need composition windows. Dialogs and background operation also invalidate “exit when the window closes.”

**Replace the opening invariant with:**

> One process per application/profile instance, one application model and command type, one surface, and zero or more windows. Windows have stable `WindowId`s and project application state. Closing a window is distinct from terminating the application. The initial implementation supports one primary window but must not encode that limit in command routing, job ownership or lifecycle.

Keep `type Ui` as the primary component type, not the type of every future window. Add:

```rust
pub enum Lifetime { WhileWindowsOpen, ExplicitExit }
```

Define the default as `WhileWindowsOpen`; headless operation must not construct a component or select a graphical backend.

### B2. The UI-thread bridge and generated-type adapter are missing — §§2.1, 2.3, 4

`Cx` being `!Send` prevents misuse but does not explain how a worker delivers a command. Capturing its `Rc` in `invoke_from_event_loop` will not compile.

**Add:**

> `Window: slint::ComponentHandle + 'static`. The runtime maintains UI-thread-local application state. Workers receive only owned inputs and a bounded, `Send` command endpoint. Event-loop wake closures capture only `Send` routing data; they resolve application state on the UI thread. No worker captures `Cx`, a component, a timer, or the model. Posting after shutdown fails harmlessly.

Also:

> `window!` expands in the app crate and adapts locally generated globals and rail types to framework-owned Rust DTOs. Required globals and conversions are an explicit adapter contract. UI adapters own retained Slint list models; those do not belong in the application model.

Make this compile in **two separate generated UI crates**, including release mode, before approving the adapter design. “One line” is a convenience goal, not evidence of feasibility.

### B3. Refusal and effects have no transactional semantics — §§2, 3, 4

`update` can mutate the model, enqueue jobs, then return `Err`. Does the surface report failure after performing the operation? Does rendering occur? `then` also creates an undefined partial-success boundary.

**Add:**

> Commands validate before mutation. Returning `Refused` must leave the model unchanged. Effects are staged and committed only on success; the framework does not automatically roll back arbitrary Rust mutations. This invariant is tested. A command requiring atomic changes across features performs those changes in one update; `then` schedules a separate, fallible command and is not a transaction.

> Every successful model change advances a model revision. Rendering and the corresponding surface projection observe the same committed revision. `Changed` defaults to all parts unless explicitly narrowed.

The “compile error is the registration check” claim must go: missing `wire`, `render` or intent registration calls compile perfectly well.

### B4. Settlement is not implementable from the declared contract — §3

An invocation can start zero jobs, several jobs or a chain. Jobs can be replaced, cancelled or panic. “The job the command started” does not identify a completion condition.

**Replace `AfterJob` with an explicit invocation protocol:**

```rust
pub enum Settle {
    OnReturn,
    Deferred,
    Explicit { within: Duration },
}
```

> Each accepted invocation receives an `InvocationId`. Explicit settlement requires exactly one terminal completion associated with that invocation, after its result command commits. Jobs and continuations retain that association. Timeout, cancellation, replacement, worker panic and shutdown produce defined terminal outcomes. A deadline does not imply that an external operation was undone. Deferred responses expose an operation identifier and a queryable terminal state.

> Replies describe the invocation’s committed result, not whichever unrelated selection exists when the reply is generated.

### B5. Keeping dispatch unchanged leaves a policy race — §§3, 5

A revision guard evaluated off the UI thread can become stale before application. Provenance can also disappear when a rail proposal becomes a plain command.

**Replace the policy paragraph with:**

> Existing surface dispatch remains the external admission authority. It issues an internal, unforgeable invocation context containing authenticated principal, origin, authorized operation/arguments and applicable grant/revision constraints. At UI-thread dequeue, the runner atomically checks revision and state-dependent preconditions before update. Approvals bind to the approved operation and arguments; changed arguments or invalidated conditions require renewed authorization.

> Jobs, continuations and rail proposals preserve invocation provenance. Agent proposals re-enter the authorized intent path; they cannot use the trusted local-input path. Direct button commands are permitted only for genuinely local UI events. Command validation applies to every origin.

`SO_PEERCRED` authenticates kernel credentials, **not an app identity merely asserted by another process with the same UID**.

**Replace the final §5 sentence with:**

> Peer credentials are transport evidence. Application identity and grants follow the platform’s authenticated identity mechanism; caller-supplied flags are never authoritative. Socket ownership, permissions, peer validation and request limits are mandatory. Manifest entitlements configure clients but are not themselves a security boundary.

### B6. Shutdown promises cannot be met with blocking threads — §§2.2, 4

Rust cannot forcibly cancel a blocking worker safely. Waiting two seconds and then joining it can wait forever. Dropping its handle allows it to continue writing during shutdown.

**Replace the shutdown rule with:**

> Shutdown first rejects new external work, completes or cancels outstanding invocations, and stops timers. Jobs use cooperative cancellation and finite transport deadlines. After the bounded grace period, late results are rejected. Workers may not directly write framework settings or checkpoints; those use an ordered persistence lane. The process-exit policy for unresponsive workers is explicit and does not claim they were killed or rolled back.

Add bounded job/command queues, overload responses, panic isolation and UI-drain fairness. Keyed jobs must be scoped by feature and relevant document/window/entity; a global `"refresh"` key is insufficient.

“No async runtime” is acceptable; unbounded blocking and queueing are not.

### B7. Persistence can lose newer edits — §§6, 2.1

Concurrent checkpoint writes can complete backwards. A failed settings write cannot safely “put the in-memory value back” after further edits. File `fsync` plus rename alone does not establish durable directory-entry replacement on relevant platforms.

**Replace the persistence semantics with:**

> Writes are serialized per destination and tagged with monotonic revisions. Completion or failure affects only the corresponding revision; older results never replace newer state. Settings distinguish desired and durably committed values. Failure preserves pending edits and reports unsaved state; it does not blindly roll back the model.

> Atomic replacement uses a same-directory temporary file, appropriate permissions, file synchronization, replacement, and parent-directory synchronization where supported. Corrupt or future-schema files enter protected/read-only recovery; ordinary saves cannot overwrite them without explicit recovery.

> Polling uses a change fingerprint rather than mtime alone, handles replacement/deletion, and suppresses self-write echoes. External edits conflicting with pending local edits produce a conflict, not silent last-writer-wins.

Define ownership of settings now: the store owns committed state; model settings change through commands; write and external-change results return as commands.

### B8. Close and stop contradict the mutation/effects rules — §§2.1, 2.2, 6

`Close::Later` cannot itself open confirmation UI or start saving. `stop(&mut Model)` mutates outside `update`, has no effects argument, and runs after the loop that would deliver write results.

**Replace both hooks with:**

> `Event::CloseRequested { window }` and `Event::ExitRequested` are translated to commands. Update changes confirmation state and requests save effects. Successful save completion may issue the close/exit effect. Cancellation leaves the window open. Final checkpoints are requested while command processing is still available. After event-loop termination, cleanup performs no application-model mutation.

Add a bounded logout/session-end policy; logout is not equivalent to a completed interactive save.

### B9. Intent metadata is not a local invocation binding — §3

`add_location(name)` cannot run from Ctrl+L without an argument. Availability, focus and text-input shortcut precedence are also missing.

**Replace the example binding with:**

> Parameterized intents require explicit local argument acquisition. Ctrl+L invokes `FocusLocationEntry`; submission invokes `AddLocation { name }`. Menus and shortcuts may launch an argument-collection workflow rather than execute an incomplete intent. Availability is shared, but authorization remains authoritative at execution. Shortcuts respect window focus, editable controls and platform conventions.

Also replace `row as usize` with checked conversion and stable item identity. List indices are not durable command targets.

### B10. Initialization and instance identity are incomplete — §2.2

A second profile currently shares the same claimed instance and socket. Publishing a surface before the event loop is ready can admit requests that cannot be processed. Manifest declaration failures cannot be debug-only.

**Replace the relevant lifecycle text with:**

> Validate manifests and intent declarations in every build. Resolve profile identity before claiming the instance; the claim and endpoint are keyed by `(AppId, ProfileId)`. Publish readiness only after command processing is available. Launch handoff has an acknowledgement and request identifier, so retry cannot duplicate a launch. Startup failure unwinds claims, sockets and resources.

Define whether `Boot` merely exposes launch data or applies it; do not also apply the same launch through `Event::Launch`.

## 2. First-year gaps

### Decide now

- **Large lists:** retained UI-side `ModelRc` adapters, row-level notifications, stable IDs, bounded paging and cancellation. Do not rebuild entire lists after every command. Surface pages need revision/cursor semantics.
- **Undo:** app/domain-owned history of committed user operations, not inverse commands inferred by the framework. External irreversible effects are explicitly non-undoable.
- **Desktop integration:** one typed effect/service boundary for file pickers, portals, clipboard, drag-and-drop and opening URLs. Results return as commands; selected resources may be capability handles, not unrestricted paths.
- **Launch/deep links:** typed, versioned launch requests with untrusted-input validation and explicit profile/window targeting.
- **Data upgrades:** versioned application-data and draft formats, migration ownership, crash recovery and future-version refusal. “App-owned JSON” does not answer these.
- **Accessibility/localisation:** semantic accessible labels, keyboard/focus contracts, translatable message IDs with arguments, locale-aware formatting and RTL readiness. Replace “`Display` is the sentence” with structured public errors plus localized rendering. Diagnostics remain separate.
- **Privacy/telemetry:** default to local, redacted diagnostics; no network telemetry without explicit policy/consent. Define retention for problem records and exclude document content, credentials and raw arguments.
- **Testing:** inject paths/profile configuration rather than mutating process environment in parallel tests. Add a manually scheduled fake executor: inline jobs cannot test races, replacement or shutdown.

### Can wait

Printing implementation, rich drag previews, advanced undo grouping, streaming-token transport and inotify. Reserve service/effect boundaries now; do not build all implementations before Weather.

## 3. Merge choices

- **No async runtime:** accept, subject to B6. Streaming does not inherently require app-owned async; bounded worker-to-command streams can work.
- **Per-app `Command` enum:** accept. It improves exhaustive routing without forcing a framework-wide enum.
- **Existing dispatch ahead of intents:** accept only with B5’s invocation context and dequeue-time checks. A second authorization system is unnecessary.
- **Mtime polling:** accept polling, reject mtime-only detection and undefined conflicts; B7 is the minimal repair.
- **Buttons directly calling commands:** accept only for trusted local events. Rail/proposal execution is not a button-shaped exemption from authorization.

## 4. Migration and pilot

PR 2 is not “M”; it is most of the framework and its test architecture. Split it into independently gated changes:

1. Mechanical extraction and compatibility tests; check for re-export dependency cycles.
2. Minimal runner, two-crate Slint adapter proof, command queue and virtual executor.
3. Invocation/provenance/settlement and real-dispatch tests.
4. Ordered persistence, recovery and lifecycle.
5. Typed services and rail.

Introduce architecture checks **with the first migrated feature**, not at final cleanup.

Weather should first migrate with behavioral parity. Its redesign is a separate PR; otherwise regressions cannot be attributed to framework versus presentation changes.

Before Notes, Weather must prove: stale-result rejection, worker panic, queue overload, timeout, shutdown during refresh, duplicate launch and unauthorized proposal rejection.

Notes is a **framework acceptance gate**, not merely the next app. Require save failure, interrupted migration, conflicting external edits, close cancellation, two document contexts and recovery after forced termination.

Add a large-list test fixture before Calendar/Email. Run at least one real socket/credential/approval integration suite; a registry-only harness cannot prove transport identity.

Remove compatibility APIs only after every consumer has migrated and the promised support interval has elapsed.

## 5. Required document edits before approval

1. Replace the single-window/process-lifetime contract using B1.
2. Specify and compile-prove the Slint adapter and delivery bridge using B2.
3. Define commit, refusal and completion semantics using B3–B4.
4. Bind authorization and provenance through execution using B5.
5. Replace worker/shutdown promises using B6 and B8.
6. Define ordered persistence and conflict handling using B7.
7. Correct invocation bindings and startup identity/readiness using B9–B10.
8. Adopt the “decide now” contracts and gated migration sequence above.

These are contract repairs, not requests for a larger framework. Without them, the twentieth app inherits undocumented exceptions rather than reusable machinery.
