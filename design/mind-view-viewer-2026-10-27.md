# A Mind's window is a viewer, not a window of the person's process

27 Oct 2026. This replaces #963 option B (PR #972). Y wrote it after the Opus security review of #972. GPT-6 Astra then red-teamed it twice: first the review, then a first draft of this document. Astra's verdict on the draft was to approve the direction and a constrained prototype, but not the specification. This revision answers each of its points; they are marked **[A2.n]**.

## Why option B is retired

#963 had to reconcile two constraints:
- the one-writer rule (#911): one process per app and user, because two copies of Notes on one vault corrupt its journal;
- winit, which gives one process one Wayland display.

Option B kept one process. It drew a Mind's window inside that process with Slint's software renderer, streamed the pixels to Mind View, and fed the Mind's pointer and keys back into the person's process.

Identity held up, but a Mind's window inherited whatever the person's process can do:
- **H1:** the person's clipboard and PRIMARY, through Slint's own TextInput;
- **H2:** the person's window actions; a Mind quit Notes and discarded unsaved text;
- **H3:** unmasked pixel copies of eight apps;
- **M1:** launches applied to the person's model;
- **M2:** hidden files' names;
- **M3:** the person's UI thread spent on the Mind's frames.

Astra listed further open channels: drag-and-drop, file dialogs, IME, accessibility, popups, notifications, open-URL, shared undo/selection/find state, timers, settings writes, logs and prompt laundering. **The boundary was in the wrong place.** The question is not which display a window is on, but which data and which authority reach the Mind's audience.

## Threat model

**In scope:** a Mind acting through Yantrik's agent tools. That means intents, `describe`, `open_app`, `read_mind_view`, its Mind View windows, and `agent_run` commands, which are themselves graded and asked.

**Out of scope:** an arbitrary process of the person's uid running unrestricted. It can already read the person's files, and no in-session design stops it. SO_PEERCRED identifies a process, not a Mind. Identity therefore comes from sessions the shell creates, never from a caller's environment or claims **[A2.1, A2.2]**.

## The model

### 1. One writer: a service start and a person's start
- **The instance.** It is the only process that holds the app's store, lock and model. It always runs on the person's display (A). Only the shell starts it, and always from the person's session environment.
- **Service start.** When the instance is started for a Mind's intent while the person doesn't have the app open, the shell starts it windowless. That start runs the framework's `start_service` path:
  - it loads the store and serves intents and `describe`;
  - it does nothing an app's person start does: no dialogs, no notifications, no account connection, no migration that asks, no recovery prompt and no indexing beyond what an intent needs;
  - each app declares which start effects are service-safe;
  - recovery and migration wait for the person's first window **[A2.5]**.
- **Person's launch.** The person's launch then presents a native window as today.
- **No smuggling.** A Mind's launch never carries files, URLs or startup actions into the person's model (review M1).
- **Concurrent starts** are arbitrated by the existing instance lock. A test drives simultaneous launches, a Mind-only start followed by the person's, closing the last window, and recovery after a crash.
- **Start effects an agent's call still needs** (recorded after #980's review):
  - Email connects the account, because its intents need it. The first look only lists folders and rows: it marks nothing read, sends nothing, and sends no receipts.
  - Weather fetches, and keeps refetching while an agent keeps the instance alive, for up to 5 minutes after the agent's last call. That sends the person's location to the provider, as any weather lookup does.
  - Download Manager notifies about an agent's own downloads.
  - Music's rescan, Network's refresh, System Monitor's polling and Calendar's timers are local and not visible to the person.
- **What step 1 does not close.** Step 1 tells the person and a Mind apart by the caller's environment and display. A Mind's `agent_run`, which runs as the same uid, can rewrite both. Step 2's shell-created sessions close that (#987).

### 2. A Mind's window is a generic viewer, per Mind and app
A Mind's `open_app` for a viewable app starts **one generic viewer process** for that Mind and that app on Mind View (B). It is one small binary that renders a few framework view kinds: a list, a document and a detail page. It has its own minimal startup: no app hooks and no platform integrations **[A2.8]**.

**Session setup** (no ticket) **[A2.2]:**
- The shell creates a connected socket pair and passes one end to the viewer as an inherited descriptor, never through argv or environment.
- It hands the other end to the instance over the instance's control socket (shell-only, 0700), together with the session's identity: Mind, app, session id and instance generation.
- The instance binds that identity to the session. Requests never carry identity, and the viewer holds no agent token.
- When the instance restarts, every session ends and the viewer blanks. A reconnect is a new session from the shell.

**Sandbox** (bwrap, launched from a dedicated spawner thread so that `--die-with-parent` follows the right thread, per the #962 lesson) **[A2.1]:**
- **Namespaces:** unshare user, pid, ipc, net, uts and cgroup.
- **Process:** `--new-session`, `--die-with-parent`, no_new_privs, a seccomp filter that denies ptrace, process_vm_* and keyctl, and rlimits on memory, file descriptors, processes and CPU.
- **Filesystem:**
  - read-only: /usr, fonts, theme assets and the viewer binary;
  - tmpfs: /tmp and $HOME;
  - not mounted: the person's home, /run/user (except B's Wayland socket, bound alone), the session bus and portals.
- **Descriptors:** everything is closed except the session socket and B's Wayland connection.
- **Environment:** cleared, then set to the Wayland display, XDG_RUNTIME_DIR (a private tmpfs) and the locale.

**On display B** **[A2.1]:**
- The viewer's text widgets have clipboard, PRIMARY and drag-and-drop disabled in the viewer itself. There is no copy, no paste and no selection export.
- Mind View's compositor gives viewers no privileged protocols: no screencopy, no data-control, no virtual input.
- One open point: whether two Minds share B. If they do, each viewer still holds only its own session's projection, and B's clipboard is unused by viewers. See the open questions.

### 3. The projection is the reviewed boundary
**[A2.3]** The claim is not "masked by construction". It is: **all application data crosses one reviewed boundary, the app's projection code, and the viewer has no other source.**

```rust
pub trait Viewable: App {
    /// Security-critical and reviewed as such: everything a viewer session may hold.
    fn project(model: &Self::Model, audience: &Audience, view: &ViewState) -> Result<Projection, Refusal>;
    /// A read request, authorised against the model at execution, never against the request.
    fn read(model: &Self::Model, audience: &Audience, request: &ReadRequest) -> Result<ReadResult, Refusal>;
}
```

- **`Projection` is a framework type, not the app's model:** list rows, document blocks and fields, each a string or a fixed enum. An app cannot serialise its whole model through it.
- **`Audience`** carries the Mind, Private mode and the agent rules for hidden files.
- **Policy helpers** in the framework decide which resources the audience may see. Search, ordering, counts, errors and pagination all go through them, so a hidden note is absent from results and counts and is never listed as "(hidden)".
- **`describe` uses the same helpers.** That reduces disagreement but does not prove agreement, so a shared test compares the two over seeded sentinels.
- **Drafts:** a projection is built from saved state only. The person's unsaved edits are never eligible.
- **Revocation:** when Private mode or a permission changes, every session is invalidated. Queued snapshots are dropped, the viewer is told to blank, and sessions resume only under the new audience. What was shown before cannot be recalled, and the doc says so.

### 4. Requests and authority
**[A2.4]**
- The viewer sends typed `ReadRequest`s: show item X, a page of the list, search titles for Y.
- The instance runs each one under the session's identity and authorises the target when it executes.
- **Local only:** selection, scroll and expanding a row stay in the viewer.
- **Never through a request:** "Show X" never selects X in the person's UI, never marks mail read and never acknowledges anything. A request never loads a path that is not in the projection.
- **Writes** are not routed through viewers at all until a later design. A Mind changes data through intents, as now, under the graded and consent machinery.

### 5. Deny by default, whatever the order
**[A2.7]**
- **Which apps:** a framework app gives a Mind a window only if it implements `Viewable`. That holds whether or not the person has the app open, and whatever order the launches came in.
- **The refusal:** every other framework app refuses a Mind's `open_app` with the next step (mind-3e's wording): "Calendar cannot open in your view; use its actions and os_describe calendar instead."
- **Registry:** one app registry in shell-core holds the declared properties. `YANTRIK_APPS`, the Mind-window rule and `mind_view.apps_the_person_has_open` all derive from it (S2).
- **Who has what open:** this comes from framework-maintained sessions and native windows, never from app assertions or launch pids (S1). It is tested with mind-3e's three-order, two-close matrix.

### 6. Cost and bounds
**[A2.5, A2.6]**
- **Rendering** happens in the viewer process, so there are no frames over the socket and nothing on the person's UI thread.
- **Projection building, serialisation and search** do run in the instance, on its job pool, with limits on size, time and pagination. There is one projection in flight per session.
- **A stalled viewer** loses its session after a bound and is never queued without limit.
- **The tests** assert bounded latency and memory under floods and stalled clients.

## Shipping order
**Step 1: containment.** It is useful on its own, and it is a behaviour change for Minds.
- **Mind windows:** no framework app gives a Mind a window, whatever the launch order, because none implements `Viewable` yet. Each answers with the refusal sentence. Intents and `describe` keep working.
- **Service start:** the shell starts instances on A, with the audited service-start path for Mind-triggered starts.
- **Shell:** owners come from the instance.
- **What it fixes:** #963's original bug, where the person's launch was swallowed by a Mind's instance on B, and the S1 list.
- **What it costs:** Minds lose their Mind View windows of framework apps until step 3. Old-style Mind View instances on B are gone.
- **Tests:** simultaneous launches, a Mind-only start then the person's start, closing the last window, crash recovery, the S1 matrix and the refusal wording.

**Step 2: the generic viewer** and one read-only test app.
- **Transport:** bounded, versioned **snapshots** with pagination and explicit invalidation; diffs come later.
- **Startup:** the socket-pair session start and the sandbox.
- **Negative tests:** malformed and oversized requests, forged ids, replay after restart, a permission change during a transfer, reconnects and resource exhaustion.
- **Real-display CI:** two headless wlroots compositors with seeded sentinels: the clipboard, PRIMARY, a hidden file, Private mode and an unsaved draft. This is evidence, not proof.

**Step 3: Notes** as the first viewable app, read-only: the library, titles, saved bodies under the audience policy, show-note and search-titles requests. Then the 520 check E.MASK1, deferred since #959.

**Step 4: Text Editor and Snippets.** Email gets its own review first, because of HTML, attachments, remote images and read receipts.

**Later:** writes routed through viewers, behind their own design.

**What happens to #972:**
- It is closed unmerged, with a pointer here. Its WIP stays on `arch/per-window-display-wip`.
- Pieces to reuse: the owner bit for window ids, per-direction message limits, link shutdown, the H2 allowlist (as test cases), the registry, and the S1 owner report.
- The asker's environment check is replaced by shell-created sessions.

## Open questions for review
1. Do two Minds share display B, or does each Mind get its own? This decides whether B's own clipboard and input can ever cross between Minds.
2. Step 1 removes Minds' Mind View windows of framework apps until step 3. Is that acceptable, or should step 3 (Notes) land before step 1 ships to users?
3. Should a viewer withhold the item the person is looking at right now?
