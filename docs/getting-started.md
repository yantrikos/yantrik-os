# Getting Started with Yantrik OS

This guide walks you through installing Yantrik OS, configuring your companion, and getting productive.

## Prerequisites

- **Alpine Linux 3.18+** installed on your target machine (bare metal, VirtualBox, QEMU/KVM, or Proxmox LXC)
- **Root access** (the installer needs to install packages and create system users)
- **Internet connection** (for downloading packages, binaries, and AI models)
- **4 GB RAM minimum** (8+ GB recommended)
- **4 GB free disk space** (includes offline AI model; 10+ GB recommended if hosting additional models)

If you don't have Alpine Linux yet, download it from [alpinelinux.org](https://alpinelinux.org/downloads/) and run `setup-alpine` to do a basic installation.

## Installation

### Option 1: One-Line Install

```bash
curl -fsSL https://get.yantrikos.com/install.sh | sh
```

### Option 2: Download and Run

```bash
wget https://releases.yantrikos.com/stable/install.sh
chmod +x install.sh
./install.sh
```

### Option 3: From Local Binaries

If you've built Yantrik from source or have binaries on hand, copy them to `/tmp/` before running the installer:

```bash
cp yantrik-ui /tmp/
cp yantrik /tmp/
./install.sh
```

The installer will detect and use local binaries automatically.

## What the Installer Does

The installer is fully automated and walks you through each step:

### Step 1: Hardware Detection

Automatically detects your CPU, RAM, GPU, disk space, and whether you're running in a hypervisor (VirtualBox, QEMU, or bare metal). This info is used to configure the display backend and compositor settings.

### Step 2: User Configuration

You'll be asked for:

- **Your name** — the companion uses this to address you
- **Companion name** — what you want to call your AI companion (default: "Yantrik")
- **LLM backend** — choose between:
  - **Ollama API** — Point to a local or remote Ollama server with GPU (recommended)
  - **Claude CLI** — Cloud quality, requires Claude Code CLI installed
  - **OpenAI API** — Any OpenAI-compatible endpoint (OpenRouter, Together, etc.)
  - **Offline only** — Use the bundled Qwen 3.5 4B model (GPU strongly recommended for usable speed)

### Step 3: System Dependencies

Installs required packages via `apk`:
- **labwc** — Wayland compositor (lightweight, wlroots-based)
- **foot** — Terminal emulator
- **mako** — Notification daemon
- **mesa** — OpenGL drivers
- **fonts, audio, networking tools**, and more

For VirtualBox guests, it also installs Guest Additions automatically.

### Step 4: User & Directory Setup

Creates the `yantrik` system user and the directory structure:

```
/opt/yantrik/
├── bin/              # yantrik-ui, yantrik binaries
├── data/             # Memory database (memory.db)
├── models/
│   ├── embedder/     # MiniLM sentence embeddings (~87MB)
│   ├── llm/          # Fallback LLM model (optional)
│   └── whisper/      # Voice input model (~146MB)
├── logs/             # yantrik-os.log
├── skills/           # YAML plugin directory
├── i18n/             # Translation files
└── config.yaml       # Main configuration
```

### Step 5: Binary Download

Binaries are fetched from (in order of priority):
1. `/tmp/` (local copies — fastest)
2. `releases.yantrikos.com` (official release server)
3. GitHub Releases

### Step 6: AI Models

Downloads three models automatically:
- **MiniLM embedder** (~87MB) — sentence embeddings for memory search. Always needed.
- **Whisper tiny** (~146MB) — speech-to-text for voice input. Optional but recommended.
- **Qwen 3.5 4B Q4_K_M** (~2.5GB) — offline LLM for the companion. Provides reasoning, structured output (CSV, JSON, slides), and instruction following. **Note:** This model requires GPU acceleration (via Ollama) for interactive response times. On CPU-only it serves as an emergency fallback but responses will be very slow.

If you're on a LAN with a model cache server, downloads are instant.

### Step 7: Configuration

Generates `/opt/yantrik/config.yaml` based on your choices. You can edit this file anytime — see the [Configuration](#configuration) section below.

### Step 8: Desktop Environment

Configures the labwc Wayland compositor:
- Hypervisor-aware display settings (software rendering for VirtualBox, hardware for bare metal)
- Keyboard shortcuts (Alt+Tab, Win+T for terminal, Print Screen for screenshots)
- Auto-maximize Yantrik OS window
- Foot terminal with dark color scheme

### Step 9: Auto-Login

Sets up automatic login as the `yantrik` user — the system boots directly into the Yantrik OS desktop. No login screen, no display manager overhead.

### Step 10: Upgrade Script

Installs `yantrik-upgrade` at `/usr/local/bin/` for future updates.

## First Boot

After installation, reboot:

```bash
reboot
```

You'll see:

1. **Boot animation** — Yantrik OS splash screen
2. **Onboarding** — First-time setup wizard (if not already completed during install)
3. **Desktop** — The main desktop with your companion, app dock, and system tray

### The Desktop

The desktop has:
- **Companion chat** — The Intent Lens at the bottom where you talk to your companion
- **App dock** — Quick-launch bar for all built-in apps
- **System tray** — Battery, WiFi, time, notifications
- **Whisper cards** — Proactive notifications from the companion (right side)

### Talking to Your Companion

Type in the Intent Lens or use voice input to:
- Ask questions: *"What's using the most CPU?"*
- Give commands: *"Open my spreadsheet"*
- Request analysis: *"Summarize my recent emails"*
- Get help: *"How do I connect to WiFi?"*

The companion has access to 116+ tools including file management, git, email, browser automation, and system commands.

## Configuration

The main config file is at `/opt/yantrik/config.yaml`. Key sections:

### LLM Backend

```yaml
llm:
  backend: "api"                    # "api", "claude-cli", or "llamacpp"
  api_url: "http://192.168.1.100:11434"  # Ollama server address
  api_model: "qwen3:8b"            # Model name
  max_tokens: 512
  temperature: 0.7
```

To switch backends, edit the `backend` field and restart. The system also has an automatic fallback — if your primary LLM is unreachable, the built-in offline model (Qwen 3.5 4B) activates automatically via the llama-server running on the device.

### Instincts (Proactive Behavior)

```yaml
instincts:
  check_in_enabled: true          # Morning/evening check-ins
  check_in_hours: 8.0             # Hours between check-ins
  emotional_awareness_enabled: true
  follow_up_enabled: true         # Follow up on open conversations
  reminder_enabled: true          # Commitment/deadline tracking
  pattern_surfacing_enabled: true # Surface observed patterns
  conflict_alerting_enabled: true # Alert on contradictions
```

### Tools & Permissions

```yaml
tools:
  enabled: true
  max_tool_rounds: 3              # Max tool calls per conversation turn
  max_permission: "sensitive"     # Cap: safe, standard, sensitive, dangerous
```

Permission levels control what the companion can do:
- **Safe** — Read-only operations (list files, check system status)
- **Standard** — Reversible writes (create files, save notes)
- **Sensitive** — System changes (install packages, modify config)
- **Dangerous** — Destructive operations (delete files, send emails) — requires explicit approval

### Updates

```yaml
updates:
  channel: "beta"                 # "stable", "beta", or "nightly"
  server: "http://releases.yantrikos.com"
  check_on_boot: true
```

## Updating Yantrik OS

### Automatic Updates

If `check_on_boot: true` is set, Yantrik checks for updates on every startup and shows a notification if one is available.

### Manual Updates

```bash
# Check what's available
yantrik-upgrade check

# Update to latest in your channel
yantrik-upgrade stable    # or beta, or nightly

# Force re-download
yantrik-upgrade stable --force
```

Updates are verified with SHA256 checksums. If a new version fails to start within 30 seconds, the system automatically rolls back to the previous version.

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| Alt + Tab | Switch windows |
| Alt + F4 | Close window |
| Win + T | Open terminal |
| Win + Left/Right | Snap window to edge |
| Win + Up | Maximize window |
| Print Screen | Screenshot (saved to ~/Pictures/Screenshots/) |

## Troubleshooting

### Black screen after boot

The Wayland compositor may need different settings for your GPU:

```bash
# SSH into the machine, then edit:
vi /home/yantrik/.config/labwc/environment

# Try changing SLINT_BACKEND:
# SLINT_BACKEND=winit          (default, hardware accelerated)
# SLINT_BACKEND=winit-software (software rendering, always works)
```

### Companion not responding

Check the LLM backend is reachable:

```bash
# For Ollama:
curl http://your-ollama-host:11434/api/tags

# Check logs:
tail -f /opt/yantrik/logs/yantrik-os.log
```

### No sound

```bash
# Check ALSA
alsamixer     # Unmute channels with 'm', adjust volume
speaker-test  # Test audio output
```

### Slow AI responses

CPU-only inference is extremely slow (~1 TPS) and not practical for interactive use. To get usable response times:
- **Point to an Ollama server with GPU acceleration** — even a modest GPU gives 50+ TPS
- **Use a remote Ollama on your LAN** — run Ollama on a machine with a GPU and point Yantrik's config to it
- **Use a cloud backend** (Claude CLI or OpenAI API) — works over the internet

### Checking logs

```bash
# Live log
tail -f /opt/yantrik/logs/yantrik-os.log

# Last 100 lines
tail -100 /opt/yantrik/logs/yantrik-os.log

# Search for errors
grep -i error /opt/yantrik/logs/yantrik-os.log
```

## Next Steps

- **Customize your theme** — see `~/.config/yantrik/theme-override.yaml`
- **Add custom tools** — create YAML plugins in `~/.config/yantrik/plugins/`
- **Connect email** — configure IMAP in Settings → Email
- **Explore apps** — try ySheets, yPresent, yDocs from the app dock
- **Read the architecture** — see [architecture.md](architecture.md) for the system design
