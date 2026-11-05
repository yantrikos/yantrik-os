# Where to share, after Show HN: draft

**Draft only. Nothing here has been posted.** Post one place at a time, a day or more apart, and
read each community's self-promotion rules first; several of the places below limit it. The angle
changes with the audience, and so does the first sentence. The honest limits (no model in the
image, unsigned update manifest, QEMU-only hardware testing) belong in every post, because each
audience will find them within minutes.

Every post links to the repo and the nightly, and asks one specific question.

## Order

1. Show HN (see [show-hn.md](show-hn.md)).
2. Within the week: r/LocalLLaMA, the Hermes and OpenClaw communities, r/selfhosted.
3. After feedback has been absorbed: r/linux, Lobsters, the Slint showcase, DistroWatch, a blog post.

## Channels

### r/LocalLLaMA (the local-model lane)
- **Angle:** an agent desktop that does not need a cloud. Point it at Ollama or llama.cpp and
  nothing you type leaves your network; a status-bar chip says whether the answering Mind is
  local or cloud. Small local models are the hard case. The project has open work on them
  (compact tool schemas #452, escalating a stalled step #453); only say what ships.
- **Say plainly:** the image has no model. A GPU on the LAN or a provider key is needed for a
  usable one. CPU-only gives a token or two a second.
- **Ask:** which local models drive the control surface well, and where they fail.
- **Format:** a short text post with the demo GIF, not a link drop.

### r/selfhosted
- **Angle:** the data stays yours: the built-in Mind's memory is plain SQLite on your machine,
  the OS sends no telemetry, it is GPL-3.0, and the egress proxy shows where the Mind account
  connects. Works with a local Ollama on the LAN.
- **Say plainly:** it is a desktop OS, not a server app; a VM on a Proxmox or libvirt box is the
  realistic way to try it. Nightly only.
- **Ask:** what the egress policy should allow by default for a self-hosted setup.

### r/linux
- **Angle:** a Debian 13 desktop (labwc, Wayland) with a typed control interface. Read the
  subreddit rules first, since it is strict about AI posts. Lead with the desktop and the
  protocol, not with "AI". Be ready for "I don't want AI in my OS": the desktop is fully usable
  without a model, and no Mind is started until you configure it.
- **Ask:** hardware reports. This is the thing we cannot test ourselves.

### Lobsters
- **Angle:** the design, not the product: the `describe`/`act` protocol, graded actions, and why
  the person's button and the agent's action are the same command
  (`design/app-framework-2026-10-21.md`). Tags: `linux`, `rust`, `ai`.
- **Needs an invite or an existing member** to submit; their norms discourage frequent posting
  of your own projects, so do it once.

### Hermes Agent / Nous Research community
- **Angle:** Hermes is integrated as a Mind. It attaches through the harness protocol
  (`harnesses/hermes`), and `yos-mcp` gives it the desktop as MCP tools. Frame it as "here is a
  desktop your agent can operate with typed actions and an approval model".
- **Be upfront about** two open issues that are ours to fix: #676 (the desktop owner refused as
  "Unauthorized user" on Hermes 0.21.x) and #712 (the install leaves Hermes's launcher pointing
  at a temporary Python).
- **Ask:** which Hermes versions to test against, and whether they want the MCP setup documented
  upstream. #553 (a good first issue) makes Hermes report its endpoint.

### OpenClaw / ClawHub community
- **Angle:** OpenClaw ships as a harness in `harnesses/openclaw`. A skill that wraps `yos` would
  give OpenClaw users a way to operate a desktop with approvals.
- **Ask:** a review of our adapter, and whether a skill belongs on ClawHub. Check ClawHub's
  publishing rules before proposing anything; this draft has not.

### Rust and Slint communities ("Made with Slint")
- **Angle:** a large real Slint codebase: a full shell and a set of apps, a shared design-token
  and UI-kit crate, software rendering with no GPU. What we learned is written up in
  `docs/app-sdk.md` (why a shared header takes its content as data, not as `@children`).
- **Action:** submit to Slint's showcase following their current contribution instructions; post
  in Slint Discussions; for r/rust, use the community's show-off thread if one is running rather
  than a standalone post.
- **Ask:** advice on very large generated UI crates (`yantrik-ui-slint` is one 55k-line generated
  module) and on window handling.

### DistroWatch
- **Angle:** a new distribution submission, written factually: Debian 13 based, labwc on
  Wayland, x86-64, live ISO, GPL-3.0, nightly builds only.
- **Check first:** DistroWatch has listing criteria (a downloadable, working release and active
  development). A nightly-only project may be asked to wait. Use their submission form and do
  not lobby.

### Blog post (outline, about 1,200 words)
Working title: "Giving agents a typed desktop instead of a screenshot".
1. The problem: agents on a desktop today, and one real failure from the `design/` audits, with
   the evidence.
2. The idea: `app.describe` and `app.act`, with a ten-line `yos` transcript.
3. The gate: grades, modes, the approval card (screenshot), the audit log.
4. Minds and egress: why the built-in Mind has its own account and what that does and does not
   cover. Say that third-party harnesses currently run as you.
5. What is not done: the unsigned manifest, the vault migrations, QEMU-only testing.
6. Where help is wanted, linking the help-wanted issues.
Publish on a personal blog or the project's own site; cross-post only after HN.

## Potential collaborators

One line on why, and the ask. These are suggestions to approach respectfully. None has been
contacted, and nothing here is a commitment on anyone's part.

| Project | Why it is a natural fit | The ask |
|---|---|---|
| **Hermes Agent (Nous Research)** | Already a Mind on the desktop; a direct user of `yos-mcp`. | A tested-versions matrix, and a docs mention of the MCP setup. |
| **OpenClaw / ClawHub** | Shipped as a harness; a skill wrapping `yos` would put the approval model in front of its users. | Review of our adapter; whether a skill belongs on ClawHub. |
| **Pi** | Shipped as a harness. | Review the adapter and report what breaks. |
| **Slint** | Our whole UI, at a scale that finds limits. | A showcase entry, and feedback on large generated UI crates. |
| **labwc** | The compositor under the shell. #849 and #877 want the compositor and the shell to know more about each other's windows. | Advice on window rules and foreign-toplevel use, before we patch anything. |
| **Ollama and llama.cpp** | The local-model lane. | A tested-configurations note, and which small models handle tool calling well. |
| **Blender addon authors** | The clearest proof that a program we did not write can join the protocol (`apps/blender/addon`). | Feedback on the addon. |
| **LibreOffice community** | We ship an adapter (`adapters/libreoffice`) that drives it from beside. | Review of the adapter; guidance on stable surfaces to drive. |
| **GNOME and KDE accessibility (AT-SPI) developers** | Our a11y service is unfinished (#39, #40), and reading third-party apps semantically is the intended path. | Pointers to the right libraries; a contributor if someone is interested. |
| **MCP client maintainers** | `yos-mcp` can wait for a person's decision for up to 270 seconds, and a client that gives up sooner cuts the person off. | Guidance on long-running tool calls that wait for a human. |
| **Other agent-on-Linux projects** (Omarchy, AgenticLinux, agent-workspace-linux, the server-side editions) | They solve neighbouring problems: pre-wired agent tooling, immutable images, a separate agent desktop. | Not a pitch. Ask whether a shared way to describe "what an app lets an agent do" would be useful to them, and only if they are interested. |
