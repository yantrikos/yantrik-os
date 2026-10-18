# GPT-6 Astra's review of the Alt+Tab design (18 Oct 2026)

Asked to red-team the first draft of `alt-tab-deep-2026-10-18.md` as a Wayland compositor engineer and security reviewer. Its answer, unedited except for quote characters. The design's last section says what changed because of it.

---

**Verdict: keep the separate renderer and tiles-first rollout. Put chord ownership, selection, and fallback in labwc. Do not ship prediction or approval prioritisation as the default Alt+Tab order.**

### 1. Step 0: plausible held-chord experiment, not proof of correctness

**Likely to work when Alt remains held until the surface gets focus. Not reliable as the complete design.** Exclusive keyboard interactivity needs an appropriate top/overlay layer and actual compositor focus; mapping alone is not proof.

Important distinctions:

- The binding’s triggering **Tab press is handled by labwc**, not delivered to the newly launched client. The client must initialise the first selection from the invocation.
- `wl_keyboard.enter.keys` is a pressed-key snapshot, **not a modifiers field**. Modifier state comes through `wl_keyboard.modifiers`; wlroots’ keyboard-focus machinery supplies current modifiers when entering a surface.
- Focus gained mid-chord does not replay the original presses. A later release can arrive without a corresponding press on that surface.
- If Alt comes up **before mapping/focus**, there is no historical release to deliver. Fixing modifier forwarding cannot recover it.

**Source-certainty caveat:** I cannot certify labwc **0.8.3’s exact bound-key release filtering** without inspecting that tag’s keyboard handler. Do not turn either “all releases arrive” or “labwc never sends Alt-up” into an architectural assumption. Trace handled/bound keys, keyboard focus, and modifier notifications in labwc—not just client callbacks. The distinction between swallowing a bound key’s release and suppressing modifier updates matters.

**Better design:** labwc owns a switch session from the initial keypress through release/cancel. It sends a persistent renderer session ID, candidates and selection changes. Labwc alone commits activation. This also handles Alt released before the renderer appears.

`onRelease`, **if supported by this tag**, is not automatically “when Alt releases”: for an Alt+Tab binding, establish whether it means the bound **Tab’s** release. It does not solve an arbitrary hold-and-cycle interaction. A release timer is not equivalent to release detection.

Test both Alts, reverse release order, repeats, Shift+Tab, Alt released before map, lock during switching, renderer death, and existing keyboard grabs.

### 2. The fallback is neither native nor race-free

`wlr-foreign-toplevel-management.activate` is an **activation request**, not labwc’s native switcher. It does not provide the compositor’s selection state, authoritative MRU policy or guaranteed activation.

Failure examples:

- At 151 ms the helper switches; at 155 ms the renderer maps and steals keyboard focus; Alt release switches again.
- A renderer acknowledges readiness, then hangs before presenting anything.
- Repeated Tab presses launch multiple helpers with conflicting snapshots of “previous”.
- A delayed activation lands after Escape, a lock transition, or a newer switching session.

**Use one compositor-owned session and one commit authority.** At the deadline, labwc revokes the renderer’s session and keeps native handling. Late messages and late surfaces must not regain focus. A cancellation message between two clients is not sufficient arbitration.

Also define whether fallback preserves **hold-to-cycle**, or merely performs one previous-window activation. Those are different guarantees.

### 3. Snapshots: the proposed boundary is much weaker than claimed

**Same UID plus executable identity is not isolation from minds.** If agents share the desktop UID, Wayland socket and host process access, they may bypass the snapshot endpoint entirely. Executable-path checks also have lifecycle, replacement and PID-reuse problems.

Required boundary: sandbox agents; restrict compositor globals and IPC; deny host screencopy, unrestricted foreign-toplevel activation, process-memory access and host display-socket access. Authenticate trusted components through supervised launch/capability registration. **`app_id` is client-asserted, not authentication; foreign-toplevel-management does not give you a trustworthy PID.**

“Memory only” still permits swap, core dumps, debugging and accidental buffer reuse. Apply retention limits, clear on lock, exclude sensitive windows, and prevent thumbnails from appearing in Mind View’s captured output.

**The rectangle capture is especially shaky:**

- Those foreign-toplevel protocols do not supply authoritative window rectangles or visibility.
- Focus-out is not an atomic capture point. Screencopy may capture the newly raised window, notifications, an approval dialog, or an overlapping password prompt.
- Scaling, decorations, multiple outputs and occlusion complicate attribution.

**On this stack: use cooperative app previews or tiles.** For third-party thumbnails, add a privileged compositor-side per-view capture service with explicit policy. Do not call an output crop a window snapshot.

Upgrading wlroots alone does **not** guarantee per-window capture: labwc must implement and advertise the required capture/source protocols and permission policy.

There is also a lifecycle contradiction: an on-demand process cannot collect historical focus-out images while absent, and dropping its cache on close defeats history. Keeping history requires a persistent privileged service.

### 4. Prediction: preserve the invariant, not just the evidence

A verified event does not verify intent. Copying from a terminal does not imply wanting it next; a mind can generate “waiting” events to keep itself first.

**First Alt+Tab should always select the previous window. Quick tap and hold should start from the same selection.** Otherwise scheduler latency and a 150 ms boundary change the destination of the same gesture.

Put suggestions in a labelled secondary lane or explicit shortcut. Freeze ordering during a session. Never insert an approval ahead of the current selection.

“First card taken” is a misleading metric when release automatically takes the first card. Measure immediate switch-backs, cancellations and extra navigation. Reason lines can themselves leak clipboard provenance and private activity.

### 5. Approval cards: navigation can still become consent

“No key path in the switcher calls approve” is necessary, **not sufficient**.

The destination may receive a queued Enter, autorepeat, pointer release or the next habitual keystroke immediately after focus changes. An agent can also manipulate urgency or imitate trusted chrome.

Selecting an approval must open a **non-actionable details state**, not focus an enabled default approval button. Require a fresh, deliberate confirmation after activation and relevant input releases. Bind consent to immutable request contents, requester, scope and expiry; reject changed or replayed requests.

Do not globally disable Alt+Tab merely because an approval is modal. That creates a focus trap—and potentially an agent-triggered denial of navigation. Scope modality and retain a trusted escape.

### 6. Missing, cuts, biggest risk

Missing essentials:

- Authoritative window identity across close/reopen, reconnect and PID reuse.
- Stable MRU semantics across workspaces, minimized windows, transients and nested Mind View.
- Lock cancellation **inside the compositor**, including pending activation requests.
- Crash recovery that removes exclusive keyboard focus.
- Authorization for `yos act shell`; an agent-accessible control path must not impersonate human navigation.
- Keyboard-layout, IME and shortcut-routing tests for **typing while Alt is held**.

**Cut initially:** prediction-first ordering, approval-first ordering, output-crop thumbnails, timeline and quick-tap pill. Ship deterministic MRU, tiles, compositor-owned release and compositor-native fallback.

**Single biggest risk:** treating a shared desktop user/session as a security boundary between humans and agents. Without real isolation, “never to a mind”, verified snapshots and trusted approval cards are UI promises—not enforceable security properties.
