# Yantrik, live: a public 24/7 instance anyone can watch

29 September 2026. Status: **design, agreed in outline with Pranab.** Nothing is built.

## Why

A demo says "this is an agentic OS". A live instance shows one: a machine that has been running
for 43 days, whose memory has grown the whole time, doing real work nobody staged. Continuity is
the whole thesis behind YantrikDB and the Mind, and a 30-minute video cannot show it.

The page, `yantrikos.com/live`:

```
YANTRIK · Autonomous OS Instance #001                ● LIVE · 43d 17h 21m
┌──────────────────────────────────────┬──────────────────────────────────┐
│                                      │  WHAT YANTRIK IS DOING           │
│         the live desktop             │  Goal      research a topic      │
│    (calm; what the mind opens)       │  Activity  reading a paper       │
│                                      │  Memory    12 recalled · 3 new   │
│                                      │  Skill     research              │
├──────────────────────────────────────┴──────────────────────────────────┤
│ PULSE  18:03:21 recalled 12 memories · 18:03:27 updated entity: Qwen …  │
│ Memories 184,392 · Skills 247 · Events 1,823,441 · Autonomous acts 62,813│
└──────────────────────────────────────────────────────────────────────────┘
```

The right-hand side matters more than the video: it lets people watch an intelligence at work,
not a desktop. The desktop stays calm; the activity around it is what's worth watching.

## Decisions (Pranab, 29 Sep 2026)

- **Host:** a new, dedicated VM on node2, on its own subnet behind a gate VM (see The network).
  It is not VM 520, the release gate that gets reinstalled constantly, and it is not the family
  box.
- **Model:** the AIG gateway (`aig.mycluster.cyou`, the home Kubernetes cluster on node2) is
  primary: qwen3.8:27b as the reasoner, gemma4:e4b for fast dispatch. The cloud subscriptions we
  already hold are the fallback, so the stream never stalls when the gateway is down.
  - AIG silently answers with qwen3.8:27b for any model name it doesn't know, so the instance
    checks the model name echoed in each reply.
  - The Mind's first-run step can't list AIG's models (the gateway returns 404 for `/api/tags`),
    so the instance is configured with an address and model name directly.
- **v1 is view-only:** the live desktop, the Pulse, the counters and the uptime. The website has
  no interaction endpoint at all; nothing a visitor does reaches the instance (Pranab,
  29 Sep 2026). "Ask Yantrik" comes later, once we know people watch and the moderation exists.
- **It must never be boring:** something real and visible happens around the clock (see
  "Twenty-four hours of real work").

## The network: inside the house, walled off

The instance reads untrusted content all day, so what matters is what it can reach if something
inside it goes wrong. On the LAN that would be Proxmox, the Kubernetes cluster, the family box
and more. "No inbound ports" doesn't help, and neither does the egress proxy alone: it runs inside
the VM, and root in the guest could turn it off. So the wall is enforced outside the VM
(Pranab, 29 Sep: a separate subnet with one route to one AIG endpoint).

```
 internet  ◄── NAT ──┐
          ┌──────────┴──────────┐         192.168.4.0/22 (the LAN)
          │  live-gate (tiny VM)│ eth1 ── vmbr0 ── AIG, Proxmox, k8s, …
          │  10.99.0.1  │ eth0  │
          └─────────────┼───────┘
                        │ vmbr9 — new bridge, no physical port, 10.99.0.0/24
          ┌─────────────┴───────┐
          │ live instance #001  │ 10.99.0.10, gateway 10.99.0.1
          └─────────────────────┘
```

- **`vmbr9` has no uplink.** The instance's only way out is through the gate. This is enforced
  by the hypervisor, so root inside the instance can't change it.
- **The gate's nftables:** traffic from 10.99.0.0/24 is NATed to public addresses. It is dropped
  for 10/8, 172.16/12, 192.168/16, 100.64/10 and 169.254/16, except the gate's own proxy port.
- **The one endpoint is not AIG's address.** 192.168.4.203:443 is the Kubernetes ingress, and it
  serves every hostname the cluster has. The gate runs a model proxy on `10.99.0.1:8443` instead.
  It forwards only the chat calls (`/api/chat`, `/v1/chat/completions`), only with the
  instance's own key, rate-limited and size-capped, and always to the one hostname
  `aig.mycluster.cyou`.
- **DNS:** the gate answers from public resolvers only, so the LAN's names (Technitium) are never
  visible from the instance.
- **Admin:** through Proxmox (`qm guest exec` / console). There is no network path from the LAN
  into `vmbr9`.
- **Nothing shared:** no SSH keys, mounts or credentials in common with the LAN. Its memory is
  backed up and checked, so a compromise means rolling back to a snapshot, not starting over.
- **Building it on node2:** the new bridge goes in a file of its own under
  `/etc/network/interfaces.d`, and nothing about `vmbr0` changes; node2 also hosts AIG, the k8s
  workers and the releases server. The Proxmox firewall stays off: enabling it cluster-wide with
  the wrong defaults can lock out node2, and the bridge with no uplink already does the
  isolating.

## What it is made of

### 1. The instance

A fresh install from a gated build, meaning one that has passed VM 520. It runs the **stable**
channel, not every nightly, because its bugs are public.

**What it holds:** no personal accounts, no keys of Pranab's, no family data. Its sign-ins are its
own: the AIG endpoint and one cloud fallback key.

**Its Mind:**
- runs in its own account with its own memory, which is the continuity the page counts;
- is in `auto` mode, with commands asking once per session (#504); a standing session rule covers
  the commands its own goals need;
- has egress in **enforce** (#503/#508, with the nftables step), allowing only: AIG, the cloud
  fallback, the relay, and its research sources.

**Updates:** it updates to gated builds on its own, at a quiet hour, and the page shows
"maintenance" while it does. Uptime counts the memory's age, not the process's, so an update
doesn't reset "43 days".

**Reaching it:** there are no inbound ports from the internet. Everything it publishes, it pushes
out. Administration is by SSH from the LAN only.

### 2. The work it does

Nothing is staged, so the instance needs **standing goals**: real, open-ended work it returns to
on its own. Proposed:

- **Research notebook.** Follow a few public feeds (arXiv lists, release notes of the projects it
  cares about) and keep a notebook of what it learned, visible on its desktop.
- **Tend its own memory.** Consolidate, correct, link. YantrikDB's own maintenance is honest,
  watchable activity.
- **A small project of its own**, for example a page it maintains about what it's been doing.
- **Idle honestly.** "No action required" is a true pulse line, just not most of them.

The goals live in the instance's config, not its prompt, so they are changed by us rather than by
whatever it reads.

### 2a. Twenty-four hours of real work: the Director

A visitor who arrives at 3 a.m. should find Yantrik doing something. Nothing is staged, so the
activity comes from **standing goals**, handed to the Mind one task at a time by a small scheduler
on the instance, the Director. The Director decides *when* and *what kind*; the Mind decides how,
and may propose its own next task within the goals. That choice is what makes it agency rather
than a loop.

| Kind | Rhythm | What the visitor sees |
|---|---|---|
| **Research round** | every ~30 min | The Browser opens a source from its feeds (arXiv lists, release notes, project blogs). It reads two or three, then writes "What I learned" in Notes. The Pulse shows the entities it links in memory. |
| **Memory upkeep** | every ~20 min | Consolidating, correcting, resolving conflicts. It's cheap, local and honest, and it's the Pulse's heartbeat. |
| **Notebook** | hourly | Tidying and cross-linking notes, and a running "today so far". |
| **Files** | a few times a day | Sorting its research folder into topics in Files. |
| **Numbers** | a few times a day | Turning something it has tracked (benchmarks, release counts) into a Spreadsheet chart. |
| **Journal** | daily; weekly retrospective | A dated journal page, and once a week "what I learned this week". |

Rules that keep it watchable:
- **Something every one to three minutes.** Never more than a few minutes without a Pulse line.
  Gaps between the heavy tasks fill with memory upkeep, which needs no cloud call.
- **Variety.** The same app is never used twice in a row. The desktop is laid out calmly with the
  snap layouts (#505), for example the Browser on the left two-thirds and Notes on the right
  third, so the picture composes itself.
- **Pacing, not token spam.** No text streaming token by token on screen. A task shows a window
  changing, a note appearing, a chart drawn. The Pulse says what is happening in a line.
- **It runs day and night.** The audience is in every time zone. Night only lengthens the
  research interval; it never stops.
- **Budgets.** The Director knows each model provider's free limits and spreads calls across
  them: AIG first, then the free tiers in turn, one per provider. When a budget runs out it
  shifts to local memory upkeep rather than going dark.
- **Fixed kinds of work.** The Director only issues the task kinds above. What they read is the
  public web, allowed host by host in the egress policy. A page it reads can't give it new kinds
  of work.

### 3. The Pulse: what Yantrik is doing, published safely

A small publisher on the instance, `yantrik-pulse`. It turns the OS's own records into public
events:

| Source | Becomes |
|---|---|
| Mind audit (`mind-audit.jsonl`) | "Executed skill: research", "Opened Notes" |
| Harness host turns | "Thinking…", then "Answered in 7s": the turn's kind, never its text |
| YantrikDB | "Recalled 12 memories", "Consolidated 37", the counters |
| Egress ledger | "Reached arxiv.org": allowed hosts only |
| Agents store | "Task started: research" |

**Allowlist, not redaction.** Each source has a list of event types and the fields each type may
carry. Anything not on the list is dropped, not scrubbed. Free text reaches the page only when it
is a field the design names: the current goal's title, a skill's name, an entity's name. Even then
it is length-capped, and checked against the secret-shaped patterns the companion's sanitizer
already knows.

**Delivery:** the publisher pushes to a relay over HTTPS. The relay fans out to browsers over SSE.
The instance never accepts a connection from the internet.

### 4. The desktop, as video

`wf-recorder` captures the labwc output at 2–5 fps. That's enough for a calm desktop and cheap
enough to run for months. ffmpeg pushes it (SRT or RTMP) to MediaMTX on the relay, which serves
it to browsers as WebRTC with an HLS fallback. VNC is never exposed.

The picture is only what the instance itself shows. Because it holds no personal data, there is
nothing on screen to leak. A screen lock or maintenance state shows a card instead of the
desktop.

### 5. The relay and the page

The relay is a small **public** host outside the house, for example the machine that already
serves yantrikdb.com. The instance pushes to it; browsers read only from it. So no public-facing
server runs inside the home network, and the home IP never appears anywhere. The page is
view-only and has no endpoint that reaches the instance. It runs:
- MediaMTX for the video;
- the SSE fan-out for the Pulse, with the last N events buffered so a new visitor sees the recent
  past;
- the counters.

The page is static: player, Pulse, counters, uptime, and a status. It shows "maintenance" or
"offline since …" honestly when the stream stops, never a frozen frame.

## Later: Ask Yantrik

Visitors type; the instance answers on the stream. It is built only once v1 has run for a while.
It depends on what we have built this month:

- Visitor text is **untrusted input**, marked tainted from the first byte (the decision door's
  taint).
- Questions go through a **moderation queue**, first by a model, then by a person for anything
  flagged, and a **rate limit** per visitor and overall.
- The instance answers under a **narrow reach**: it may read its notebook, recall memory and open
  its own apps. It may not reach anything new on the network for a visitor, and egress
  enforcement makes that a machine rule, not a prompt.
- Answers are published like the Pulse: allowlisted, never raw.

## Phases

1. **Instance.**
   - VM on node2, a gated install, its own Mind and memory, AIG plus fallback, egress enforce,
     auto-update at a quiet hour.
   - Done when it runs a week unattended with its counters growing and no manual restarts.
2. **Pulse and relay.** `yantrik-pulse` with its per-source allowlists and tests, and the relay's
   SSE and counters.
   - Done when a week of Pulse contains nothing outside the allowlist. A test replays the week's
     raw records through the publisher.
3. **Video and page.** wf-recorder to MediaMTX, and `yantrikos.com/live` with the maintenance and
   offline states.
4. **Work.** The standing goals, tuned until a day's Pulse is mostly real work.
5. **Ask Yantrik**, as above.

## Open questions

1. **Where the relay runs:** the yantrikdb.com host (warpmode.io), or a new small VPS.
2. **Where `yantrikos.com` itself is served:** `releases.yantrikos.com` is on the LAN.
3. **The standing goals:** which feeds and topics, and what its own project should be.
4. **Which cloud subscription** is the fallback, and its monthly ceiling.
