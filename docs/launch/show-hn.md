# Show HN: draft

**Draft only. Nothing here has been posted.** Every claim below was checked against the code or
the docs on `origin/main` when this was written; where a claim is "planned" it says so. Re-check
the numbers that move (image size, nightly version) on the day.

Before posting: record the demo (README has a placeholder), confirm the latest nightly boots, and
make sure `https://iso.yantrikos.com/latest.iso` downloads. Post a weekday morning US Pacific, and
be at the keyboard for the first three hours.

## Title options (80 characters max)

1. `Show HN: Yantrik OS – a Linux desktop AI agents can operate, with your approval` (79)
2. `Show HN: Yantrik OS – Debian desktop where AI agents use typed APIs, not pixels` (79)
3. `Show HN: A Linux desktop where every app tells AI agents what it can do` (71)

Recommended: 1. It says what it is and the trade (your approval) in the title. The URL to submit
is the GitHub repository, not the ISO.

## First comment (from Pranab Sarkar)

> Hi HN, I'm Pranab. Yantrik OS is a Debian 13 desktop built so AI agents can work on it beside
> you, safely.
>
> Why: agents increasingly do real work on a person's machine, today through screenshots and
> guessed clicks or a CLI with full trust. I wanted the desktop itself to be the interface. Every app and screen publishes what it holds and what it can
> be asked to do (`yos describe notes`, `yos act notes append text=...`), as typed actions on a
> unix socket. An agent uses the same path a person's button does.
>
> What's different: each action has a grade (safe, standard, sensitive, dangerous). A permission
> gate decides what runs without asking, and a sensitive action puts an approval card in front of
> you. Whatever ran unasked is written to a log you can read. The built-in Mind runs under its own
> system account, with its network egress forced through a local proxy that records where it
> connects (and can restrict it). Agents attach to the desktop and keep their own model and key: Hermes, Pi,
> OpenClaw, DeepSeek and the built-in Mind all work that way.
>
> Trade-offs, honestly: the image ships no model, so you need a provider account (a free one
> works) or a local GPU for the agents to be useful. The update manifest is signed (ed25519), but
> the ISO you first download is only checked against a sha256 from the same host, and the nightly
> signing key is a CI secret. The Email app's passwords and your saved provider keys are in the
> credential vault; the built-in companion's own mail account still keeps a password in plain
> text (#1022). An agent you start in a terminal runs as you, so the gate holds it only where it
> goes through `yos`. Applications are not sandboxed. It's nightly-quality: CI boots every image
> in QEMU, a fresh-install test run installed it under UEFI on Proxmox, and real hardware is untested.
>
> It's Rust and Slint, GPL-3.0, no telemetry (it does look up its own location once, from your IP,
> unless you set `YANTRIK_NO_GEOLOCATE=1`). Images: https://iso.yantrikos.com/nightly/
>
> What I'd like from you: try it in a VM and tell me where the approval model is annoying or
> wrong, try to break the gate (SECURITY.md has the private route), and tell me what app you'd
> want an agent to drive. Writing a YantrikApp or attaching your own agent is a good first
> contribution (CONTRIBUTING.md). I'll be here all day.

(About 405 words, counting the comment only. Trim the last paragraph first if it needs to be shorter.)

## Prepared answers: the ten hardest questions

Short enough to paste, in Pranab's voice. Each is true per the code; the source is in brackets.

### 1. "Giving an AI agent access to my desktop is a terrible idea. How is this secure?"

It's a risk, so the design assumes the agent might be wrong or manipulated rather than trusting it.
Every action has a grade, and what runs unasked depends on a mode you set (`plan` is read-only,
`ask` is the default and puts a card up for sensitive actions). A machine-wide ceiling caps what
can run at all, even in bypass, and nothing on the control surface can grant a permission or
loosen the mode. What ran without asking is logged. The built-in Mind runs as its own system
account with its egress held by the kernel to a local proxy. The limits: an agent you start
yourself runs as you, so the gate holds it only where it goes through `yos`/`yos-mcp`; apps are
not sandboxed; and several security issues are open and public (#1022, #1027, #548 among them).
[README modes table; `docs/app-control.md`; `design/mind-egress-2026-09-29.md`; issues #411,
#1022, #1027, #548]

### 2. "Why not just use Omarchy?"

Omarchy is a good distro and it does something different: it pre-wires coding-agent CLIs onto an
Arch/Hyprland setup, and the agent works the way it would in any terminal, with the access you give
it. Yantrik's bet is that the desktop should be a typed interface the agent works through, with a
permission model and a log between the agent and the apps. If you want a polished tiling setup and
trust your agents, Omarchy is the better pick; if you want to see and gate what an agent does
across your apps, that's what we're building. (I have not benchmarked either against the other.)

### 3. "Why a whole OS? This should be an app or a protocol."

Part of it is a protocol: the surface protocol is documented, there is a Python and a Rust SDK, and
a program we didn't write (Blender, through an addon) already publishes one. But the guarantees
need the OS: the gate and approval cards live in the shell, the Mind's egress is held by the
kernel via nftables on its own account, and "the person's button and the agent's action are the
same command" needs the apps to be built that way. An app on someone else's OS can't make those
promises. That is also why the SDK is usable on plain Debian apps without the rest. [`docs/sdk/README.md`;
`apps/blender`; `docs/architecture.md`]

### 4. "What model do I need? Does my data leave the machine?"

The image ships no model. You need either an account with a provider (a free one works; any
OpenAI-compatible endpoint, or the Claude CLI) or a local GPU with Ollama or llama.cpp. Without a
GPU it works but inference is a token or two a second. Point it at local Ollama or llama.cpp and
what you type stays on your network; point it at a provider and it goes to that provider, as with
any client. The Lens names the Mind that is answering and the Mind panel shows what it runs on;
the top bar has no such chip today (#1162 is the fix in review). [`docs/getting-started.md`
section 5]

### 5. "Telemetry?"

No analytics, and nothing of yours is sent in the background. But it is not silent: when it knows
no location it asks ipinfo.io once, which tells that service your public IP (`YANTRIK_NO_GEOLOCATE=1`
turns it off; the installers ask ip-api.com the same way), and after first run it downloads and
installs Hermes from the internet. Otherwise it reaches out where you pointed it: the update
check against `releases.yantrikos.com`, whatever your weather, mail and calendar are configured
against, and a problem report that you can read first and that only leaves when you press Send.
Crash records are scrubbed locally. A Mind you point at a cloud model talks to that model's
provider. [README "Privacy"; `crates/yantrik-ui/src/wire/location.rs`; `docs/getting-started.md`]

### 6. "Are updates signed?"

The update information is, since 30 Oct (#537, PR #1020). Each channel serves one signed envelope
(ed25519); `yantrik-update` checks it before it reads a single field, and refuses one that is
unsigned, signed by a key not allowed for that channel, expired (60 days) or carrying an older
serial than the last one accepted. Then it checks the bundle's sha256 against that manifest,
backs up the current binaries, and rolls back if the new shell doesn't answer its control socket.
The limits: the public keys come with the build you already have, so the first image you
download is trusted only through the sha256 in `latest.json` on the same host (the ISO itself is
not signed); the nightly signing key is a CI secret, so whoever controls that CI can sign a
nightly; beta and stable have no CI key and nothing has been published to them. A fresh install (VM 542, 10 Nov) runs `yantrik-update check` against the live signed nightly and
reports it up to date. [issue #537; PR #1020;
`deploy/yantrik-os/yantrik-update-verify`]

### 7. "Why Debian?"

Because every Debian program installs and runs, the hardware support is the one people already
know, and I didn't want to maintain a distribution's worth of packaging. The image is Debian 13
(trixie) with labwc as the Wayland compositor. (An earlier version of the project started on
Alpine and moved; the docs say so.) [`docs/getting-started.md`; `docs/hardware-requirements.md`]

### 8. "Why Rust and Slint?"

One language from the shell to the control surface to the memory engine, with no Electron, no
Python runtime and no Docker needed to run the desktop. Slint renders in software, which is what
lets it run on a VM with no GPU: the project's measurement on the 2409 nightly (VM 542, 10 Nov, lock screen,
no model attached) was the shell idling at about 2% of a core and about 250 MB of memory (PSS)
for the shell and about 265 MB with its services; `docs/footprint.md`'s 186 MB was WSL2 in
September and is out of date. The cost is that Slint is a smaller ecosystem than GTK or Qt,
and the generated UI crate is large. [`docs/footprint.md`; `Cargo.toml`]

### 9. "What's the business model? And the license?"

The code here is GPL-3.0 and that is the whole license; there's no paid tier or CLA in the
repository. **[Pranab: fill in the honest answer about funding and plans here. The repo does not
state one, so this draft does not invent it.]** YantrikDB, the memory engine, is a separate public
repository that the build pins by revision.

### 10. "How is this different from computer-use agents?"

Computer-use agents look at pixels and click. That works anywhere, but it's slow, costly, and it
can't tell a safe click from a destructive one. Here our own apps answer `describe` with exact state
(a few hundred bytes) and take named, graded actions, so the gate can know what an action is before
it runs. For software we didn't write, a pixel fallback exists (`yos screen text` reads a display's
text; there is an `analyze_screen` vision tool) and the person's desktop asks them first. Using
the accessibility tree for third-party apps is the intended middle path, but the a11y service is
not finished (#39, #40), so don't count on it today. The honest limit: this only helps for apps
that publish a surface, and today that is ours plus Blender and a LibreOffice adapter. [`docs/app-control.md`
"Three ways to see"; `deploy/yantrik-os/yos`; issues #39, #40; `adapters/libreoffice`]

## Likely follow-ups to be ready for

- **"Does it run on my laptop?"** Unknown. Only QEMU (BIOS in CI) and one Proxmox VM under UEFI
  are measured. Say so and ask for reports.
- **"Why isn't there a stable release?"** Only the nightly channel has ever been published to.
- **"Is the ISO reproducible?"** Not claimed; do not answer yes.
- **"The Discord is a red flag."** It's optional; issues and GitHub discussions are the record.
