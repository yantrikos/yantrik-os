# Yantrik OS

An AI-native desktop operating system where the AI **is** the shell. Built entirely in Rust with a local-first philosophy — your data stays on your machine, your AI runs on your hardware.

Yantrik OS replaces the traditional desktop metaphor with an intelligent companion that watches your system, learns your patterns, and proactively helps — while giving you a full suite of built-in productivity apps.

## What Makes It Different

- **AI is the shell, not an add-on.** The companion isn't a chatbot bolted onto a desktop — it's woven into every layer: file management, email triage, presentation generation, spreadsheet formulas, system monitoring.
- **Local-first AI.** Runs on-device using quantized open models (Qwen 3.5). No cloud dependency. Works offline. Your conversations and memories never leave your machine.
- **Proactive, not reactive.** A 4-stage pipeline (Detect → Generate → Score → Deliver) decides *when* to speak, *what* to say, and *how* to say it — so the companion helps without nagging.
- **Single binary.** One Rust binary: Slint UI + AI agent + memory database + system observer. No Electron, no Python, no Docker.
- **Runs on modest hardware.** 4GB RAM minimum. Ships with an offline 4B LLM for full offline capability. GPU recommended for interactive AI responses (~70 TPS with GPU vs ~1 TPS on CPU-only). Works in VirtualBox, QEMU/KVM, Proxmox, or bare metal.

## Built-in Apps

| App | Description |
|-----|-------------|
| **ySheets** | Spreadsheet with formula engine, AI data generation, formatting, multi-sheet tabs |
| **yPresent** | Presentation editor with AI deck generation, templates, speaker notes, slideshow mode |
| **yDocs** | Document editor with rich text, AI writing assistance, word count |
| **Email** | IMAP email client with AI-powered triage and smart notifications |
| **Calendar** | Event management with schedule awareness |
| **Weather** | Local weather with forecasts |
| **Music Player** | Audio playback with playlist management |
| **Files** | File browser with AI-assisted organization |
| **Terminal** | Built-in terminal emulator |
| **Notes** | Quick notes with search |
| **System Monitor** | CPU, RAM, disk, network, process management |
| **Network Manager** | WiFi and ethernet configuration |
| **Package Manager** | System package management |
| **Settings** | System and companion configuration, themes, privacy controls |

All apps have AI integration — ask the companion to generate spreadsheet data, write presentation slides, draft emails, or explain system processes.

## The Companion

The AI companion is not a chatbot. It's a proactive agent with:

- **9+ instincts** — Email watch, open loops guardian, routine learning, commitment tracking, security monitoring, and more
- **Bond system** — Relationship evolves from Stranger → Acquaintance → Companion → Confidant → Partner based on interaction quality
- **Memory** — Persistent vector-indexed memory that grows over time. The companion remembers your preferences, past conversations, and solutions to problems you've encountered
- **Model-adaptive intelligence** — Automatically detects model capabilities and adjusts tool usage, prompt complexity, and agent behavior to match
- **Proactive pipeline** — 4-stage system (Detect → Generate → Score → Deliver) with silence policy to avoid notification fatigue
- **116+ tools** — File operations, git, browser automation, email, calendar, system commands, memory search, web browsing, and more
- **YAML plugins** — Extend tool capabilities without writing Rust

## Architecture

```
┌─────────────────────────────────────────────────┐
│                  Yantrik OS                      │
│                                                  │
│  ┌──────────┐  ┌──────────────┐  ┌───────────┐  │
│  │ yantrik  │  │   yantrik    │  │ yantrik   │  │
│  │   -ui    │←→│  -companion  │←→│   -ml     │  │
│  │  (Shell) │  │   (Agent)    │  │  (AI/LLM) │  │
│  └────┬─────┘  └──────┬───────┘  └───────────┘  │
│       │               │                          │
│  ┌────┴─────┐  ┌──────┴───────┐                  │
│  │ yantrik  │  │  yantrikdb   │                  │
│  │   -os    │  │   -core      │                  │
│  │ (System) │  │  (Memory DB) │                  │
│  └──────────┘  └──────────────┘                  │
│                                                  │
│  Alpine Linux → labwc (Wayland) → Slint UI       │
└─────────────────────────────────────────────────┘
```

**5 crates, 3 threads:**

| Crate | Purpose |
|-------|---------|
| `yantrik-ui` | Desktop shell — Slint UI, app wiring, main binary |
| `yantrik-companion` | AI agent — tools, instincts, bond, personality, proactive pipeline |
| `yantrik-ml` | AI inference — LLM backends (Ollama, OpenAI API, llama.cpp), embeddings, TTS |
| `yantrikdb-core` | Memory database — SQLite + HNSW vector search, knowledge graph, vault |
| `yantrik-os` | System integration — D-Bus, inotify, sysinfo, battery, network, process monitoring |

**Threads:**
1. **Main** — Slint UI event loop, app rendering
2. **System Observer** — D-Bus listeners, file watchers, process polling
3. **Companion Worker** — LLM inference, memory queries, tool execution

## Quick Start

### One-line Install (Alpine Linux)

```bash
curl -fsSL https://get.yantrikos.com/install.sh | sh
```

Or download and run manually:

```bash
wget https://releases.yantrikos.com/stable/install.sh
chmod +x install.sh
./install.sh
```

The installer will:
1. Detect your hardware (CPU, RAM, GPU, hypervisor)
2. Install system dependencies (labwc, mesa, fonts, audio)
3. Download Yantrik binaries
4. Download AI models (embedder, small fallback LLM)
5. Walk you through configuration (your name, companion name, LLM backend)
6. Configure the Wayland compositor and auto-login
7. Set up automatic updates

### Hardware Requirements

| | Minimum | Recommended |
|--|---------|-------------|
| **CPU** | x86_64, 2 cores | 4+ cores |
| **RAM** | 4 GB | 8+ GB |
| **Disk** | 4 GB free | 10+ GB free |
| **GPU** | Not required (UI works without) | **NVIDIA/AMD recommended** for interactive AI |
| **OS** | Alpine Linux 3.18+ | Alpine Linux 3.23 |
| **Platform** | VirtualBox, QEMU, Proxmox | Bare metal |

### LLM Backend Options

| Backend | Setup | Quality | Speed |
|---------|-------|---------|-------|
| **Ollama** | Point to local/remote Ollama server | Best (use any model) | Depends on hardware |
| **Claude CLI** | Install Claude Code CLI | Excellent | Cloud latency |
| **OpenAI API** | Any OpenAI-compatible endpoint | Varies | Cloud latency |
| **Built-in offline** | Included — Qwen 3.5 4B GGUF | Strong (reasoning, structured output) | ~70 TPS with GPU, very slow on CPU-only |

## Updates

Yantrik OS updates automatically via the built-in upgrade mechanism:

```bash
# Check for updates
yantrik-upgrade check

# Update to latest stable
yantrik-upgrade stable

# Switch to nightly builds
yantrik-upgrade nightly
```

Updates include SHA256 verification and automatic rollback if the new version fails to start.

### Release Channels

| Channel | Description |
|---------|-------------|
| `stable` | Tested releases, recommended for daily use |
| `beta` | Preview releases, promoted from nightly |
| `nightly` | Latest builds, may have rough edges |

## Configuration

Yantrik OS uses a single YAML config file at `/opt/yantrik/config.yaml`:

```yaml
user_name: "Your Name"
companion_name: "Yantrik"

# LLM backend
backend: "api"              # "api", "claude-cli", or "llamacpp"
api_url: "http://localhost:11434"
api_model: "qwen3:8b"

# Proactive features
features:
  resource_guardian:
    enabled: true
    battery_warning_threshold: 20
  email_watch:
    enabled: true
    check_interval_minutes: 5
  focus_flow:
    enabled: true
    deep_work_threshold_mins: 20
```

### Themes

Create `~/.config/yantrik/theme-override.yaml` to customize colors:

```yaml
name: "Nord"
enabled: true
bg_deep: "#2e3440"
bg_surface: "#3b4252"
accent: "#81a1c1"
text_primary: "#eceff4"
```

See [CONTRIBUTING.md](docs/CONTRIBUTING.md) for the full list of theme tokens.

### YAML Plugins

Extend the companion with custom tools — no Rust required:

```yaml
# ~/.config/yantrik/plugins/my-tools.yaml
name: "my-tools"
version: "1.0"
tools:
  - name: "check_vpn"
    description: "Check if VPN is connected"
    permission: "safe"
    command: "mullvad status"

  - name: "deploy_staging"
    description: "Deploy branch to staging"
    permission: "sensitive"
    parameters:
      branch:
        type: "string"
        required: true
    command: "cd ~/projects && ./deploy.sh {branch}"
```

## Development

### Prerequisites

- Windows 11 with WSL2 (Ubuntu) or native Linux
- Rust 1.93+
- VirtualBox or QEMU for testing

### Build

```bash
# From WSL2 or Linux
cd /path/to/yantrik-os
CARGO_TARGET_DIR=/home/$USER/target-yantrik cargo build --release -p yantrik-ui -p yantrik
```

### Deploy to Test VM

```bash
# Build + deploy to VirtualBox VM
bash deploy.sh

# Skip rebuild, deploy existing binaries
bash deploy.sh --skip-build
```

### Project Structure

```
yantrik-os/
├── crates/
│   ├── yantrik-ui/          # Desktop shell (main binary)
│   │   ├── src/
│   │   │   ├── main.rs      # Entry point
│   │   │   ├── wire/        # App backends (one .rs per app)
│   │   │   ├── features/    # Proactive features
│   │   │   └── apps.rs      # App registry
│   │   └── ui/              # Slint UI files
│   │       ├── desktop.slint
│   │       ├── spreadsheet.slint
│   │       ├── presentation.slint
│   │       └── ...
│   ├── yantrik-companion/   # AI agent
│   │   └── src/
│   │       ├── companion.rs # Agent loop
│   │       ├── instincts/   # Proactive instincts
│   │       ├── tools/       # Tool implementations
│   │       ├── cortex/      # Pattern recognition, playbooks
│   │       └── bond.rs      # Relationship system
│   ├── yantrik-ml/          # AI inference
│   │   └── src/
│   │       ├── llm/         # LLM backends (api, claude_cli, llamacpp, fallback)
│   │       ├── capability.rs # Model-adaptive intelligence
│   │       └── embeddings.rs
│   ├── yantrikdb-core/      # Memory database
│   └── yantrik-os/          # System observer
├── install.sh               # Universal installer
├── deploy.sh                # Dev deploy to VM
├── deploy-release.sh        # Release publisher
├── build.sh                 # Component build system
└── docs/
    ├── architecture.md      # System design
    ├── CONTRIBUTING.md      # Contributor guide
    └── getting-started.md   # Installation walkthrough
```

## CLI

Yantrik also ships a CLI for headless/SSH usage:

```bash
# Ask the companion a question
yantrik ask "What's using the most disk space?"

# With JSON output
yantrik ask --json "Summarize my recent emails"

# With custom config
yantrik ask --config /opt/yantrik/config.yaml "Check system health"
```

## Privacy & Security

- **Local-first**: All AI inference runs on your machine. No telemetry, no cloud calls (unless you choose a cloud LLM backend).
- **Memory is yours**: The companion's memory database lives at `/opt/yantrik/data/` — plain SQLite files you can inspect, export, or delete.
- **Permission system**: Tools are categorized as Safe → Standard → Sensitive → Dangerous. Sensitive and dangerous operations require explicit user approval.
- **Path sandboxing**: File tools block access to `.ssh`, `.gnupg`, and other sensitive directories.
- **No tracking**: Zero analytics, zero telemetry, zero phone-home.

## License

Proprietary. See LICENSE for details.

## Links

- **Releases**: https://releases.yantrikos.com
- **Issues**: https://github.com/spranab/yantrik-os/issues
