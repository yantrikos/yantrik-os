# Recommendation

Yantrik does not need a new framework. It needs **enforced ownership boundaries, one feature architecture, a real design-system contract, and smaller compilation units**.

The strongest existing foundations are worth preserving: typed IPC, a shared control-surface protocol, pure Rust model functions, the UI kit, incremental releases, and mandatory security reviews.

The main problems are structural:

- `yantrik-ui` is both the composition root and the implementation of too many domains.
- Slint globals and wiring closures have become an implicit application architecture.
- The design system supplies controls but does not yet govern complete interaction patterns.
- The largest generated UI compilation unit is also the broadest change boundary.
- Security-sensitive flows cross several modules without sufficiently obvious ownership.
- Tests sometimes verify implementation spelling instead of behavior.

**Do not replace 38 crates with 100 speculative crates.** First establish feature boundaries inside crates. Extract a crate when it creates a meaningful dependency, testing, reuse, security, or compilation boundary.

The recommendations below are based on the supplied inventory; Slint compilation changes in particular need a measured prototype before committing to a split.

---

# 1. Target architecture: the one-page version

## Dependency rule

**Executables compose; libraries implement; contracts describe.**

A dependency arrow may point toward a lower layer, never toward a consumer. Feature siblings do not import one another’s internals. Cross-process interactions use contracts and authenticated transport, not implementation-crate dependencies.

```text
                    Composition roots
        shell binary     app binaries     service binaries
              │                │                 │
              ▼                ▼                 ▼
       Shell features     App features      Service / Mind domains
              │                │                 │
              ▼                ▼                 ▼
       Presentation + runtime adapters     Infrastructure adapters
              │                │                 │
              └──────────┬─────┴─────────────────┘
                         ▼
            Domain types, policy, contracts
                         ▼
                  Small foundations

UI source dependency direction:
screens → templates → patterns → components → primitives → tokens

Generated UI adapters depend on screen models.
Screen models never depend on Slint or generated UI types.
```

The UI stack and service/Mind stack are **parallel consumers**, not a sequence in which the Mind depends on the shell.

## Crate ownership

| Layer | Crates | Single responsibility |
|---|---|---|
| Contracts | **Existing** `yantrik-ipc-contracts` | Versioned wire messages, IDs, and protocol errors. No transport or service implementation. |
| Contracts | **Existing** `yantrik-surface` | Typed describe/act vocabulary, semantic target IDs, capabilities, action/result schemas. No shell-specific controls. |
| Security domain | **New** `yantrik-security-core` | Pure policy evaluation and types for principals, approval requirements, ceilings, and bound action intents. No UI, network, or privileged execution. |
| Transport | **Existing** `yantrik-ipc-transport` | Framing, connection lifecycle, peer-credential acquisition, cancellation, and transport failures. |
| Client adapters | **New, extracted incrementally** `yantrik-service-clients` | Typed service clients, reconnect behavior, subscriptions, deadlines, and idempotency-aware retries. |
| Platform adapters | **New when extracted** `yantrik-platform` | Narrow Linux/Wayland integration, initially the labwc window-management adapter. Not a replacement service layer. |
| Design system | **Existing** `yantrik-design-tokens`, `yantrik-ui-kit` | Theme vocabulary and reusable visual/interaction building blocks. |
| Presentation | **New** `yantrik-presentation` | Shared user-facing formatting, provenance-aware text presentation, and locale-aware display policies. No Slint or service access. |
| App runtime | **Existing** `yantrik-app-runtime` | App lifecycle, window bootstrap, UI-thread handoff, and control-surface registration. Not app business logic. |
| Shell domain | **New** `yantrik-shell-model` | Slint-free shell feature states, reducers, projections, and semantic surface descriptions. |
| Shell UI | **Existing, narrowed** `yantrik-ui`; split UI build units where viable | Shell composition, Slint adapters, effect execution, and window lifecycle. |
| Apps | **Existing** `apps/*` | App-specific features and composition. They do not import shell implementation crates. |
| Services | **Existing** `services/*` | Authoritative background state and privileged operations for their domains. |
| Mind | **Existing** companion core/cortex/instincts/tools and companion executable | Core: domain model; cortex: planning/model orchestration; instincts: proactive triggers; tools: execution adapters; executable: composition. |
| Harness | **Existing** `yantrik-harness` | Third-party mind lifecycle and protocol adaptation under explicit capability constraints. |

For the Mind, enforce approximately:

```text
companion executable → cortex / instincts / tools / harness
cortex / instincts / tools → companion-core + contracts
```

Planning code should depend on a small tool-execution interface, not concrete tool implementations. Third-party harness output remains untrusted input, not authenticated authority.

### Security is not another shell feature

`yantrik-security-core` makes policy consistent; **it is not itself a security boundary**.

- The shell presents approval requests and conveys user decisions.
- The service or broker performing an operation authorizes it.
- Authenticated ingress derives caller identity from kernel credentials and the trusted identity registry—not request fields.
- Approval grants bind to the intended operation, relevant arguments, principal, expiry, and replay policy.
- The executor revalidates authorization and relevant resource state immediately before the side effect.
- Egress and vault access remain at their existing trusted boundaries.
- Delegation preserves the original principal and limits; a shell or Mind proxy must not silently upgrade authority.

Where a domain service enforces authorization itself, do not add a second broker merely to satisfy an architecture diagram.

---

# 2. The UI component library

## One kit, five tiers

Keep one `yantrik-ui-kit` initially. Tiers are directories and API categories, not five new crates.

```text
yantrik-design-tokens/
  schema/
  themes/
  generated/theme.slint

yantrik-ui-kit/
  ui/
    primitives/
    components/
    patterns/
    templates/
  src/                 # preview support and small Rust adapters
  catalog/
  tests/
```

| Tier | Contains | Must not contain |
|---|---|---|
| Tokens | Semantic colors, typography, spacing, radii, motion, elevations | Screen-specific names such as `email-blue` |
| Primitives | Text, icon, focus ring, surface, separator, spacing/layout helpers | Service calls or domain actions |
| Components | Button, input, badge, chip, list-row base, tabs, dialog frame | Approval authority or application state |
| Patterns | Search/filter bar, selectable list, empty/error/loading states, inspector layout | Feature-specific business rules |
| Templates | App frame, list/detail, settings, reading, conversation, dashboard layouts | Email/calendar/Mind implementations |

Make semantic tokens primary: `text-primary`, `surface-raised`, `status-warning`, `focus-ring`. Palette values are an implementation detail.

Keep TOML as the authoring source if practical, with a schema and deterministic generation of Slint tokens. Do not maintain independent handwritten Rust, Slint, and TOML theme definitions.

## The component contract

Every public component must provide:

1. **A documented API**
   - Properties, callbacks, defaults, slot behavior, and ownership of mutable state.
   - No hidden access to screen globals.
   - Callback names express intent: `activated`, `dismiss-requested`, `selection-changed`.

2. **An explicit state matrix**
   - Applicable states: normal, hover, pressed, focused, disabled, selected, loading, error.
   - Long text, empty text, missing icons, narrow layouts, and content overflow.
   - Light/dark and supported text scales.

3. **An interaction and accessibility contract**
   - Keyboard operation, focus order, focus restoration, visible focus, accessible name/role/value where supported.
   - Label association for fields.
   - No hover-only essential actions.
   - Reduced-motion behavior for animated components.
   - Test Slint/backend accessibility capabilities on the shipped Wayland stack; document and track gaps rather than assuming markup guarantees accessibility.

4. **A semantic control-surface mapping**
   - Document which user intents it exposes and what state is observable.
   - Accept a stable semantic target key where needed.
   - Emit intents; never dispatch IPC or authorize actions itself.
   - The containing feature supplies domain identity and typed actions.

5. **A catalog entry and tests**
   - Representative state fixtures, screenshots, keyboard tests, and usage examples.
   - An owner and deprecation policy.

A decorative card does not need an agent action. A selectable row does. **The control surface describes product semantics, not the Slint widget tree.**

## Missing components: build these first

The audit suggests these families—not 124 new public components:

| Missing family | Concrete additions |
|---|---|
| Actions | `YIconButton`, `YToggleButton`, `YSplitButton`; standard destructive/loading variants of `YButton` |
| Rows | `YListRow`, `YSelectableRow`, `YSettingsRow`, `YActionRow`, `YMetadataRow` |
| Labels | `YBadge`, `YStatusBadge`, `YFilterChip`, `YRemovableChip` |
| Containers | `YCard`, `YSectionHeader`, `YInsetGroup`, `YDisclosure` |
| Feedback | `YEmptyState`, `YErrorState`, `YLoadingState`, `YInlineNotice`, `YProgressState` |
| Input patterns | `YSearchField`, `YSearchFilterBar`, `YValidationMessage` |
| Keyboard/navigation | The consolidated `YKeycap`, `YShortcutHint`, menu-item and selection patterns where absent |
| Layout | `YReadingColumn`, `YListDetail`, `YSettingsPage`, `YInspectorPanel` |
| Trust presentation | `YProvenanceLabel`, `YRiskSummary`, a visual `YApprovalCard` frame |

Do not add an “everything row” with thirty booleans. Start with a row primitive with controlled content slots, then a handful of opinionated patterns.

`YApprovalCard` renders structured data and user choices. The approval feature owns the state machine; the executor owns authorization.

## “Screens only use the kit” needs a precise meaning

Screens may use Slint layout constructs and **private domain compositions**. They may not redefine generic controls.

An `EmailMessageRow` that composes `YListRow`, `YBadge`, and tokenized text is legitimate. A private `RoundedBlueButton` is not.

Adjust the lint accordingly: prohibit private **generic control implementations**, not all private components. Otherwise the lint encourages giant, unreadable screen files.

## Stop drift beyond lint

- Put the component catalog and `tests/ui-preview` into the workspace and CI.
- Require an existing-kit assessment before accepting a new generic component.
- Give the kit an owner and require review for new variants or token names.
- Use visual regression review, not just syntax checks.
- Track duplicate removal as a migration backlog.
- Maintain brief usage guidance: “use `YStatusBadge` for state; use `YFilterChip` for selectable filters.”
- Use temporary, owner-assigned exceptions with expiry dates. No permanent “legacy screen” blanket exemption.

---

# 3. One Rust ↔ Slint feature pattern

## Adopt state → action → transition → effect

For every screen or coherent feature:

```text
Authenticated service events ─┐
Slint callbacks ───────────────┼→ Action → transition(State, Action)
Control-surface requests ──────┘                  │
                                      new State + Effects
                                                │
                                  pure projection to ViewModel
                                                │
                                     thin Slint rendering adapter

Effects → async adapters → result Actions
```

This is a small application pattern, not a new framework. Use ordinary Rust enums and functions. Do not build a universal reactive runtime or custom UI DSL.

### Example feature layout

```text
yantrik-shell-model/src/approvals/
  mod.rs          # narrow facade
  state.rs
  action.rs
  transition.rs
  effect.rs
  view_model.rs
  surface.rs
  tests.rs

yantrik-ui/src/features/approvals/
  mod.rs
  binding.rs      # Slint callbacks and VM → generated-type mapping
  effects.rs     # calls authenticated clients
```

Conceptually:

```rust
pub fn transition(
    state: &mut ApprovalState,
    action: ApprovalAction,
) -> Vec<ApprovalEffect>;

pub fn view_model(
    state: &ApprovalState,
    context: &PresentationContext,
) -> ApprovalViewModel;

pub fn describe(
    state: &ApprovalState,
) -> SurfaceDescription;
```

The transition function does not read the clock or generate randomness itself. Supply times, IDs, and external results through actions or explicit inputs.

## State ownership

**Rust owns domain state and asynchronous workflow state.**

Slint may own transient view state:

- Hover and animation progress.
- Focus/scroll state when it need not be persisted or agent-observable.
- Temporary editing buffers, with an explicit commit/update contract.

Slint must not own:

- Whether an action is authorized.
- Approval validity.
- Business validation.
- Service synchronization rules.
- Durable settings.
- Tool-execution state machines.

Avoid two-way domain properties unless there is a documented owner and reconciliation rule. Prefer input properties plus intent callbacks.

Limit globals to deliberately global presentation concerns, such as theme. A Slint global is not an application service locator.

## Where logic belongs

| Logic | Location |
|---|---|
| Authorization, ceilings, approval requirements | Security policy and authoritative executor |
| Feature workflow and state transitions | Rust feature model |
| Sorting, filtering, row construction | Pure Rust projection/model |
| Locale/time/provenance presentation | Presentation library plus feature projection |
| Network, persistence, timers | Effect adapters |
| Callback registration and generated-type conversion | Binding module |
| Layout, visual states, animation | Slint |

Delete rules embedded in large callback closures. A callback should decode input, dispatch an action, and return.

## Control surface and visual UI share semantics

A feature owns one typed set of domain commands. Both UI and agent adapters map into those commands, with appropriate origin and authorization context.

Important distinctions:

- Observing state and changing state are different capabilities.
- UI-only events need not become remote commands.
- An agent must not acquire “trusted user gesture” status by invoking the same command.
- A disabled button is not authorization.
- Describe output may be stale: an action carries stable target identity and, where appropriate, an expected revision; the handler revalidates it.

Maintain a capability matrix linking user-visible workflows to typed agent operations or documented exclusions. This catches control-surface omissions without exposing internal widgets.

## Efficient rendering

Do not replace every `VecModel` immediately.

First:

- Project only affected feature state.
- Keep stable model instances.
- Update changed rows using supported model notifications.
- Use stable domain IDs, never row indices as business identity.
- Batch high-frequency updates and hand them to the Slint event loop.
- Associate asynchronous results with request/generation IDs so late results cannot overwrite newer state.
- Cancel feature tasks when their lifetime ends.

Introduce a specialized model only after profiling demonstrates a need.

## Breaking up `yantrik-ui-slint`

**Splitting files is not splitting compilation units.**

Slint source imports can still pull an entire reachable component tree into the same generated Rust unit. And generated Rust from one crate should not be assumed to be a reusable Slint markup component package.

Do this in order:

1. **Measure the actual graph.**
   Record generated-source size, clean/incremental wall time, peak RSS, and which entry points import which screens.

2. **Remove umbrella imports and unused roots.**
   Each build entry should compile the smallest required reachable UI.

3. **Separate genuinely independent windows/processes first.**
   Candidate build crates:
   - `yantrik-shell-ui`
   - `yantrik-switcher-ui`
   - `yantrik-settings-ui`, **only if settings is independently hosted**
   - App-local generated UI for each `apps/*`

   Renderer choice remains a host/runtime concern; preserve the switcher’s software-renderer behavior.

4. **Prototype cross-unit interaction on Slint 1.17.**
   Prove imports, generated types, callbacks, globals, resources, and incremental rebuild behavior. Do not promise that arbitrary embedded pages can simply move into separate Rust crates.

5. **Treat an inseparable shell root honestly.**
   If screens are embedded in one Slint component tree, moving them into separate windows/processes is a product and lifecycle change. Otherwise reduce reachable source and generator complexity, and investigate upstream compiler options with measurements.

6. **Bound build concurrency.**
   Multiple 14 GB rustc processes can exceed the host even when each crate is smaller. Set measured Cargo job limits and CI memory budgets, leaving room for linking and the OS.

Share kit **source APIs**, not generated screen structs across crate boundaries. Use ordinary Rust view models and explicit adapters.

---

# 4. Shared libraries beyond UI

Create shared libraries from demonstrated duplication—not from a wish list.

| Concern | Recommended home | Rules |
|---|---|---|
| Relative time, dates, sizes, truncation | `yantrik-presentation` | Explicit locale/time zone/clock inputs; no scattered `"{} ago"` formatting |
| User-facing text cleaning | `yantrik-presentation::text` | Name exact transformations; distinguish preview truncation, control-character handling, and display escaping |
| Untrusted model content | Typed provenance in relevant contracts/core types | Never pretend “cleaned text” is trusted or prompt-injection-safe |
| IPC lifecycle | `yantrik-ipc-transport` | Transport mechanics only |
| Service client behavior | `yantrik-service-clients` | Deadlines, cancellation, subscriptions, reconnect; no automatic replay of mutations without an idempotency contract |
| Settings/config | New `yantrik-config` | Typed schema, validation, migrations, defaults, atomic file replacement; distinguish defaults from security policy |
| Persistence | New `yantrik-storage`, only after a second real consumer | Connection setup, transactions, migration plumbing, safe file operations—not a universal repository abstraction |
| Errors | Domain-local error enums | `thiserror` for library errors; contextual reports at executable boundaries; stable IPC error codes |
| Logging | New `yantrik-observability` | `tracing` setup, correlation IDs, redaction helpers, consistent fields |
| Localization | New `yantrik-i18n` if multiple consumers require it | Locale negotiation, catalogs, plural/date rules; prototype Rust/Slint integration before selecting bindings |

Additional rules:

- Do not create `yantrik-utils`, `common`, or one global `YantrikError`.
- Operational logs are not security audit records. Record authorization and approval decisions through the responsible boundary’s explicit audit path.
- Never log secrets, vault payloads, or full prompts by default.
- Services own their schemas and migrations. A persistence helper must not turn the shell into a direct reader of every service database.
- Clock abstractions should be small and local unless truly shared.

## Resolve `yantrik-ml` immediately

This is a supply-chain and reproducibility problem as well as duplication.

1. Inspect `cargo metadata`, resolved package IDs, dependency trees, revisions, and all patch tables.
2. Identify exactly which copy each shipped binary uses.
3. Choose **one canonical repository/package identity**, preferably the dedicated sibling `yantrik-ml` repository.
4. Port any companion-only changes there and test API/model compatibility.
5. Make companion and workspace consumers resolve to the same approved revision.
6. Remove the duplicate source tree and obsolete patches only after that resolution is verified.
7. Make local sibling overrides explicit developer tooling; verify releases with a clean checkout and canonical dependencies.

Do not leave release behavior dependent on which sibling directories happen to exist. Check in the application lockfile and produce dependency/license/security inventories from the resolved release graph.

---

# 5. Module, file, and API rules

## Size budgets

Treat these as review thresholds, not incentives to mechanically split code.

| Item | Target | Required intervention |
|---|---:|---|
| Production Rust file | Usually under 400 lines | Over 600 requires decomposition review |
| Slint component file | Usually under 300 lines | Over 500 requires decomposition review |
| `main.rs` | Under 150 lines | Bootstrap only |
| Callback closure | Usually under 15–20 lines | Extract behavior if it branches into a workflow |
| Public facade | Small and intentional | No blanket reexports of implementation modules |

Exclude generated code, fixtures, and large declarative tables. Review large test files separately.

For existing violations, CI should reject **growth beyond a recorded baseline**, not block all changes. New 4,000-line files are forbidden; touching an old one does not require rewriting it.

## Naming and visibility

- Name by domain: `approvals`, `window_switcher`, `notifications`.
- Within features, use consistent role names: `state`, `transition`, `view_model`, `surface`, `binding`, `effects`.
- Prefer private visibility; use `pub(super)` and `pub(crate)` deliberately.
- Public APIs expose domain types, not generated Slint structs, database handles, or transport internals.
- Feature `mod.rs` files are facades.
- No sibling imports such as `features::agents::store::internal_helper`.
- Introduce traits at real effect/replacement boundaries, not for every struct.
- No public “manager” object that accumulates unrelated responsibilities.

## Target shell tree

```text
crates/yantrik-ui/src/
  main.rs

  app/
    mod.rs
    bootstrap.rs
    composition.rs
    lifecycle.rs

  runtime/
    mod.rs
    dispatch.rs
    tasks.rs
    ui_thread.rs

  platform/
    mod.rs
    labwc.rs
    windows.rs

  services/
    mod.rs
    subscriptions.rs
    companion_events.rs

  control/
    mod.rs
    router.rs
    authenticated_context.rs
    registration.rs

  features/
    approvals/
      mod.rs
      binding.rs
      effects.rs
    dock/
      mod.rs
      binding.rs
      effects.rs
    window_switcher/
    notifications/
    mind_panel/
    agents/
    lens/
    settings/
    launcher/

  ui/
    mod.rs
    generated.rs
    conversions.rs

  diagnostics/
    mod.rs
    health.rs
```

These feature directories contain adapters; pure feature implementation lives in the matching modules of `yantrik-shell-model`.

Extract `platform/labwc.rs` into `yantrik-platform` only once the boundary is stable or reused.

### Map the worst files explicitly

- `control_approvals.rs` → approval surface adapter, typed commands, approval-policy calls, contract tests.
- `approvals.rs` → approval state machine/projection plus shell effects.
- `control.rs` → routing and registration only; operations move to features.
- `wire/*.rs` → feature-local bindings. Delete the global wiring layer.
- `bridge.rs` → typed companion client plus event-to-action translation. No domain decisions.
- `mind_mode.rs` → pure mode policy and state transitions plus thin rendering adapter.
- `agents/store.rs` → owned repository/transaction boundary and a separate presentation projection.
- `companion.rs` → bootstrap plus domain modules; no all-knowing companion object.
- `harness/host.rs` → process lifecycle, protocol session, capability policy, and event adapter.
- `recipe_executor.rs` → execution state machine, checkpointing, tool dispatch, and authorization adapter.
- Large app `main.rs` files → the same feature pattern used by the shell.

Do not perform all these moves simultaneously. A mechanical move and a behavioral redesign should normally be separate PRs.

---

# 6. Testing and enforcement

## Test pyramid

| Layer | Tests |
|---|---|
| Pure feature model | Transitions, projections, sorting/filtering, stale results, cancellation, error recovery |
| Security policy | Table-driven and property tests for ceilings, principals, approvals, delegation, expiry, replay, changed arguments |
| Components | State-matrix snapshots, keyboard/focus tests, light/dark, text scale, long/localized text |
| Feature integration | Real model + fake effects + UI binding; callback dispatch and observable state |
| IPC/control surface | Schema compatibility, typed action round trips, capability coverage, target stability, revision handling |
| Authenticated transport | Real Unix-socket peer credentials, disconnects, reconnects, malformed inputs, impersonation attempts |
| Services/storage | Real migrations, transaction rollback, crash recovery, authorization at execution |
| VM | End-to-end workflows, compositor integration, renderers, security-negative scenarios |

Add fuzzing at parsers and untrusted ingress: IPC decoding, surface action payloads, harness messages, and model-generated tool arguments.

### Visual testing

Bring the preview harness into the workspace now.

Use:

- Pinned fonts, locale, clock, assets, and renderer settings.
- A deterministic software-rendered PR baseline.
- Text-scale and theme matrices for the kit.
- Scheduled/release GL and software-GL validation in the VM.
- Explicitly reviewed image diffs with controlled tolerance.

Keep the existing VM screenshot collection as reference material, but curate it into named scenarios. **A directory of 520 screenshots is not a regression suite.**

Continue the required VM 520 check on every PR while gradually automating critical flows: shell launch, window switching, notifications, approvals, app control actions, and recovery after service restart.

Test the carried labwc patch against the packaged compositor version, including window identity and control behavior.

## Source-reading tests: keep a small subset

Keep source/static checks for architectural facts:

- No forbidden imports.
- No raw token bypasses.
- No unauthorized direct vault/egress clients.
- No new dependency cycles or forbidden edges.
- No direct generic-control reimplementations.

Prefer syntax-aware checks over fragile substring matches.

Replace source-reading tests that assert:

- A particular callback spelling.
- A specific helper invocation.
- A screen’s internal component nesting.
- Security based on the presence of a conditional.
- Business behavior inferred from source text.

Replace them with behavior, contract, rendering, or authorization tests.

A source test may prove that code mentions an approval check. It cannot prove that an operation cannot execute without approval.

## CI gates

Implement one workspace tool, for example `cargo xtask check-architecture`, rather than many unrelated scripts.

It should enforce:

1. **Dependency allowlist** from `cargo metadata`, including allowed features.
2. **Module boundaries** for the remaining monolith, through import checks and facade discipline.
3. **File-size ratchets** with reviewed exceptions.
4. **Design-system lint**, including exception expiry.
5. **Public API changes** for published/shared crates; review snapshots and semver checks where applicable.
6. **IPC schema compatibility**, including one-version skew where independent restarts/upgrades can create it.
7. **Control-surface capability coverage**.
8. **Formatting, Clippy, tests, and reproducible generation**.
9. **Dependency/license/security checks** and canonical ML resolution.
10. **Scheduled compile-time/RSS tracking**, with calibrated hard budgets after collecting stable measurements.

Split CI into fast PR gates and heavier scheduled/release matrices, but do not weaken the existing per-PR security review or VM requirement.

---

# 7. Prioritized incremental migration plan

Sizes describe implementation scope, not elapsed-time promises.

| # | Step | Size | Risk | Unblocks | Nightly-safe approach |
|---:|---|---|---|---|---|
| 1 | Record dependency rules, owners, build/RSS baseline, and current exceptions | Small | Low | Objective enforcement | Add reports first; fail only new violations |
| 2 | Resolve the duplicate `yantrik-ml` and canonical dependency graph | Medium | Medium | Reproducible builds | Preserve selected behavior, test companion compatibility, then remove duplicate |
| 3 | Move UI preview harness into workspace/CI | Small–medium | Low | Safe kit consolidation | Start with existing stable fixtures; expand coverage |
| 4 | Add missing row/badge/card/layout families and consolidate obvious duplicates | Medium | Low–medium | Rapid consistency gains | Migrate one family/screen per PR; retain temporary aliases |
| 5 | Make domain component exceptions explicit; add facade and size ratchets | Small | Low | Stops renewed drift | Baseline legacy code; no repository-wide cleanup requirement |
| 6 | Pilot the feature pattern on a bounded feature, such as notification-center presentation | Medium | Medium | A proven reference implementation | Preserve IPC and UI contracts; compare old/new projections offline |
| 7 | Measure and prototype the first real Slint compilation split | Medium | Medium | Lower build memory/rebuild scope | Use an already independent window or app; require measured improvement before expanding |
| 8 | Extract `yantrik-shell-model`; migrate dock, launcher, and window-switcher logic | Large | Medium | Slint-free tests and smaller wiring | One feature at a time; temporary adapter facades |
| 9 | Extract typed service clients and shared presentation helpers | Medium | Medium | Removes repetitive reconnect/formatting code | Preserve public client APIs; distinguish safe reads from non-replayable mutations |
| 10 | Map approval/authorization ownership and extract the pure policy core | Large | High | Clear, reviewable security flows | First characterize existing behavior; run side-effect-free decision comparisons; migrate one operation family at a time |
| 11 | Unify each migrated feature’s typed UI/control command path | Medium–large | High | Complete, consistent agent control | Preserve caller origin; retain protocol adapters; require negative authorization tests |
| 12 | Move remaining shell modules into feature ownership and shrink large app entry points | Large | Medium | A navigable codebase | Mechanical moves first, workflow refactors second; nightly after each feature |
| 13 | Split companion/harness execution flows and remove superseded paths | Large | High | A maintainable Mind and smaller trusted flows | Move one execution state machine at a time; checkpoint/recovery and capability tests before deletion |

Two migration rules apply throughout:

- **Never dual-execute side effects.** Shadow comparisons are for projections and authorization decisions only.
- Keep compatibility adapters short-lived, named, owned, and scheduled for deletion. Do not let the migration produce two permanent architectures.

For security-sensitive cutovers, rollback must not restore a known authorization bypass. Keep protocol compatibility separate from permission-policy rollback.

---

# 8. What to delete or stop doing

## Delete as replacements land

- The noncanonical `yantrik-ml` tree and obsolete patch entries.
- Private generic buttons, badges, chips, cards, and keycaps.
- Repeated reading-column and other standard layout math.
- The global `wire/` namespace.
- `control_*` files that duplicate feature behavior.
- Slint domain-state globals and competing Rust/Slint sources of truth.
- The all-screen umbrella import/build entry where it is unnecessary.
- Callback-local business workflows.
- Source-reading tests superseded by meaningful tests.
- Dead screens, unused kit variants, stale compatibility adapters, and unreferenced assets.

## Stop doing

- Adding another feature directly to `main.rs`, `bridge.rs`, or `companion.rs`.
- Treating “button disabled” as authorization.
- Letting an agent-controlled field claim caller identity.
- Letting generic text cleanup imply prompt-injection safety.
- Automatically retrying non-idempotent mutations after reconnect.
- Giving the shell direct access to service databases or vault internals.
- Exposing generated UI types as shared application APIs.
- Updating entire models on every event without measuring or preserving identity.
- Creating a crate merely because a file is large.
- Creating generic `Manager`, `Utils`, or repository frameworks before a second concrete need exists.
- Claiming compile-memory improvement from directory moves.
- Treating “enterprise grade” as maximum abstraction.

## Definition of done

The architecture is working when:

- A new screen follows one reference feature pattern and composes kit components.
- Its behavior and control-surface semantics can be tested without starting Slint.
- Its UI and agent actions converge on typed domain commands without conflating identity.
- A reviewer can find the authoritative authorization check for every risky effect.
- An app can reuse presentation, runtime, and clients without importing the shell.
- A routine screen edit does not unnecessarily rebuild unrelated windows and apps.
- Large files and exceptions shrink over time.
- The shipped dependency graph has one unambiguous ML implementation.

That is the goal: **less code to understand per change, fewer places where policy can diverge, and smaller boundaries to build, test, and trust.**
