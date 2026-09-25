# Yantrik OS

[![Chat on Discord](https://img.shields.io/badge/Discord-join%20the%20server-5865F2?logo=discord&logoColor=white)](https://discord.gg/7cDw3jd3Xf)
[![Nightly image](https://img.shields.io/badge/nightly-iso.yantrikos.com-informational)](https://iso.yantrikos.com/nightly/)
[![CI](https://github.com/yantrikos/yantrik-os/actions/workflows/ci.yml/badge.svg)](https://github.com/yantrikos/yantrik-os/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)

An AI-native desktop operating system where the AI **is** the shell. Built in Rust, on Debian
13, shipped as a live ISO you boot and try before you install anything.

Yantrik OS replaces the traditional desktop metaphor with an agent that watches the system,
learns your patterns and helps without being asked, alongside a suite of built-in apps.

> **Come and talk to us: [discord.gg/7cDw3jd3Xf](https://discord.gg/7cDw3jd3Xf)**
> — `#nightly` announces every published image, `#apps-and-surfaces` is where the app control
> protocol gets argued about, and the **help** forum is the place to say what broke. It is a small
> server and a question there is read by the people who wrote the code.

## What makes it different

- **The AI is the shell, not an add-on.** The companion is woven through file management, email
  triage, presentations and system monitoring rather than sitting in a chat window beside them.
- **Every screen is drivable by an agent.** The shell and every app publish their state and
  accept actions over a unix socket — `yos describe shell`, `yos act shell open_app name=notes`.
  Anything a person can do from the keyboard, an agent can do through the same path. See
  [docs/app-control.md](docs/app-control.md).
- **Any mind, and the desktop says which one.** A local model through Ollama or llama.cpp, a
  cloud provider, or a separate agent that attaches — all equally first-class. **The OS itself
  sends nothing anywhere**: no telemetry, no phone-home, no call it makes on its own. But it is
  built to be driven by whatever mind you give it, and a mind you point at a provider talks to
  that provider. The status bar carries a chip naming which mind is answering and whether it is
  local or cloud, so that is never something you have to go and check.
- **Proactive, not reactive.** A four-stage pipeline (Detect → Generate → Score → Deliver)
  decides *when* to speak and *what* to say, so it helps without nagging.
- **Rust throughout.** Slint UI, agent, memory database and system observer. No Electron, no
  Python runtime, no Docker.

## Built-in apps

**Sixteen** application binaries ship, each its own window, each drivable by the agent. Two
more exist in the tree and are **shelved** — deliberately not in this build, and listed below
so that this page cannot promise them.

| App | What it is |
|-----|-----------|
| **Notes** | Markdown notes with semantic search, backlinks, versions |
| **Email** | IMAP client with AI triage and smart notifications |
| **Calendar** | Events, week and day views, reminders |
| **Weather** | Current conditions, hourly and daily forecast, alerts |
| **Terminal** | Terminal emulator with AI command assist and split panes |
| **Editor** | Text editor with tabs, find and replace, go-to-line |
| **yDoc** | Document editor — rich text, comments, change tracking |
| **yPresent** | Presentations — AI deck generation, templates, speaker notes |
| **Images** | Viewer with zoom, rotate, crop, slideshow, batch operations |
| **Downloads** | Resumable transfers with checksum verification |
| **Snippets** | Code snippets by language and collection |
| **Containers** | Docker/Podman containers, images and volumes |
| **Network** | WiFi, ethernet and bluetooth |
| **System Monitor** | CPU, memory, disk, network, processes |
| **Arcade** | Builds a playable game from two small JSON specs, verified headlessly |
| **Studio** | Pictures from a sentence — your own GPU through ComfyUI, a hosted service, or neither |

**Blender**, if it is installed, is drivable too — and it is not ours. The launcher starts it with
an addon that binds the same kind of socket every app here does, so `yos describe blender` reports
the scene, the objects, the engine and the resolution, and `yos act blender add_primitive`,
`set_light`, `set_camera` and `render` work from a terminal or a mind exactly as an app's own
actions do. `render` is `sensitive` and `run_python` is `dangerous`, for the obvious reasons. This
is the first thing on the desktop that shows the control protocol is not a house convention:
a program nobody here wrote can join it by publishing one socket.

**Shelved — not in this build:**

| App | Why | What brings it back |
|-----|-----|---------------------|
| **ySheets** | No cell model behind the grid, so nothing could be typed into it, by mouse or by mind. No formula engine at all | A cell model, CSV load and save, and arithmetic with references plus `SUM`/`AVG`/`MIN`/`MAX`/`COUNT` |
| **Music** | Nothing plays audio — no playback engine, no scanner and no library behind the screen | mpv driven over its JSON IPC socket, a folder scan filling a library store, and play/pause/next/queue and `open <file>` working |

Both crates stay in the tree and keep compiling; what changed is that a person cannot reach
them and a mind is not told they exist. Asking for one gets a refusal that says why, not
"unknown app". The account is
[design/shelved-2026-09-20.md](design/shelved-2026-09-20.md) and the list every surface reads
is `SHELVED` in `crates/yantrik-ui/src/wire/dock.rs`. (Audio does play — the shell's own media
screen drives a real mpv. That was never the Music app.)

The shell itself provides Files, Settings, Memories, Notifications, Bond, Personality,
Permissions, Devices, Packages, Skills and About as screens rather than separate windows.

Every one of them wears the same frame. See [docs/app-sdk.md](docs/app-sdk.md) for why that is
structural rather than a convention anyone has to remember.

## The companion

Not a chatbot. A proactive agent with:

- **Instincts** — email watch, open loops, routine learning, commitment tracking, security
- **Bond** — a relationship that moves from Stranger through Acquaintance, Companion and
  Confidant to Partner, based on the quality of the interaction
- **Memory** — persistent vector-indexed recall that grows over time
- **Model-adaptive behaviour** — detects what the model can do and adjusts tool use and prompt
  complexity to match
- **Pluggable minds** — the built-in companion is one harness among several. Anything that
  speaks the attach protocol can answer instead, managing its own endpoint and credentials.
  Five exist: Yantrik Mind, Hermes Agent, Pi, DeepSeek and OpenClaw — the last four ship as
  source in `harnesses/`, and none of them is started until you configure it. See
  [docs/harness.md](docs/harness.md).
- **YAML plugins** — add tools without writing Rust

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│                        Yantrik OS                          │
│                                                            │
│   yantrik-ui ──────── yantrik-companion ──── yantrik-ml    │
│    (shell)              (agent)               (inference)  │
│        │                    │                              │
│   yantrik-os          yantrikdb                            │
│    (system)            (memory)                            │
│                                                            │
│   16 app binaries · 10 services · one control surface      │
│                                                            │
│   Debian 13 → labwc (Wayland) → Slint                      │
└────────────────────────────────────────────────────────────┘
```

25 crates, 16 shipped apps (18 in the tree, two shelved) and 10 services. The ones worth
knowing:

| Crate | Purpose |
|-------|---------|
| `yantrik-ui` | The shell — Slint UI, app wiring, the control surface |
| `yantrik-companion` | The agent — tools, instincts, bond, personality, proactive pipeline |
| `yantrik-ml` | Inference — LLM backends (Ollama, OpenAI-compatible, llama.cpp, Claude CLI), embeddings, STT/TTS |
| `yantrik-os` | System integration — D-Bus, inotify, sysinfo, battery, network, processes |
| `yantrik-harness` | The attach protocol a third-party mind implements to answer for the shell |
| `yantrik-ui-kit` | The UI kit every app draws from, including the mandatory `AppHeader` |
| `yantrik-app-runtime` | What an app binary is built on — instance guard, theme, IPC, control surface |
| `yantrik-design-tokens` | Colour, type, spacing and size tokens, shared by the shell and every app |

**Threads:** the Slint event loop, a system observer (D-Bus, file watches, polling) and a
companion worker (inference, memory, tools).

## Quick start

### Install

Yantrik OS is a **Debian 13 (trixie) live ISO**, built and boot-tested by CI. Download it,
check the checksum, boot it; the disk installer is inside the running image.

Images are at **<https://iso.yantrikos.com/nightly/>**, where `latest.json` names the current
file, its sha256, its size and its version.

```bash
sh install.sh               # prints url, size and sha256 of the current image, and stops
sh install.sh --download    # also fetches it here and verifies the sha256
```

With no flag it installs nothing, writes nothing and wants no privilege — including when it is
piped into a shell, which is the way it is most often run.

**Only the nightly channel has ever had a build published to it.** `beta` and `stable` are
names the updater knows and nothing has been published to either.

Once it is booted, `yantrik-install` in the live session writes it to a disk.
[docs/getting-started.md](docs/getting-started.md) is the whole path.

There is also `deploy/yantrik-os/cloud-init/user-data.yaml` for Proxmox, libvirt or anything
that takes a cloud-init file. That is how the project's own test machines are made, not how a
person installs this.

### Hardware

The image is about **1.31 GiB**. The Minimum column is what the first-boot hardware scan
checks — the disk figure against the whole size of the disk the installer will write to,
not against free space on the live session. The other two columns are what the image has
actually been booted on:

| | Minimum | CI's boot test, every published image | The project's test machine |
|--|---------|---------------------------------------|----------------------------|
| **CPU** | 2 cores | 4 | 4 |
| **RAM** | 4 GB | 4 GB | 8 GB |
| **GPU** | none | none (virtio-vga, software rendering) | none (virtio-gpu, software rendering) |
| **Disk** | 6 GB, to install | none — boots live | 32 GB |
| **Firmware** | BIOS (UEFI is built, untested) | BIOS | BIOS |

Real hardware, the UEFI path and any GPU other than QEMU's are **not measured** — nobody has
checked. [docs/hardware-requirements.md](docs/hardware-requirements.md) says exactly which
numbers are measured and which are not.

Without a GPU the desktop is fully usable and inference is slow. That trade is deliberate: the
shell renders through Slint's software rasteriser and idles at around 2% of a core.

### LLM backends

The image ships **no model and no mind**. You point it at one during first-run setup.

| Backend | Setup | Where what you type goes |
|---------|-------|--------------------------|
| **Ollama** | Point at a local or remote Ollama server | That machine. Nothing leaves your network |
| **llama.cpp** | Built in, GGUF on disk | Nowhere. Stays on this machine |
| **OpenAI-compatible** | Any endpoint speaking the API | That endpoint's operator |
| **Claude CLI** | Install the Claude Code CLI | Anthropic |

Or attach a separate agent, which keeps its own endpoint and credentials and dials in to the
desktop — see [docs/harness.md](docs/harness.md).

## Updating

```bash
yantrik-update check        # what is installed vs what the channel has
yantrik-update apply        # download, verify, install, restart the session
yantrik-update rollback     # restore the previous build
yantrik-update status       # current build and available backups
yantrik-update set-channel nightly|beta|stable   # which channel this machine follows
```

The bundle is verified against the manifest's sha256 before a file is touched, the current
binaries are backed up first, and if the new shell does not answer its control socket the
update rolls back on its own. Restarting is done through the session unit rather than by
respawning the shell, so it comes back exactly as it does on boot.

| Channel | What it is |
|---------|-----------|
| `nightly` | Latest builds. **The only channel anything has ever been published to** |
| `beta` | Intended for builds promoted from nightly. Empty |
| `stable` | Intended for tested releases. Empty |

`set-channel stable` will succeed and then `check` will report that the channel is not
published, which is the honest answer rather than an error. Machines default to `nightly`
because it is the only one with builds on it.

A machine can also be *ahead* of its channel — a developer deploy from `main`, or a beta build
on a machine moved back to nightly. `check` compares the two builds' positions rather than
their commit hashes, says the machine is ahead, and offers nothing: an older build is not an
update. Installing one is a downgrade, which `apply` refuses unless `--allow-downgrade` names
it as one.

## Driving it from a terminal or an agent

```bash
yos describe shell                          # where you are, what is open, what is wrong
yos act shell open_app name=notes           # launch or focus an app
yos act shell show_screen screen=settings section=ai
yos describe notes                          # any running app answers for itself
yantrik ask "what is using the most disk?"  # ask the companion
```

The same two verbs reach everything, including the things this project did not write:

```bash
yos ls                                      # what is open, and what can be opened
yos act arcade build game="Tuk's Teal Morning"
yos act arcade verify game=tuk-s-teal-morning   # boots, no console errors, a bot wins, a bot loses
yos act blender add_primitive kind=monkey       # Blender, through the addon the launcher starts it with
yos act blender render output=/tmp/suzanne.png  # {"path": …, "seconds": 1.53, "bytes": 368298}
yos act image-viewer open path=/tmp/suzanne.png # and look at it, on the same desktop
```

Actions are graded safe, standard, sensitive or dangerous. `yos-mcp` exposes the same surface
over MCP.

**How often you are asked is a mode you set**, from the chip in the status bar beside the mind
chip, or in Settings → AI & Intelligence:

| mode | the mind may… |
|---|---|
| `plan` | read only — every change is refused and it has to tell you what it *would* do |
| `ask` | routine things run; sensitive ones put a card in front of you (the default) |
| `auto` | sensitive things run; you are still asked about destructive ones |
| `bypass` | nothing is asked. Time-boxed — 15 minutes, an hour, or until the shell restarts |

When a mode says to ask, a card says who is asking, what it will do and with which arguments, and
offers **Allow once**, **Deny**, and — for anything recoverable — **Allow for this session**,
which stops the asking for that one action until the shell restarts and is listed in the mode
menu with a ✕ beside it. Only those clicks grant anything: no action on any control surface can
grant, and none can make the desktop more permissive either. The one published action about modes,
`set_mind_mode`, can only tighten, so a mind can put itself into plan mode and can never take
itself out.

Nothing graded above the machine's own ceiling (`tool_permission`, on the AI page in Settings) is
ever run or even asked about, in any mode — bypass included. Bypass is never written to disk, so
a machine never boots into it. Everything that runs without you being asked is written down, in
the mode menu ("See what it did without asking") and in `~/.local/share/yantrik/mind-audit.jsonl`.

`YOS_MCP_MAX_PERMISSION` still exists as a cap a harness puts on itself. It can only ever be
*stricter* than the desktop's mode — it turns an unasked run into a card — and never looser.
Leave it unset and the desktop's mode is the whole policy.

Because a call may wait for a person, **an MCP client must allow `os_act` up to 270 seconds**.
A client that gives up sooner cuts the person off mid-decision. The wait does not make the
bridge deaf: pings, `tools/list` and other tool calls are all answered while a card is up, so a
client's liveness check has no reason to declare the server dead and restart it. For Hermes:

```yaml
mcp_servers:
  yantrik_os:
    command: /opt/yantrik/bin/yos-mcp
    timeout: 300
```

## Configuration

One YAML file at `/opt/yantrik/config.yaml`:

```yaml
user_name: "Your Name"
companion_name: "Yantrik"

backend: "api"                      # api, claude-cli, or llamacpp
api_url: "http://localhost:11434"
api_model: "qwen3:8b"

features:
  resource_guardian:
    enabled: true
    battery_warning_threshold: 20
  email_watch:
    enabled: true
    check_interval_minutes: 5
```

Themes live at `~/.config/yantrik/theme-override.yaml`; the token list is in
[docs/CONTRIBUTING.md](docs/CONTRIBUTING.md).

## Development

### Prerequisites

- Linux, or Windows with WSL2 — the workspace builds on Debian/Ubuntu
- Rust 1.92+ (Slint 1.17 requires it)

### Build and test

```bash
cargo build --workspace
cargo test --workspace
```

The tests are worth running for their own sake: a good number of them exist to stop a specific
mistake returning — that every shipped app draws its header with the shared component, that the
screen a caller asks for is the screen the shell draws, that an application answers to one name
on every surface, that the release script packages the directory it built into.

### Release

```bash
deploy/yantrik-os/build-release.sh --publish nightly
```

Discovers what the OS is made of rather than reading a list, packages it, uploads it, verifies
what is actually being served matches what was built, and prunes the channel.

### Looking at it

```bash
scripts/screen-survey.sh                 # photograph every screen, section and app
scripts/screen-survey.sh --only apps
```

Design review needs the actual pixels. Nearly every defect worth finding in this project was
found by looking at a running machine, and almost none of them were visible in the source.

### Project structure

```
yantrik-os/
├── crates/                    23 crates
│   ├── yantrik-ui/            the shell
│   │   ├── src/wire/          one module per screen, wiring UI to system
│   │   ├── src/features/      proactive features
│   │   └── src/control*.rs    the agent-facing control surface
│   ├── yantrik-ui-slint/ui/   the shell's Slint markup, one file per screen
│   ├── yantrik-ui-kit/slint/  the shared components every app draws from
│   ├── yantrik-companion/     the agent
│   ├── yantrik-ml/            inference
│   ├── yantrik-os/            system observer
│   └── yantrik-harness/       the pluggable-mind protocol
├── apps/                      17 application binaries, 15 of which ship
│   └── desktop-files/         their freedesktop entries
├── harnesses/                 minds that attach: hermes, pi, deepseek, openclaw, and the half they share
├── services/                  10 background services
├── config/labwc/              compositor config, theme and autostart
├── deploy/yantrik-os/         cloud-init, session, release and update scripts
├── scripts/                   probes and the screen survey
└── docs/
    ├── getting-started.md     download, verify, boot, attach a mind, install, update
    ├── hardware-requirements.md  what it has actually been run on, and what is unmeasured
    ├── architecture.md        system design
    ├── app-sdk.md             how to write an app, and why the frame is not yours
    ├── app-control.md         how apps publish state and accept actions (the guide)
    ├── surface-protocol.md    the protocol itself, normative; schema/ beside it
    ├── harness.md             attaching a different mind
    ├── footprint.md           what it costs to run, and where that goes
    └── CONTRIBUTING.md        contributor guide
```

## Privacy and security

- **The OS itself sends nothing anywhere.** No telemetry, no phone-home, no analytics, no call
  it makes on its own behalf. The image ships pointed at loopback and at nothing else, because
  an endpoint baked into a public image is an endpoint every copy of it talks to. The two
  places it does reach out are ones you asked for: the update check against
  `releases.yantrikos.com`, and whatever your weather, mail and calendar are configured
  against — and a problem report you chose to send. When an app crashes, the desktop writes a
  record locally, scrubbed of your home directory, your user name and anything shaped like a
  key; *Report a problem* shows you that exact file, and only pressing Send moves it, to
  `report.yantrikos.com`, with no name, hostname or address attached.
- **The mind is a separate question, and it is yours to answer.** This OS is built to be
  driven by any mind, cloud models included; that is the point of the harness protocol, and
  the token-efficiency comparison this project publishes was itself measured against a cloud
  model. Point it at Ollama or llama.cpp and nothing you type leaves your machine. Point it at
  a provider and what you type goes to that provider, exactly as it would from any other
  client. The OS never holds your key — an attached mind manages its own credentials and the
  protocol has nowhere to put one.
- **The desktop says which.** A chip in the status bar names the mind that is answering and
  whether it is local or cloud, with the provider or model on it. It is shown the moment
  something other than the built-in companion is answering, because at that point it is the
  most important thing in the bar.
- **The memory is yours.** It lives at `/opt/yantrik/data/` as plain SQLite you can read,
  export or delete.
- **Graded permissions.** Tools are safe, standard, sensitive or dangerous; the last two need
  explicit approval.
- **Path sandboxing.** File tools refuse `.ssh`, `.gnupg` and similar.
- **Applications are not sandboxed.** Software installed through the package manager runs with
  your own access, as on any ordinary Debian desktop. Said plainly here rather than left to be
  discovered.

## License

GPL-3.0. See [LICENSE](LICENSE).

## Community

**[discord.gg/7cDw3jd3Xf](https://discord.gg/7cDw3jd3Xf)** — a small server, read by the people who
wrote the code.

| Where | For |
|-------|-----|
| `#nightly` | every published image, with its version, size and sha256 |
| `#apps-and-surfaces` | the control protocol — `describe`/`act`, grades, what an app owes a mind |
| `#minds` | attaching a harness: Hermes, Pi, OpenClaw, DeepSeek, or one you wrote |
| **help** forum | say what broke; the audits under `design/` mean nothing here needs defending |
| `#showcase` | what you made with it |

Issues are at [github.com/yantrikos/yantrik-os/issues](https://github.com/yantrikos/yantrik-os/issues).
The ones labelled [`good first issue`](https://github.com/yantrikos/yantrik-os/issues?q=is%3Aopen+label%3A%22good+first+issue%22)
are real and unassigned — say which one you want and it is yours.

## Links

- **Images (nightly)**: https://iso.yantrikos.com/nightly/
- **Install helper**: https://get.yantrikos.com/install.sh
- **Release bundles**: https://releases.yantrikos.com
- **Issues**: https://github.com/yantrikos/yantrik-os/issues
- **Discord**: https://discord.gg/7cDw3jd3Xf
