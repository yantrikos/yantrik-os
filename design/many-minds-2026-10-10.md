# Many minds, one witness: the Lens when minds start anywhere (2026-10-10)

People start minds everywhere. Pranab types `hermes chat` in a terminal, launches an agent from the Agents screen, a timer starts a nightly job, someone ssh-es in and runs Claude Code. Each runs independently. Today the Lens (the chat bar) sees only the active mind's `main` conversation. This doc says what the Lens shows and what the OS builds underneath for that to be true.

**History.**
- The first concept was "the chat bar as a switchboard", with @-names, a presence rail and leases. DeepSeek V4 Pro, GLM 5.3 and Kimi K3 critiqued it blind (`F:/yantrik/qwen/lead-work/switchboard-critiques.json`) and agreed: *the chat bar is a view, not the architecture.*
- A second draft built the ledger first. Fable then reviewed it against the code (`fable-design-review.md`) and corrected the ledger seam, the trust vocabulary and the row source. It also designed terminal-connect, which this version adopts.
- yantrik-mind-72 set one hard constraint (the `mind` wall, below).

The mockups are in the "Many Minds" artifact.

## The idea

**The OS is the witness.** Every action any mind takes on the desktop goes through `app.act`, and the kernel says which process made the call. A chat product knows only what its own agent says it did. So the Lens shows two kinds of statements and never mixes them:

- **Seen**: the OS recorded it ("10:42 · moved 3 files in Downloads"). The act happened; that part is certain.
- **Says**: a mind, or our bridge running inside a mind's process tree, reported it ("Drafting the reply"). It may be wrong.

The agents store already has this distinction (`Provenance Reported|Verified`, `crates/yantrik-ui/src/agents/model.rs:164`). This design makes it the Lens's visual grammar. The paper's 30-plant pilot is a reason to take it seriously: a provenance label a model reads did not lower how often it adopted a false memory (36% hidden, 40% shown, across 7 families). The person needs the distinction drawn for them, by the OS.

## What the OS can and cannot know

The OS sees acts, not sessions.

| | What the OS knows | How sure |
|---|---|---|
| An act happened (app, action, outcome) | Always: the app ran or refused it | Certain |
| Which process tree called | `SO_PEERCRED` + the `/proc` walk (`peer_identity.rs`) | Certain at call time |
| That a later call comes from the same session | Attach: every later call from the attach pid or a descendant, with an OS-minted token on every act (`host.rs:400-416`, `1138-1153`) | **Continuity**, not identity |
| Which mind it is | Only the `yantrik-mind` account is *verified* (`wire/harness.rs:740-758`). Any other `id` and `name` is self-declared: a process running as the person can be anything (`approvals-2026-09-21.md:704-712`) | Says |
| Where it was started | The chain, tty, cgroup and `sshd` (#718) | Good evidence, not proof |
| What it is doing | Only what an attached harness reports | Says |

**One vocabulary, used everywhere** (strip, card, ledger), taken from the approval card's own words (`caller_identity.rs:84-91`):

| Tier | Established by | Shown as |
|---|---|---|
| **Your Mind** | the `yantrik-mind` uid at attach | "Yantrik Mind" |
| **Attached** | pid continuity + token | "Hermes · says it is Hermes · python -m hermes_cli chat · terminal · attached 10:58" |
| **Not attached** | a call with no token | "a program · python3 nightly.py · ssh from 192.168.4.12 · not attached" |

"Verified" is reserved for your Mind. "Recognised" is dropped: it would have been a third vocabulary.

## Terminal-connect: how a mind started anywhere reaches the Lens

Today `hermes chat` in a terminal never reaches the harness socket. The Hermes adapter is a gateway platform plugin, and the protocol has no way for a harness to start a turn itself (`adapter.py:416-420`). So every act from that Hermes is `Caller::NoAgent`, the person (`control_agents.rs:94-108`). That's the gap.

**Separate presence from driving.** Today attaching means both "I exist" and "the desktop gives me turns". Add a presence-only mode, and attach where the OS already is inside every third-party harness: **`yos-mcp`**. Every MCP harness with the desktop's tools (Hermes CLI, Claude Code, pi, openclaw, anything over ssh) runs our bridge as a child for the life of its session. When the bridge starts with no `YANTRIK_AGENT_TOKEN` in its environment, it attaches presence-only and puts the minted token on every `yos act`. Every such session becomes visible, addressable for approvals, and holdable, with **no change to third-party code**. Where a real adapter exists (the Hermes gateway), the adapter attaches as today and hands the bridge its token; the bridge then does nothing new.

### Protocol additions (harness → OS, poll-based, backwards compatible)

- **`harness.attach` gains optional fields:**
  - `shared: bool` (default false: replace, as today). A shared attach never replaces another; the host mints its key (below) and answers `{session, as: "hermes.1"}`. Capped at `MAX_SHARED_PER_ID = 4`.
  - `takes_turns: bool` (default true). With `false` (presence-only), the host never queues an assignment for it, and `send_to` is refused: "`hermes.2` takes its turns in its terminal; leave it a note instead". It polls every 20 s, inside the 90 s presence timeout, for `cancelled`, `ended` and `answers`.
  - `scope` (phase 2): `{apps, paths, network}`. Recorded and shown; enforcement is reach (below).
- **New `harness.open {session, conversation?, text?}` → `{turn_id, conversation, agent_token}`** opens a turn the harness is answering somewhere else. Everything after it is the existing wire: `chunk`, `event`, `complete` and `fail`.
  - One turn in flight per conversation.
  - A new conversation counts toward `MAX_LIVE_AGENTS`; `main` is exempt, as today.
  - A missing `text` is drawn as "a turn in its terminal", never as a person's bubble.
  - The run store records `origin: self`.
- **Origin and program are the OS's, never the harness's.** At attach, the shell walks the attach pid (`peer_identity::walk`, then `origin_of` from #718) and annotates the entry (`Host::annotate`). `Entry` and `AgentEntry` gain `program`, `origin`, `shared` and `takes_turns`.

### The `mind` wall (hard constraint, yantrik-mind-72)

Today the OS trusts the id `mind` only when the kernel says the caller is the yantrik-mind uid. Grant requests, search-grant cards, first-party memory defaults and egress lean on that (`wire/harness.rs:740-758`, `memory_grants.rs:130, 318-319`, `control_approvals.rs:510, 531, 1293`). A self-attached mind must never stand under `mind`, or under any id a grant, card or search grant is bound to. This is enforced in **one function**, `Host::allocate_key(announced, shared, uid, is_mind)` in `host.rs`, called before anything is inserted:

1. An `id` in `RESERVED` (`mind`, `companion`, every built-in) from a uid other than the mind account is refused. The shell's `first_party_claim_refused` stays as the outer wall; the host repeats the check, so a test proves it without the shell.
2. A shared attach never takes a bare id. It always gets `<id>.<n>`, even when `<id>` is free. A dotted `id` is refused on the wire, so nobody can *ask* for `hermes.2`.
3. `<n>` is the smallest integer unused for this run of the host (`State.issued_keys`); it restarts only with the shell. Nothing is ever granted to a dotted key by default. A grant made in Settings names the key *and* the program line, so tomorrow's `hermes.2`, running a different program, inherits nothing.
4. Approvals, `Verified.agent` and `grant_belongs` already bind to `<key>:<conversation>` and the token, never to a display name.

The test is `a_shared_or_terminal_attach_never_takes_a_grant_bearing_id`, beside the shell's `only_the_mind_account_attaches_as_the_first_party_mind…`. It covers:
- `mind` refused from uid 1000;
- a shared `hermes` keyed `hermes.1` even when `hermes` is free;
- an exclusive `hermes` alongside it;
- no memory credential for `hermes.1`;
- `hermes.2` refused on the wire;
- the counter restarting with a new `Host`, and an old token refused on `resume`.

**When the bridge does not auto-attach.** Each of these alone is enough:
- It runs under the mind account. Its acts already need a live agent token (`call.rs:141-147`).
- `YANTRIK_AGENT_TOKEN` is in its environment.
- `YANTRIK_YOS_MCP_NO_ATTACH=1` is in its environment. The Mind sets this on its own `yos-mcp` launch from boot, through `set_server_env`. It covers machines where the Mind still runs as the person's account because it was never migrated with `migrate-minds`; there, the Mind starts `yos-mcp` without a token and sets one only on its first turn (yantrik-mind-72, 10 Oct).
- **The host refuses it** when the attaching pid descends from an already-attached harness's pid. The answer is "already covered by `<key>`", and the bridge then carries on without attaching. This is the general rule; the environment variable is the explicit one.

**Replace semantics stay.** A non-shared attach still replaces the previous session under its id. The Mind depends on this for `mind`: it re-attaches plainly on start and after "lost the desktop", and reads only `reply["session"]` (mind-core `harness.rs` ~428-460). A test pins it: a second plain `mind` attach from the mind uid replaces the first.

**Conditions on `harness.open`** (yantrik-mind-72), because it creates turns the person did not type:
1. A self-opened turn never carries `turn["run"]`. Run grants stay person-stamped only.
2. Any card raised inside a self-opened turn names the opening key and its program line, so the person never takes it for a desktop request of their own.
3. Its `agent_token` is scoped to that one turn.
4. A grant given to one key (a session search grant to `mind`, say) never applies to a turn opened under another key.

Each condition gets a test in task 7.

### Trust and consent

Attaching grants nothing: no memory, standard reach, every act still graded. So there is **no card at attach**, because it would be approving a name nobody can check. The person confirms where they confirm today: at the first graded act, on a card that now carries the instance, the program line and the origin. Granting an instance memory or wider reach goes through the existing grants UI, per key. Bridge-reported events are Says (a hostile harness can run a fake bridge); the ledger entry for the same act is Seen.

### A Lens turn into a terminal-driven session

Only harnesses with a real adapter take turns from the Lens. The host already serialises turns per conversation (`host.rs:1686-1697`): a Lens turn waits while a self-opened turn is in flight. It carries `origin.channel = "lens"`, so the terminal prints `▸ from the desktop: …` and answers in its own loop. The terminal stays primary: a self-opened turn never waits for the Lens.

For presence-only sessions, **Talk to it becomes Leave a note**. `Host::note_for` (`host.rs:1316`) queues it, and the bridge delivers it in the reply of the session's next `yos act` as `notes`, printed "Desktop note (from you, 10:59): …". Only the person may leave notes (`Caller::NoAgent`), never another agent.

### Session state

- **Presence-only:** the bridge opens a turn on the first act after idle, sends `tool_start` and `tool_end` per act and `status` while it waits on a card, and closes the turn after 30 s idle. The state is Working while a turn is open, Needs you when the shell holds a card for its token (an OS fact), and otherwise "Last update 10:42". Never Finished, because the bridge can't know.
- **Real adapters** also mirror the harness's own approvals as `Request` events (`event.rs:125-133`). Hermes's `/approve` becomes a card in the one inbox, and the answer rides back on `poll.answers`. Today those approvals are invisible to the OS; this is the biggest usability win in the design.

### Failure modes

- **The terminal closes or the harness crashes:** the pid dies and the entry is reaped (`host.rs:426-429`). Open self-turns end orphaned. The row reads "Connection lost", then stays ledger-only for 15 min.
- **Two terminals:** `hermes.1` and `hermes.2`, each with its own program line and tty. The gateway stays exclusive on `hermes`. An old adapter without `shared` keeps replace semantics (`host.rs:1506-1526`).
- **A spoofed name:** "Hermes · says it is Hermes · python3 evil.py · terminal". `caller_identity::mismatch` must compare against the instance the token resolves to, not "any Hermes", or a genuine second instance would fire the warning.
- **Private mode:** attach and poll are refused (`wire/harness.rs:708-718`). The bridge prints one line and re-attaches when the mode ends. The strip shows "Private · 4 attempts refused".
- **A shell restart:** resume works as today.

## The Lens

### 1. At the machine

Under the header, one collapsed line, "3 minds at the machine · 1 needs you", expands to one 44px row per mind that is attached or acted in the last 15 minutes. The rows are a read model that merges three sources:
- `Host::list()` (an idle attached Hermes has no agent row, `host.rs:1060-1068`);
- `Host::agents()`;
- the ledger's actors, keyed by token when attached, else by `program + chain_root_pid + started`.

Each row shows the name and its tier line, with origin as plain text (never an icon), and a state from the shared vocabulary only when it is reported or is an OS fact. Otherwise it shows "Last update 10:42".

### 2. One approvals inbox

All pending approvals sit at the top whenever any exist, oldest first. Each card carries the program line (the card already prints it; keep that rather than the self-declared name), the instance and the origin. Mirrored harness `Request`s join them. The existing rules stand: Approve once / Decline, no approve-all, and the approval binds to the action as displayed.

### 3. Ask the machine, instead of @all

"Who's touching my Documents?" The home mind answers from the ledger with a read-only tool graded `safe` and filtered by reach: the person sees everything, an agent only its own token's rows. Nothing is broadcast to other minds, so there is no new path for an injected prompt. When the Lens opens after idle, it suggests "Since you left".

### 4. Address by what it is doing

There are no @-handles. "Tell the one editing index.html to stop" resolves by token through the ledger to one instance, and becomes a chip ("To Claude Code ×") that the person can see and remove. Turn-takers get **Talk to it**; presence-only minds get **Leave a note**. Callers that aren't attached get no chip, only "not attached".

### 5. Look in

One rule: show what the OS knows, and nothing it doesn't.
- **Attached:** its self-turns with tool cards (Says), interleaved with its ledger rows (Seen).
- **Not attached:** ledger only, with the banner "Give it the desktop's tools and it appears here".

### 6. Holds (phase 2)

**A hold is reach.** `reaches::hold` already limits an agent to some surfaces and a ceiling on every door, keyed on its token (`reaches.rs:46`, `control.rs:605-612`). Phase 2 narrows it to apps and paths, driven by `Attach.scope` and an overlap forecast ("Hermes.1 and Claude Code can both write to ~/Projects/site").

Per-name holds on unattached callers are out: renaming dodges them. Instead, one switch, **"Hold everything not attached"**, refuses tokenless acts above `safe`. It can't be dodged by a name, and the switch says plainly that your own bare `yos act` is affected too.

### 7. Time travel (phase 3)

"The machine as of 10:42", and undo where an app's act has an inverse.

## One mind, many threads

**The scenario** (Pranab, 10 Oct): "I asked you to work on the OS UI, the website, and the YantrikDB part." That is one mind with three jobs, not three minds. It is also what this machine already looks like: about 25 Claude sessions, many of them threads of the same Claude (yantrik-os-22, yantrik-mind-72, yantrikdb-…). Today they show as 25 unrelated sessions.

**The principle.** The person talks to a mind, not to processes. Pranab's words: "Memories are what makes something whole, not the llm." So threads share one memory and one identity, and the Lens shows **one mind with lanes**, never three chats.

### What a thread is

A thread is a conversation, `<key>:<conversation>`, with a **title** and a **goal**. The protocol already models several conversations per harness (`Attach.conversations`, `AgentMeta {mind, parent, role}`); what's missing is a name a person reads.
- The person starts one in plain words ("work on the website"). The mind opens it with `start_agent` or `harness.open`, both gaining `title` and `goal`.
- Separate instances started separately (two terminals: `hermes.1`, `hermes.2`) are grouped under the name they say. **The grouping is display only.** Each lane keeps its own tier line, and grants, approvals and holds stay bound to the key. A process that borrows the name joins the group visually and gains nothing else; its lane's program line gives it away.

### In the Lens

1. **One row per mind.** The strip shows one row, "Claude · 3 threads · 1 needs you", which expands to lanes. Each lane has a title, a state from the one vocabulary, one Seen line and one Says line:
   - OS UI · Working · "PR #721 in review"
   - Website · Needs you · "merge PR #1?"
   - YantrikDB · Paused · "waiting for the model window"
2. **Routing you can see.** A plain message goes to the mind. It works out which thread the message is about (from the thread titles, the ledger and the message itself) and shows a chip, "→ Website", **before** sending, so the person can change it. Tapping a lane sets the chip directly. A message about two threads gets two chips.
3. **One digest, never three reports.** "Since you left" is a single message, ordered by what the person must do, not by thread:
   - what needs you;
   - what finished, with evidence (PR links, screenshots; Seen);
   - what is moving (one line per thread).

   Every line carries its thread tag.
4. **Decisions with a recommendation.** A thread's question is a `Request` with `kind: decision`, its `options`, one `recommended`, and a one-line reason. The inbox collects decisions from every thread. Each has **Go with recommendation**, and there is **Accept all recommendations** for decisions only. Approvals of actions never batch: the existing no-approve-all rule stands, and the two kinds are visibly different cards. (This is the pattern Pranab just used: "I will go with your recommendation.")
5. **Cross-thread awareness.** When one thread's work touches another's (overlapping paths in the ledger, or a dependency a thread declared), the mind says so once, where it matters: "The dock change (OS UI) makes the website's screenshots stale; I added a refresh to the Website thread."
6. **Shared resources, shown honestly.** Threads share the model window, the build box and the test VM. The mind shows the queue as it is: "YantrikDB paused until 18:00: the Ollama 5-hour window is spent." The person can reorder in words ("website first"), and the mind confirms what moved.
7. **Quiet by default.** A notification fires only when a thread is blocked on the person. Everything else waits for the digest.

### Memory across threads

One memory, many hands. Every memory a thread writes is tagged with its thread (provenance). When two threads disagree, the mind does not pick silently; the disagreement becomes a decision: "The Website thread has the brand gold as #C9A227; the UI thread uses #D4AF37. Which is right?" The paper's pilot is the reason: a label alone did not stop models adopting false memories, so the mind must reconcile, not just tag.

### What changes

Protocol (P, yantrik-mind-72 first):
- `title` and `goal` on `harness.open` and `start_agent`;
- `Request` gains `kind: decision | approval`, `options`, `recommended` and `reason`.

Shell:
- the grouped strip and lanes;
- the routing chip;
- the digest;
- the decision inbox with Accept all, for decisions only;
- cross-thread notes from ledger overlaps;
- the resource line.

These are phase-plan tasks 18–22.

## The board: sticky notes between you and your minds

Pranab, 10 Oct: "We should also add the sticky note board." The Lens is the conversation; **the board is the asynchronous space**: what you leave for your minds, what they leave for you, and what is still open between you. It gathers in one visible place several things this design already has: Leave a note, decisions, cross-thread notes and the digest.

### Where it lives

- **On the desktop**, as a board you see when you come back, with columns: **For you**, then one per mind (with that mind's thread lanes), then **Anyone**.
- **In the Lens**, as a Board tab. The digest links to it.
- **On the phone**, through the existing channels, but only a note that is blocking a thread ("quiet by default").

### A note

- **Who wrote it and who it's for.** "You", or a mind thread shown in its tier line; addressed to a mind, a thread, "anyone", or you.
- **What it is:** a note, a **decision** (with the mind's recommendation, its reason, and Go with recommendation), a **blocker** (amber: it is waiting on you), an FYI, or a reminder.
- **Links:** a PR, a file, a ledger entry, a screenshot.
- **Its state:** Open → **Delivered** → **Answered / Done** → Archived.
  - *Delivered* is Seen. The OS records it when the note rode on the recipient's next turn or act reply (`Host::note_for`, the bridge's `notes` field): "delivered 10:59 with its next action".
  - A mind's acknowledgement ("got it") is Says.
  - The board never claims a mind acted on a note; only the ledger can show that.

### How it works

- **You write a sticky and drop it on a lane.** That is Leave a note, made visible. Dropping it on a lane routes it to that thread.
- **A mind posts to For you.** It posts with a new harness event, `note {to, kind, text, links, options?, recommended?, reason?}`. A decision becomes a decision card; `Accept all` applies to decisions only, and action approvals are never on the board.
- **Within one mind**, a thread may post to its sibling threads (the same key). That is how "the dock change makes the website's screenshots stale" travels.
- **Across different minds, nothing is posted directly.** A note from Hermes to Claude Code lands in For you as "Hermes wants to tell Claude Code: … · Forward / Dismiss", and you forward it. This keeps the injection point the critiques named closed and matches `may_direct` (`control_agents.rs:1408`).
- **Content is data.** A delivered note is quoted to the recipient as the person's words, or as another mind's words forwarded by the person, never as an instruction from the OS. Text is shown plain; links are opened by the person, never followed automatically.

### Store and trust

- The board is a shell surface, like approvals, **not** notes-service. Notes-service holds the person's files and lets any mind with notes access edit them, which is the opposite of the board's posting rules.
- Its history is event-sourced on the shared journal (#717): post, deliver, acknowledge, answer, archive. Every event is also a ledger entry.
- The posting rules are enforced in one function: the person may post anywhere; a mind only to For you and to its own key's lanes. Tested.
- Archived notes follow the ledger's retention: 90 days or 256 MiB.

### Tasks

23. The board store: event-sourced on `yantrik-journal`, plus the posting-rules function and its tests. S.
24. Protocol: the `note` event from harnesses (decision, blocker, FYI) and delivery of a person's note on a turn or act reply. P.
25. The board surface: desktop columns per mind and thread, drop-to-route, the Board tab in the Lens.
26. Cross-mind Forward / Dismiss, the decision cards' Go with recommendation and Accept all (decisions only), and blocker-only phone delivery. S.

## The ledger (#148)

- **The seam.** `ActCall::log` (`call.rs:197-211`) is a tracing line that runs once, before the handler, after every refusal has already returned (`control.rs:601-633`), so it is **not** the seam. Record at dispatch entry, right after `ActCall::parse` (`control.rs:599`), and again on the `Result` the dispatch returns, including refusals (`PRIVATE`, standing, reach, grant, the handler's `Err`). The test: a `PRIVATE` refusal appears with `outcome: refused:private`.
- **The keeper.** A supervised `services/action-ledger` on `yantrik-service-sdk`, not the shell, which is the most crash-prone thing on the machine. It reuses the shared segment journal (#717).
- **An entry:**
  - `seq`, `at`, the reporting app, `action`;
  - an args summary (the card's; secrets reduced to their kind);
  - grade, `granted`, outcome, `action_id`, revision;
  - the actor `{tier, key?, says_name?, program, chain_root_pid, started, origin, uid}`.
- **Trust.** The entries are hash-chained, so an edit or deletion shows. `ledger.record` is accepted only from the desktop's own programs (`owner::desktop_programs_only`), and the reporter is recorded. It is tamper-evident and hard to forge, not proof against a determined same-user program; the `yantrik-mind` account can't touch it at all.
- **Readers.** The raw `ledger.since` serves desktop programs only. Everyone else, `yos ledger` included, reads through a graded read-only action on the shell's `app.act`, filtered by caller: the person sees everything, a mind only its own entries.
- **What it replaces.** The ledger replaces `mind-audit.jsonl` as the source for Recent actions.

## Phases

S = Claude security review. P = harness-protocol change: yantrik-mind-72 gets the spec before it reaches 520.

1. #717: the shared segment journal crate (in review, PR #722).
2. #718: the origin classifier. S.
3. `services/action-ledger`: the hash chain, `ledger.record/since/status`, retention, `desktop_programs_only`. S.
4. The runtime records every act at dispatch entry and on its `Result`, refusals included. S.
5. `yos ledger` and the mind panel's Recent actions read the ledger; the mind-audit readers move over.
6. Protocol: `Attach.shared/takes_turns`, `as`, `Host::allocate_key` (the `mind` wall and its test), the caps, the `Entry` fields, `annotate`. P, S.
7. Protocol: `harness.open`; the run-store origin; presence-only `send_to` refusal. P, S.
8. Shell: annotate attaches with program and origin; per-instance `caller_identity` and `mismatch`. S.
9. `yos-mcp` auto-attaches presence-only, with the token on every act, self-turns, and notes in replies. S, P (tell mind-72).
10. Harness lib: `shared`, `takes_turns`, `open_turn`, slow poll; pi mirrors its own approvals as `Request`s.
11. The Lens strip (the merged read model) and the one vocabulary.
12. The approvals inbox at the top, with origin and instance.
13. Ask the machine: the built-in mind's ledger tool, filtered by reach. S.
14. Talk to it, Leave a note, chips and resolve-by-ledger. S (the note channel).
15. Look in, in both shapes.
16. `Attach.scope`, the overlap forecast, holds as narrowed reach, "Hold everything not attached". P, S.
17. Phase 3: the as-of view, and undo.
18. Protocol: thread `title` and `goal`; decision `Request`s with `options`, `recommended` and `reason`. P.
19. The strip groups one mind's threads into lanes (display only; trust stays per key).
20. The routing chip ("→ Website"), shown before sending; a message about two threads gets two chips.
21. The digest ("Since you left"): needs you, then finished with evidence, then moving.
22. The decision inbox with Go with recommendation and Accept all (decisions only); cross-thread notes; the shared-resource line; thread-tagged memory writes that surface disagreements as decisions. S (memory).
23–26. The board (see "The board"): the store and posting rules (S); the `note` event and delivery (P); the surface; cross-mind Forward and decisions (S).

## Decided (Pranab, 10 Oct: "I will go with your recommendation")

1. **Retention:** 90 days or 256 MiB, whichever comes first. Record your own `yos` use too: a chain with holes is where a hostile act hides.
2. **Private mode:** record nothing but two boundary entries ("private on 10:42", "private off 11:03 · Hermes refused 4"), so the gap reads as deliberate.
3. **Erase:** include the ledger, and leave one final entry, "erased 1,204 entries on 2026-10-10 by you".
4. **Unverified minds:** no per-name holds. Offer the single "Hold everything not attached" switch instead.

## Left out, on purpose

`@all`; @-handles; a live hybrid tap-in; origin glyphs as identity; blocking leases; inferred states; "verified" for anything but your Mind; a card at attach; the Lens taking over a mind started elsewhere.
