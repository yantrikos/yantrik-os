# Mind View — round 3

This spec supersedes earlier rounds. `mindview.html` contains clean frames A–E followed by five annotated copies and external legends. `v3-1.png` through `v3-5.png` are clean; `v3-annotated-1.png` through `v3-annotated-5.png` have review pins. Scenes, narration, calls and model metadata are synthetic review fixtures. All HTML actions are local demonstrations; no harness, message, export or OS permission is changed.

## Five moments

A / Working retains desk tabs, quoted Reported narration, live app and memory. The running tool is in a dedicated chrome strip beneath narration, with provenance, elapsed snapshot and outcome counts. Only the Live/Replay chip and reticle overlay the app.

B / Needs you shows pending call 17, files.write, at 06:12. The shell card remains outside the room and above it. A subtle connector routes around the memory heading to the fully visible now cell. This is an OS-mediated write request; its later completion at 06:18 belongs to D's history, not the pending frame. Existing Deny, Allow for session and Approve once retain their backend meanings. Permission granted does not mean write succeeded.

C / Idle retains static artwork and invitation. All three suggestions open an editable, unsent draft in the existing Lens. The footer describes the built-in Mind's actual execution identity.

D / Replay shows call 14 of 17 at 05:12. The factual chrome strip contains index, tool name, call time, outcome, provenance and capture time. There is no OS-written headline or scene caption over the capture. Mind narration remains separately quoted and Reported. The capture uses the same composition as live; production displays the retained bitmap, not reconstructed geometry. Read-only is explicit.

E / You're driving is NEW. Hermes is paused, the narration is “Waiting while you drive”, ownership says Input: You, and Hand back is explicit. Violet #B5A0F5 outlines the room and accents narration and the button. Violet distinguishes person ownership from teal Mind activity, amber decisions, red failure and green completion; text repeats the meaning. The last tool call is Interrupted, not Running.

## Per-Mind truth

Hermes, Pi, OpenClaw and DeepSeek run AS THE PERSON today. Calls through the OS control surface receive applicable approvals, but a harness can also run a shell command as the person. The nested compositor is a workspace, not a security boundary.

Hermes footer: **Runs as you · OS actions ask first · Not isolated (planned)**. OS actions ask first describes only the OS-mediated policy path, not interception of every direct shell/filesystem operation. B explicitly labels its OS-mediated write. The blanket Exports: ask first claim is removed.

Built-in Yantrik Mind footer: **Own account · Network checked · OS actions ask first**. Only the built-in Mind currently uses its own account and mediated network access. Selection must use the selected Mind's execution facts. Unknown facts render unavailable, not optimistic defaults.

**NEW: third-party harness isolation — issue #411.** Requires a real boundary around harness processes and descendants, filesystem/network enforcement and queried effective execution identity. Until implemented and verified, retain Not isolated (planned). Input takeover does not provide isolation. NEW labels belong in annotations/spec, never clean product pixels.

## Exact idle actions: existing Lens only

| Button | Existing action | Exact editable starter prompt |
|---|---|---|
| Build a small world ↗ | Open Lens addressed to Yantrik Mind; focus composer | Build a small world with three connected islands in Blender. |
| Explore an idea ↗ | Open Lens addressed to Yantrik Mind; focus composer | Help me explore an idea: [describe your idea]. |
| Organize a project ↗ | Open Lens addressed to Yantrik Mind; focus composer | Help me organize a project: [describe your project]. |

Clicking a suggestion never creates a run, opens Blender, executes a tool or submits a message. The person may edit/cancel; only the existing Send action submits. Preserve existing Lens drafts; use supported separate-draft behavior or ask before replacing one. These are bindings to existing actions, not a new starting mechanism. The HTML demonstrates an editable Lens dialog; Send records a local demonstration only.

## Layout and compositor

At 1280×800, room (48,62), 1184×656: header 55px, narration 74px, tool/replay chrome 40px, nested surface 302px, memory 140px, footer 43px plus border rounding. Idle retains its 556px invitation region. E has a 2px violet border. Dock (492,740), 296×43. Approval (638,245), 570px wide and approximately 275px tall, leaving its complete now cell visible.

NEW: atomic compositor grouping for docked sibling header, status, memory and footer surfaces, including geometry, focus, stacking and minimize. Preserve nested force-maximised apps, Alt+Tab and Alt+F4. Only the Live/Replay chip and attributed reticle may overlay nested desktop pixels. Tool and replay metadata are chrome. Desktop-level shell approval is the explicit exception; review pins do not ship.

At smaller sizes retain provenance, Stop and handoff controls before model details or expanded tabs. Support wrapping/scrolling at large text scales and adequate hit targets. This review artifact is fixed-size, not a responsive implementation. Cache wallpaper/orbits and thumbnails. Solid fills and alpha only; no blur, shaders, live screenshot polling, animation loop or fabricated ticking metrics.

## Memory and evidence — NEW capture journal

Seven selected moments span the empty plane through islands, including terminal captures. The full ordered call index exposes all retained calls; selecting a hidden call reveals its cell. The visible subset is labelled Selected moments, not presented as consecutive calls. Preserve stable call IDs when new events append.

Call 8, 02:16: Verified · Failed, red mark and MeshError: missing face. Call 11, 03:51: Verified · Interrupted, neutral muted mark and SIGINT · exit 130. Terminal steps look like terminals. These strings are synthetic safe fixtures, not authority to place arbitrary raw output in chrome.

Outcome (Running, Ok, Failed, Interrupted, Untold) and evidence (Verified, Reported, unavailable) are independent. Pending is an approval-request state. An OS screenshot does not upgrade a harness report to Verified; Verified is not a quality guarantee. Store turn/desk/Mind/call IDs, ordinal, tool name, start/end offsets, outcome, evidence, capture ID/time/dimensions, classified narration/time and optional last-input coordinates/time.

Use existing safe-grade screenshot_mind_view no more than once every two seconds per source surface. Capture eligible call boundaries; retain metadata for every call. Running calls may have a start-boundary capture. Rate-limited, excluded, failed, disconnected and evicted captures show explicit placeholders, never recycled images posing as new captures. Capture time is independent of call time. Production terminal captures retain their actual aspect ratio; HTML uses simplified fixtures.

Start with an in-memory ring: 48 captures per desk, four desks, 1280×720 maximum aspect-preserving images, 96 MiB compressed budget, 24 MiB decoded cache, 200 metadata records per turn. Evict oldest images at either limit and show Earlier frames expired. Decode selected/adjacent and visible thumbnails only. No persistence by default; clear on request or teardown.

Explicit capture eligibility must exclude sensitive/protected windows and shell approval overlays. Readback masking alone does not establish retention safety. If exclusion is impossible, show Capture unavailable. Selecting a frame never shares it. Any later export needs review/redaction and explicit export.

Replay is read-only; no click reaches the actual app. New calls must not steal selection. Preserve aspect ratio with letterboxing. Provide keyboard traversal and accessible call names. Return to Live reveals the real current surface without changing ownership or resuming execution.

## Take over / Hand back — NEW backend need

E is proposed acknowledged behavior, not a pause capability already guaranteed for Hermes. Suspend requests need desk ID, Mind ID and run generation. Quiesce in-flight input, gate queued/new UI and relevant tool operations, and obtain a harness pause acknowledgement before claiming paused or granting person input. If a harness can continue uncontrolled commands, disable takeover until integration can suspend it. Expose non-interruptible work rather than claiming it paused.

During acknowledgement show Taking over… and keep person input disabled. Failure retains the prior owner and explains the failure. Success shows E, removes the Mind reticle, freezes metrics and retains interrupted call evidence. To label Waiting while you drive as Reported, that text must come from the harness status channel. Otherwise show neutral unquoted OS state Mind paused.

Hand back explicitly resumes the same generation. Keep person ownership until acknowledgement; show Handing back… while pending. Invalidate stale responses on Stop/disconnect. Never resume on blur, tab switch, timeout, window close or Return to Live. Handoff grants no pending approvals and changes no account, network or filesystem policy. The HTML simulates transitions instantly; production must not.

## Remaining contracts

NEW: stable per-Mind compositor attribution, capture routing and normalized input reticle coordinates/time. Unknown/shared windows need explicit rules. Until attribution exists, show one shared Mind View and selected Mind activity separately; do not imply app ownership.

Narration is classified display-safe Mind text, quoted and Reported with event time. Missing text uses a neutral unquoted state/tool label. Do not leak raw prompts, tokens, private paths, query strings or arguments into chrome, hover or accessibility labels. The model deepseek-v4.1-flash via ollama-cloud is a harness-reported fixture, not the model used to create this design. Unknown provider/model renders unavailable.

Approvals use immutable request ID/version and existing session scope. NEW: explicit request-to-call association for the connector; do not match by tool name. Disable buttons on submit; expired requests show No longer pending; retry only still-valid requests. Deny is not Stop. Stop retains confirmation and only claims stopped after backend acknowledgement. Existing shell stacking stays outside the nested surface.

WaitingForYou uses amber; Failed red; Done may use green. HarnessGone shows Connection lost and disables backend controls. Quiet duration alone is not failure. Replay state, execution state, input ownership and approval state are independent.

## Validation and delivery

Build: node build-v3.cjs. Render: node render-v3.cjs using installed Node Playwright/Chromium headless shell. Ten 1280×800 screenshots are supplied. Every clean and annotated render is visually inspected. verify-v3.cjs checks frame count, removed claims, chrome placement, now-cell geometry, failed/interrupted evidence, Lens drafts, replay metadata and local handoff. Rendering does not validate real compositor integration, harness suspension, isolation or approval enforcement.
