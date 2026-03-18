#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# Yantrik OS — Universal Installer (Beta)
# ═══════════════════════════════════════════════════════════════
#
# Installs Yantrik OS on an existing Alpine Linux system.
# Works on: bare-metal, VirtualBox, QEMU/KVM, Proxmox.
#
# Usage:
#   curl -fsSL https://get.yantrik.dev/install.sh | sh
#   # or
#   wget -qO- https://get.yantrik.dev/install.sh | sh
#   # or
#   chmod +x install.sh && ./install.sh
#
# Prerequisites:
#   - Alpine Linux 3.18+ installed (setup-alpine completed)
#   - Root access
#   - Internet connection
#
# What it does:
#   1. Detects hardware (GPU, CPU, RAM, hypervisor)
#   2. Installs system dependencies (labwc, mesa, fonts, etc.)
#   3. Downloads Yantrik binaries from GitHub Releases
#   4. Downloads AI models (embedder, whisper)
#   5. Creates yantrik user and directory structure
#   6. Generates config.yaml
#   7. Configures labwc compositor + auto-login
#   8. Sets up yantrik-upgrade for future updates
#
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

# ── Constants ──
YANTRIK_VERSION="0.3.0"
GITHUB_REPO="spranab/yantrik-os"
GITHUB_RELEASES="https://github.com/$GITHUB_REPO/releases"
YANTRIK_HOME="/opt/yantrik"
YANTRIK_USER="yantrik"
BIN_DIR="$YANTRIK_HOME/bin"
DATA_DIR="$YANTRIK_HOME/data"
MODEL_DIR="$YANTRIK_HOME/models"
LOG_DIR="$YANTRIK_HOME/logs"

# ── Colors ──
CYAN='\033[0;36m'
AMBER='\033[0;33m'
GREEN='\033[0;32m'
RED='\033[0;31m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m'

step()  { echo -e "${CYAN}::${NC} ${BOLD}$1${NC}"; }
info()  { echo -e "   ${DIM}$1${NC}"; }
ok()    { echo -e "   ${GREEN}✓${NC} $1"; }
warn()  { echo -e "   ${AMBER}!${NC} $1"; }
fail()  { echo -e "   ${RED}✗${NC} $1"; exit 1; }

# ── Banner ──
echo
echo -e "${CYAN}╔═══════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║${NC}       ${BOLD}Yantrik OS${NC} — Installer v${YANTRIK_VERSION}      ${CYAN}║${NC}"
echo -e "${CYAN}║${NC}       ${DIM}AI-native desktop shell${NC}              ${CYAN}║${NC}"
echo -e "${CYAN}╚═══════════════════════════════════════════╝${NC}"
echo

# ── Pre-checks ──
if [ "$(id -u)" -ne 0 ]; then
    fail "This script must be run as root. Try: sudo ./install.sh"
fi

if [ ! -f /etc/alpine-release ]; then
    warn "This doesn't appear to be Alpine Linux."
    echo -n "   Continue anyway? [y/N] "
    read -r ans
    [ "$ans" = "y" ] || [ "$ans" = "Y" ] || exit 1
fi

ALPINE_VERSION=$(cat /etc/alpine-release 2>/dev/null || echo "unknown")
info "Alpine Linux $ALPINE_VERSION"

# ═══════════════════════════════════════════════════════════════
# STEP 1: Hardware Detection
# ═══════════════════════════════════════════════════════════════
step "Detecting hardware..."

# CPU
CPU_MODEL=$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2 | sed 's/^ //' || echo "unknown")
CPU_CORES=$(nproc 2>/dev/null || echo "?")
info "CPU: $CPU_MODEL ($CPU_CORES cores)"

# RAM
RAM_KB=$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo 0)
RAM_GB=$(echo "scale=1; $RAM_KB / 1024 / 1024" | bc 2>/dev/null || echo "?")
info "RAM: ${RAM_GB} GB"

# Hypervisor detection
HYPERVISOR="bare"
if [ -f /sys/class/dmi/id/product_name ]; then
    PRODUCT=$(cat /sys/class/dmi/id/product_name 2>/dev/null || true)
    case "$PRODUCT" in
        *VirtualBox*) HYPERVISOR="vbox" ;;
        *QEMU*|*KVM*|*Standard*PC*) HYPERVISOR="qemu" ;;
    esac
fi
if [ "$HYPERVISOR" = "bare" ] && command -v lspci >/dev/null 2>&1; then
    if lspci 2>/dev/null | grep -qi virtualbox; then
        HYPERVISOR="vbox"
    elif lspci 2>/dev/null | grep -qi "virtio\|qemu\|red hat"; then
        HYPERVISOR="qemu"
    fi
fi
info "Platform: $HYPERVISOR"

# GPU detection
GPU_INFO="none"
HAS_NVIDIA=false
HAS_AMD=false
HAS_INTEL_GPU=false
GPU_VARIANT=""
if command -v lspci >/dev/null 2>&1; then
    GPU_LINE=$(lspci 2>/dev/null | grep -i 'vga\|3d\|display' | head -1 || true)
    if [ -n "$GPU_LINE" ]; then
        GPU_INFO=$(echo "$GPU_LINE" | sed 's/.*: //')
        if echo "$GPU_LINE" | grep -qi nvidia; then
            HAS_NVIDIA=true
            GPU_VARIANT="cuda"
        elif echo "$GPU_LINE" | grep -qi 'amd\|radeon'; then
            HAS_AMD=true
            GPU_VARIANT="rocm"
        elif echo "$GPU_LINE" | grep -qi 'intel.*arc\|intel.*a[0-9]\{3\}'; then
            HAS_INTEL_GPU=true
            GPU_VARIANT="vulkan"
        fi
    fi
fi
info "GPU: $GPU_INFO"
if [ -n "$GPU_VARIANT" ]; then
    info "GPU acceleration: $GPU_VARIANT"
fi

# Disk
DISK_TOTAL=$(df -h / 2>/dev/null | awk 'NR==2{print $2}' || echo "?")
DISK_FREE=$(df -h / 2>/dev/null | awk 'NR==2{print $4}' || echo "?")
info "Disk: ${DISK_FREE} free of ${DISK_TOTAL}"

echo

# ═══════════════════════════════════════════════════════════════
# STEP 2: User Configuration
# ═══════════════════════════════════════════════════════════════
step "Configuration..."

# User name
DEFAULT_USER="User"
echo -n "   Your name (for companion to use) [$DEFAULT_USER]: "
read -r USER_NAME
USER_NAME=${USER_NAME:-$DEFAULT_USER}

# Companion name
DEFAULT_COMPANION="Yantrik"
echo -n "   Companion name [$DEFAULT_COMPANION]: "
read -r COMPANION_NAME
COMPANION_NAME=${COMPANION_NAME:-$DEFAULT_COMPANION}

# LLM backend selection
echo
echo -e "   ${BOLD}LLM Backend:${NC}"
echo -e "   ${AMBER}   GPU is strongly recommended for interactive AI responses.${NC}"
echo -e "   ${AMBER}   CPU-only inference is very slow (~1 TPS). GPU gives 50-130+ TPS.${NC}"
echo
echo -e "   ${DIM}1)${NC} Ollama API  ${DIM}— Local/remote Ollama with GPU (recommended)${NC}"
echo -e "   ${DIM}2)${NC} Claude CLI  ${DIM}— Cloud quality, requires Claude Code installed${NC}"
echo -e "   ${DIM}3)${NC} OpenAI API  ${DIM}— Any OpenAI-compatible API${NC}"
echo -e "   ${DIM}4)${NC} Offline only ${DIM}— Use bundled 4B model (requires GPU for usable speed)${NC}"
echo -n "   Choice [1]: "
read -r LLM_CHOICE
LLM_CHOICE=${LLM_CHOICE:-1}

LLM_BACKEND="api"
LLM_MODEL="qwen3.5:4b"
API_URL="http://localhost:11434"
API_KEY=""

case "$LLM_CHOICE" in
    1)
        LLM_BACKEND="api"
        echo -n "   Ollama URL [http://localhost:11434]: "
        read -r API_URL
        API_URL=${API_URL:-http://localhost:11434}
        echo -n "   Model name [qwen3.5:4b]: "
        read -r LLM_MODEL
        LLM_MODEL=${LLM_MODEL:-qwen3.5:4b}
        ;;
    2)
        LLM_BACKEND="claude-cli"
        LLM_MODEL="sonnet"
        ;;
    3)
        LLM_BACKEND="api"
        echo -n "   API URL [https://api.openai.com]: "
        read -r API_URL
        API_URL=${API_URL:-https://api.openai.com}
        echo -n "   API Key: "
        read -rs API_KEY
        echo
        echo -n "   Model name [gpt-4o]: "
        read -r LLM_MODEL
        LLM_MODEL=${LLM_MODEL:-gpt-4o}
        ;;
    4)
        LLM_BACKEND="api"
        API_URL="http://localhost:8341"
        LLM_MODEL="qwen3.5-4b"
        info "Using bundled offline model via llama-server on port 8341"
        ;;
esac

ok "Config: $USER_NAME + $COMPANION_NAME, LLM: $LLM_BACKEND/$LLM_MODEL"
echo

# ═══════════════════════════════════════════════════════════════
# STEP 3: System Dependencies
# ═══════════════════════════════════════════════════════════════
step "Installing system packages..."

# Enable community repo
if grep -q "^#.*community" /etc/apk/repositories; then
    sed -i 's|^#\(.*community\)|\1|' /etc/apk/repositories
    ok "Community repo enabled"
fi

apk update -q

# Core packages (required)
apk add -q \
    labwc foot mako \
    dbus dbus-openrc \
    eudev eudev-openrc \
    mesa-dri-gallium \
    font-dejavu \
    seatd seatd-openrc \
    wlr-randr \
    alsa-utils alsa-lib \
    curl wget \
    openrc gcompat \
    libinput \
    wayland-libs-egl wayland-libs-client wayland-libs-server wayland-libs-cursor \
    ca-certificates \
    gcc musl-dev \
    grim slurp wl-clipboard jq bc diffutils \
    sudo pciutils \
    speech-dispatcher
ok "Core packages installed"

# Optional packages (non-fatal)
for pkg in thunar firefox-esr mpv chromium nmcli docker; do
    apk add -q "$pkg" 2>/dev/null && ok "Optional: $pkg" || true
done

# VirtualBox Guest Additions
if [ "$HYPERVISOR" = "vbox" ]; then
    apk add -q virtualbox-guest-additions virtualbox-guest-additions-openrc 2>/dev/null && {
        rc-update add virtualbox-guest-additions default 2>/dev/null || true
        modprobe vboxguest 2>/dev/null || true
        ok "VirtualBox Guest Additions installed"
    } || warn "VBox Guest Additions not available"
fi

# Build gcompat shim
cat > /tmp/gcompat_shim.c <<'SHIM'
#include <fcntl.h>
#include <stdarg.h>
int fcntl64(int fd, int cmd, ...) {
    va_list ap; va_start(ap, cmd);
    void *arg = va_arg(ap, void *); va_end(ap);
    return fcntl(fd, cmd, arg);
}
int __res_init(void) { return 0; }
const char *gnu_get_libc_version(void) { return "2.38"; }
SHIM
gcc -shared -o /usr/lib/libgcompat_shim.so /tmp/gcompat_shim.c
rm /tmp/gcompat_shim.c
ok "glibc compatibility shim built"

# Fix DRI paths
if [ -d /usr/lib/xorg/modules/dri ] && [ ! -d /usr/lib/dri ]; then
    mkdir -p /usr/lib/dri
    cp /usr/lib/xorg/modules/dri/*.so /usr/lib/dri/ 2>/dev/null || true
fi

# Build wlrctl if missing
if ! command -v wlrctl >/dev/null 2>&1; then
    info "Building wlrctl..."
    apk add -q build-base wayland-dev wayland-protocols meson ninja scdoc 2>/dev/null || true
    cd /tmp
    if [ ! -d wlrctl ]; then
        wget -q -O wlrctl.tar.gz "https://git.sr.ht/~brocellous/wlrctl/archive/main.tar.gz" 2>/dev/null || true
        if [ -f wlrctl.tar.gz ]; then
            mkdir -p wlrctl && tar xzf wlrctl.tar.gz -C wlrctl --strip-components=1
            cd wlrctl
            meson setup build --prefix=/usr/local 2>/dev/null || true
            ninja -C build 2>/dev/null && ninja -C build install 2>/dev/null && ok "wlrctl built" || warn "wlrctl build failed (non-fatal)"
            cd /tmp && rm -rf wlrctl wlrctl.tar.gz
        fi
    fi
fi

echo

# ═══════════════════════════════════════════════════════════════
# STEP 4: Create User & Directories
# ═══════════════════════════════════════════════════════════════
step "Setting up user and directories..."

id "$YANTRIK_USER" &>/dev/null || adduser -D -h "/home/$YANTRIK_USER" "$YANTRIK_USER" 2>/dev/null || true
for grp in seat video audio input; do
    addgroup "$YANTRIK_USER" "$grp" 2>/dev/null || true
done

mkdir -p "$BIN_DIR" "$DATA_DIR" "$MODEL_DIR" "$LOG_DIR" \
    "$MODEL_DIR/llm" "$MODEL_DIR/embedder" "$MODEL_DIR/whisper" \
    "$YANTRIK_HOME/skills" "$YANTRIK_HOME/i18n"

# Sudoers for power commands
cat > /etc/sudoers.d/yantrik-power <<SUDOERS
$YANTRIK_USER ALL=(ALL) NOPASSWD: /sbin/poweroff, /sbin/reboot, /usr/sbin/zzz
SUDOERS
chmod 440 /etc/sudoers.d/yantrik-power

ok "User $YANTRIK_USER created with seat/video/audio groups"
echo

# ═══════════════════════════════════════════════════════════════
# STEP 5: Download Yantrik Binaries
# ═══════════════════════════════════════════════════════════════
step "Downloading Yantrik binaries..."

# Try: /tmp local → releases.yantrikos.com/beta → GitHub Releases
RELEASES_URL="http://releases.yantrikos.com/beta"
GITHUB_URL="$GITHUB_RELEASES/download/v${YANTRIK_VERSION}"
BINARY_INSTALLED=false

# If GPU detected, prefer the matching GPU-accelerated binary
UI_BINARY="yantrik-ui"
if [ -n "$GPU_VARIANT" ]; then
    GPU_BIN="yantrik-ui-$GPU_VARIANT"
    info "$GPU_INFO detected — trying $GPU_VARIANT-accelerated binary..."
    if [ -f "/tmp/$GPU_BIN" ]; then
        cp "/tmp/$GPU_BIN" "$BIN_DIR/yantrik-ui"
        chmod +x "$BIN_DIR/yantrik-ui"
        ok "$GPU_BIN (from /tmp, $GPU_VARIANT GPU-accelerated)"
        BINARY_INSTALLED=true
        UI_BINARY="done"
    elif curl -fsSL --connect-timeout 10 "$RELEASES_URL/$GPU_BIN" -o "$BIN_DIR/yantrik-ui" 2>/dev/null; then
        chmod +x "$BIN_DIR/yantrik-ui"
        ok "$GPU_BIN (from releases.yantrikos.com, $GPU_VARIANT GPU-accelerated)"
        BINARY_INSTALLED=true
        UI_BINARY="done"
    else
        info "GPU variant not available — falling back to CPU binary"
    fi
fi

for bin in $( [ "$UI_BINARY" = "done" ] && echo "yantrik" || echo "yantrik-ui yantrik" ); do
    if [ -f "/tmp/$bin" ]; then
        cp "/tmp/$bin" "$BIN_DIR/$bin"
        chmod +x "$BIN_DIR/$bin"
        ok "$bin (from /tmp)"
        BINARY_INSTALLED=true
    elif curl -fsSL --connect-timeout 10 "$RELEASES_URL/$bin" -o "$BIN_DIR/$bin" 2>/dev/null; then
        chmod +x "$BIN_DIR/$bin"
        ok "$bin (from releases.yantrikos.com)"
        BINARY_INSTALLED=true
    elif curl -fsSL --connect-timeout 10 "$GITHUB_URL/$bin" -o "$BIN_DIR/$bin" 2>/dev/null; then
        chmod +x "$BIN_DIR/$bin"
        ok "$bin (from GitHub Releases)"
        BINARY_INSTALLED=true
    else
        warn "$bin not found — upload to /tmp/$bin and re-run"
        touch "$BIN_DIR/$bin"
    fi
done

echo

# ═══════════════════════════════════════════════════════════════
# STEP 6: Download AI Models
# ═══════════════════════════════════════════════════════════════
step "Downloading AI models..."

# LAN model cache (optional — speeds up installs on local network)
MODEL_CACHE="http://192.168.4.92:8888"
CACHE_OK=false
if curl -sf --connect-timeout 2 "$MODEL_CACHE/" >/dev/null 2>&1; then
    CACHE_OK=true
    info "LAN model cache available"
fi

download_model() {
    local name="$1" dir="$2" hf_base="$3"
    shift 3
    local files=("$@")

    if [ -f "$dir/model.safetensors" ] || [ -f "$dir/$(ls "$dir"/*.gguf 2>/dev/null | head -1)" ]; then
        ok "$name already present"
        return
    fi

    info "Downloading $name..."
    mkdir -p "$dir"
    for f in "${files[@]}"; do
        if $CACHE_OK && curl -sf --connect-timeout 2 "$MODEL_CACHE/$name/$f" -o "$dir/$f" 2>/dev/null; then
            true
        else
            wget -q -O "$dir/$f" "$hf_base/$f" 2>/dev/null || warn "Failed to download $f"
        fi
    done
    ok "$name downloaded"
}

# MiniLM embedder (~87MB) — always needed
download_model "embedder" "$MODEL_DIR/embedder" \
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main" \
    config.json tokenizer.json tokenizer_config.json special_tokens_map.json model.safetensors

# Whisper tiny (~146MB) — for voice input
download_model "whisper" "$MODEL_DIR/whisper" \
    "https://huggingface.co/openai/whisper-tiny/resolve/main" \
    config.json tokenizer.json model.safetensors

# Offline LLM — Qwen 3.5 4B Q4_K_M (~2.5GB)
# The 4B model is the minimum viable size for reliable reasoning,
# structured output (CSV, JSON, slides), and instruction following.
# Thinking is disabled (think: false) at the API layer to maximize output quality.
LLM_GGUF="Qwen3.5-4B-Q4_K_M.gguf"
LLM_GGUF_URL="https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/$LLM_GGUF"
if [ -f "$MODEL_DIR/llm/$LLM_GGUF" ]; then
    ok "Offline LLM already present ($LLM_GGUF)"
else
    info "Downloading offline LLM ($LLM_GGUF, ~2.5 GB)..."
    if [ "$CACHE_OK" = true ]; then
        curl -fsSL --connect-timeout 5 "$MODEL_CACHE/$LLM_GGUF" -o "$MODEL_DIR/llm/$LLM_GGUF" 2>/dev/null && \
            ok "Offline LLM (from LAN cache)" || true
    fi
    if [ ! -f "$MODEL_DIR/llm/$LLM_GGUF" ]; then
        curl -fSL --progress-bar "$LLM_GGUF_URL" -o "$MODEL_DIR/llm/$LLM_GGUF" 2>&1 && \
            ok "Offline LLM downloaded ($LLM_GGUF)" || \
            warn "Offline LLM download failed — companion will need a remote LLM backend"
    fi
fi

echo

# ═══════════════════════════════════════════════════════════════
# STEP 7: Generate Configuration
# ═══════════════════════════════════════════════════════════════
step "Generating configuration..."

# Build the LLM config section based on backend choice
LLM_CONFIG=""
case "$LLM_BACKEND" in
    claude-cli)
        LLM_CONFIG="backend: \"claude-cli\"
  api_model: \"$LLM_MODEL\""
        ;;
    api)
        LLM_CONFIG="backend: \"api\"
  api_base_url: \"${API_URL}/v1\"
  api_model: \"$LLM_MODEL\""
        if [ -n "$API_KEY" ]; then
            LLM_CONFIG="$LLM_CONFIG
  api_key: \"$API_KEY\""
        fi
        ;;
esac

cat > "$YANTRIK_HOME/config.yaml" <<CONFIG
# Yantrik OS Configuration
# Generated by install.sh on $(date +%Y-%m-%d)

user_name: "$USER_NAME"

personality:
  name: "$COMPANION_NAME"
  system_prompt: >
    You are $COMPANION_NAME, a personal AI companion running as the desktop shell.
    You remember everything the user tells you. You are warm, thoughtful,
    and occasionally curious. You are aware of the system state — battery,
    network, running apps, files. When you notice patterns or have concerns,
    you bring them up naturally. You never fabricate memories —
    if you don't know, say so.

llm:
  $LLM_CONFIG
  max_tokens: 1024
  temperature: 0.6
  max_context_tokens: 16384
  fallback:
    backend: "api"
    api_base_url: "http://localhost:8341/v1"
    api_model: "qwen3.5-4b"

server:
  host: "0.0.0.0"
  port: 8340

yantrikdb:
  db_path: "$DATA_DIR/memory.db"
  embedding_dim: 384
  embedder_model_dir: "$MODEL_DIR/embedder"

conversation:
  max_history_turns: 10
  session_timeout_minutes: 30

tools:
  enabled: true
  max_tool_rounds: 3
  max_permission: "sensitive"

cognition:
  think_interval_minutes: 15
  think_interval_active_minutes: 5
  idle_think_interval_minutes: 30
  proactive_urgency_threshold: 0.7

instincts:
  check_in_enabled: true
  check_in_hours: 8.0
  emotional_awareness_enabled: true
  follow_up_enabled: true
  follow_up_min_hours: 4.0
  reminder_enabled: true
  pattern_surfacing_enabled: true
  conflict_alerting_enabled: true
  conflict_alert_threshold: 5

urges:
  expiry_hours: 48.0
  max_pending: 20
  boost_increment: 0.1
  cooldown_seconds: 3600.0

bond:
  enabled: true

voice:
  enabled: true
  whisper_model: "openai/whisper-tiny"
  piper_voice: "en_US-lessac-medium"
  silence_threshold: 0.01
  silence_duration_ms: 800

updates:
  channel: "beta"
  server: "http://releases.yantrikos.com"
  check_on_boot: true
CONFIG

ok "config.yaml generated"
echo

# ═══════════════════════════════════════════════════════════════
# STEP 8: Configure labwc Compositor
# ═══════════════════════════════════════════════════════════════
step "Configuring desktop environment..."

LABWC_DIR="/home/$YANTRIK_USER/.config/labwc"
mkdir -p "$LABWC_DIR"

# Environment — hypervisor-aware
{
    echo "# Yantrik OS — labwc environment"
    echo "LD_PRELOAD=/lib/libgcompat.so.0 /usr/lib/libgcompat_shim.so"
    echo "WLR_RENDERER_ALLOW_SOFTWARE=1"
    echo "WLR_LIBINPUT_NO_DEVICES=1"
    echo ""
    case "$HYPERVISOR" in
        qemu)
            echo "WLR_DRM_DEVICES=/dev/dri/card0"
            echo "SLINT_BACKEND=winit"
            ;;
        vbox)
            echo "WLR_NO_HARDWARE_CURSORS=1"
            echo "SLINT_BACKEND=winit-software"
            ;;
        *)
            echo "SLINT_BACKEND=winit"
            ;;
    esac
} > "$LABWC_DIR/environment"

# Autostart
cat > "$LABWC_DIR/autostart" <<'AUTOSTART'
#!/bin/sh
foot --server &
mako &
/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
AUTOSTART
chmod +x "$LABWC_DIR/autostart"

# rc.xml (key bindings, window rules)
cat > "$LABWC_DIR/rc.xml" <<'RCXML'
<?xml version="1.0" encoding="UTF-8"?>
<labwc_config>
  <core><gap>4</gap></core>
  <theme><titlebar><font name="DejaVu Sans" size="10" /></titlebar></theme>
  <keyboard>
    <keybind key="A-Tab"><action name="NextWindow" /></keybind>
    <keybind key="A-F4"><action name="Close" /></keybind>
    <keybind key="W-t"><action name="Execute"><command>foot</command></action></keybind>
    <keybind key="W-Left"><action name="SnapToEdge" direction="left" /></keybind>
    <keybind key="W-Right"><action name="SnapToEdge" direction="right" /></keybind>
    <keybind key="W-Up"><action name="Maximize" /></keybind>
    <keybind key="W-Down"><action name="UnMaximize" /></keybind>
    <keybind key="Print">
      <action name="Execute">
        <command>sh -c 'mkdir -p ~/Pictures/Screenshots &amp;&amp; grim ~/Pictures/Screenshots/$(date +%Y%m%d_%H%M%S).png'</command>
      </action>
    </keybind>
  </keyboard>
  <windowRules>
    <windowRule identifier="yantrik-ui"><action name="Maximize" /></windowRule>
  </windowRules>
</labwc_config>
RCXML

# Foot terminal config
FOOT_DIR="/home/$YANTRIK_USER/.config/foot"
mkdir -p "$FOOT_DIR"
cat > "$FOOT_DIR/foot.ini" <<'FOOTINI'
[main]
font=DejaVu Sans Mono:size=11
pad=8x4
shell=/bin/ash
dpi-aware=no

[scrollback]
lines=10000

[cursor]
style=beam
blink=yes

[colors]
background=0c0b10
foreground=c8c8d0
regular0=1a1a2e
regular1=e86b6b
regular2=5ac8a0
regular3=d4a04a
regular4=5ac8d4
regular5=a87bd4
regular6=5ac8d4
regular7=c8c8d0
bright0=2e2e48
bright1=f09090
bright2=7ee0c0
bright3=e0c070
bright4=80d8e8
bright5=c0a0e0
bright6=80d8e8
bright7=e0e0e8
FOOTINI

# Mako notification config
MAKO_DIR="/home/$YANTRIK_USER/.config/mako"
mkdir -p "$MAKO_DIR"
cat > "$MAKO_DIR/config" <<'MAKOCONF'
font=DejaVu Sans 11
background-color=#0c0b10e6
text-color=#c8c8d0
border-color=#5ac8d460
border-size=1
border-radius=8
padding=12
margin=12
width=360
default-timeout=8000
max-visible=3
layer=overlay
anchor=top-right

[urgency=critical]
background-color=#1a0a0ae6
border-color=#e86b6b80
default-timeout=0
MAKOCONF

chown -R "$YANTRIK_USER:$YANTRIK_USER" "/home/$YANTRIK_USER/.config"
ok "labwc + foot + mako configured"
echo

# ═══════════════════════════════════════════════════════════════
# STEP 9: System Services & Auto-Login
# ═══════════════════════════════════════════════════════════════
step "Configuring system services..."

# Enable services
rc-update add dbus default 2>/dev/null || true
rc-update add seatd default 2>/dev/null || true
rc-update add udev sysinit 2>/dev/null || true
rc-service seatd start 2>/dev/null || true
rc-service dbus start 2>/dev/null || true

# Login wrapper
cat > "$BIN_DIR/yantrik-login" <<'LOGIN'
#!/bin/sh
if [ "$(whoami)" = "root" ]; then
    exec su -l yantrik -c "exec /opt/yantrik/bin/yantrik-start"
else
    exec /opt/yantrik/bin/yantrik-start
fi
LOGIN
chmod +x "$BIN_DIR/yantrik-login"

# Startup script
cat > "$BIN_DIR/yantrik-start" <<'START'
#!/bin/sh
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
mkdir -p "$XDG_RUNTIME_DIR"
LOG=/opt/yantrik/logs/yantrik-os.log
echo "$(date): Starting Yantrik OS desktop..." >> "$LOG"
exec labwc >> "$LOG" 2>&1
START
chmod +x "$BIN_DIR/yantrik-start"

# Auto-login on tty1
if [ -f /etc/inittab ]; then
    sed -i 's|^tty1::.*|tty1::respawn:/sbin/getty -n -l /opt/yantrik/bin/yantrik-login 38400 tty1|' /etc/inittab 2>/dev/null || true
fi

# XDG runtime dir on boot
cat > /etc/local.d/xdg-runtime.start <<'XDG'
#!/bin/sh
YANTRIK_UID=$(id -u yantrik 2>/dev/null || echo 1000)
mkdir -p "/run/user/$YANTRIK_UID"
chown yantrik:yantrik "/run/user/$YANTRIK_UID"
chmod 700 "/run/user/$YANTRIK_UID"
XDG
chmod +x /etc/local.d/xdg-runtime.start
rc-update add local default 2>/dev/null || true

ok "Services configured (dbus, seatd, auto-login)"
echo

# ═══════════════════════════════════════════════════════════════
# STEP 10: Offline LLM Server (llama-server)
# ═══════════════════════════════════════════════════════════════
step "Setting up offline LLM server..."

LLM_GGUF_PATH="$MODEL_DIR/llm/$LLM_GGUF"
if [ -f "$LLM_GGUF_PATH" ]; then
    # Build llama-server from llama.cpp if not already present
    if ! command -v llama-server >/dev/null 2>&1 && [ ! -f /usr/local/bin/llama-server ]; then
        info "Building llama-server from source (this takes a few minutes)..."
        apk add -q build-base cmake linux-headers 2>/dev/null || true
        cd /tmp
        if [ ! -d llama.cpp ]; then
            git clone --depth 1 https://github.com/ggerganov/llama.cpp.git 2>/dev/null || true
        fi
        if [ -d llama.cpp ]; then
            cd llama.cpp
            cmake -B build -DGGML_BLAS=OFF -DGGML_CUDA=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_SERVER=ON 2>/dev/null
            cmake --build build --target llama-server -j$(nproc) 2>/dev/null && {
                cp build/bin/llama-server /usr/local/bin/
                ok "llama-server built and installed"
            } || warn "llama-server build failed — offline fallback won't be available"
            cd /tmp && rm -rf llama.cpp
        fi
    else
        ok "llama-server already installed"
    fi

    # Create OpenRC service for llama-server
    cat > /etc/init.d/llama-server <<'LLAMA_SVC'
#!/sbin/openrc-run
name="llama-server"
description="Offline LLM server (Qwen 3.5 4B)"
command="/usr/local/bin/llama-server"
command_args="--model /opt/yantrik/models/llm/Qwen3.5-4B-Q4_K_M.gguf --host 127.0.0.1 --port 8341 --ctx-size 4096 --threads 2 --no-mmap"
command_background=true
pidfile="/run/llama-server.pid"
output_log="/opt/yantrik/logs/llama-server.log"
error_log="/opt/yantrik/logs/llama-server.log"

depend() {
    after localmount
}
LLAMA_SVC
    chmod +x /etc/init.d/llama-server
    rc-update add llama-server default 2>/dev/null || true
    ok "llama-server OpenRC service configured (port 8341)"
else
    info "No offline LLM model found — skipping llama-server setup"
fi

echo

# ═══════════════════════════════════════════════════════════════
# STEP 11: Install Upgrade Script
# ═══════════════════════════════════════════════════════════════
step "Installing upgrade mechanism..."

cat > "$BIN_DIR/yantrik-upgrade" <<'UPGRADE'
#!/bin/sh
# Yantrik OS — Self-Upgrade Script
# Downloads latest binaries from releases.yantrikos.com and swaps them.
#
# Usage:
#   yantrik-upgrade              Upgrade to latest on configured channel
#   yantrik-upgrade --check      Check for updates without installing
#   yantrik-upgrade --channel X  Override channel (nightly/beta/stable)

set -eu

CONFIG="/opt/yantrik/config.yaml"
BIN_DIR="/opt/yantrik/bin"
STAGING="/opt/yantrik/staging"
LOG="/opt/yantrik/logs/upgrade.log"

CYAN='\033[0;36m'
GREEN='\033[0;32m'
AMBER='\033[0;33m'
RED='\033[0;31m'
NC='\033[0m'

log() { echo "$(date '+%Y-%m-%d %H:%M:%S') $1" >> "$LOG"; echo -e "$1"; }

# Read channel from config or default to beta
CHANNEL="beta"
SERVER="http://releases.yantrikos.com"
if [ -f "$CONFIG" ]; then
    CFG_CHANNEL=$(grep 'channel:' "$CONFIG" 2>/dev/null | head -1 | sed 's/.*: *"\{0,1\}\([^"]*\)"\{0,1\}/\1/' | tr -d ' ')
    [ -n "$CFG_CHANNEL" ] && CHANNEL="$CFG_CHANNEL"
    CFG_SERVER=$(grep 'server:' "$CONFIG" 2>/dev/null | grep -v 'host:' | head -1 | sed 's/.*: *"\{0,1\}\([^"]*\)"\{0,1\}/\1/' | tr -d ' ')
    [ -n "$CFG_SERVER" ] && SERVER="$CFG_SERVER"
fi

# Parse args
CHECK_ONLY=false
while [ $# -gt 0 ]; do
    case "$1" in
        --check) CHECK_ONLY=true ;;
        --channel) CHANNEL="$2"; shift ;;
    esac
    shift
done

# Get current version
CURRENT=$(cat /opt/yantrik/.version 2>/dev/null || echo "unknown")

# Fetch manifest
MANIFEST=$(curl -fsSL --connect-timeout 10 "$SERVER/manifest.json" 2>/dev/null || echo "")
if [ -z "$MANIFEST" ]; then
    log "${RED}Failed to reach update server ($SERVER)${NC}"
    exit 1
fi

# Parse latest version for channel (simple grep — no jq dependency)
LATEST=$(echo "$MANIFEST" | grep -A3 "\"$CHANNEL\"" | grep '"version"' | sed 's/.*: *"\([^"]*\)".*/\1/' || echo "")
if [ -z "$LATEST" ]; then
    log "${RED}Channel '$CHANNEL' not found in manifest${NC}"
    exit 1
fi

log "${CYAN}Channel: $CHANNEL  Current: $CURRENT  Latest: $LATEST${NC}"

if [ "$CHECK_ONLY" = true ]; then
    if [ "$CURRENT" = "$LATEST" ]; then
        log "${GREEN}Already up to date ($CURRENT)${NC}"
    else
        log "${AMBER}Update available: $LATEST${NC}"
    fi
    exit 0
fi

if [ "$CURRENT" = "$LATEST" ]; then
    log "${GREEN}Already running latest version ($LATEST)${NC}"
    exit 0
fi

# Download new binaries
DOWNLOAD_URL="$SERVER/$CHANNEL"
mkdir -p "$STAGING"

log "Downloading v$LATEST from $CHANNEL..."
for bin in yantrik-ui yantrik; do
    if ! curl -fsSL "$DOWNLOAD_URL/$bin" -o "$STAGING/$bin"; then
        log "${RED}Failed to download $bin${NC}"
        rm -rf "$STAGING"
        exit 1
    fi
    chmod +x "$STAGING/$bin"
done

# Verify checksums if available
if curl -fsSL "$DOWNLOAD_URL/sha256sums.txt" -o "$STAGING/sha256sums.txt" 2>/dev/null; then
    cd "$STAGING"
    if sha256sum -c sha256sums.txt >/dev/null 2>&1; then
        log "Checksums verified"
    else
        log "${RED}Checksum verification failed — aborting${NC}"
        rm -rf "$STAGING"
        exit 1
    fi
    cd /
fi

# Backup current binaries
mkdir -p "$BIN_DIR/backup"
for bin in yantrik-ui yantrik; do
    [ -f "$BIN_DIR/$bin" ] && cp "$BIN_DIR/$bin" "$BIN_DIR/backup/$bin.bak"
done

# Stop, swap, start
log "Stopping yantrik-ui..."
for pid in $(pgrep -f '/opt/yantrik/bin/yantrik-ui' 2>/dev/null); do
    kill "$pid" 2>/dev/null
done
sleep 2

log "Installing new binaries..."
cp "$STAGING/yantrik-ui" "$BIN_DIR/yantrik-ui"
cp "$STAGING/yantrik" "$BIN_DIR/yantrik"
echo "$LATEST" > /opt/yantrik/.version

# Also run Alpine package updates
log "Updating system packages..."
apk update -q 2>/dev/null && apk upgrade -q 2>/dev/null && log "System packages updated" || log "${AMBER}Package update skipped${NC}"

log "Starting yantrik-ui..."
su - yantrik -c '
    WAYLAND_DISPLAY=wayland-0 \
    XDG_RUNTIME_DIR=/run/user/1000 \
    DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
    LD_PRELOAD="/lib/libgcompat.so.0 /usr/lib/libgcompat_shim.so" \
    nohup /opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml \
        >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
'

# Verify (rollback if crash)
sleep 5
if pgrep -f '/opt/yantrik/bin/yantrik-ui' >/dev/null 2>&1; then
    log "${GREEN}Upgrade successful: $CHANNEL v$LATEST${NC}"
    rm -rf "$STAGING"
else
    log "${RED}New binary crashed — rolling back...${NC}"
    for bin in yantrik-ui yantrik; do
        [ -f "$BIN_DIR/backup/$bin.bak" ] && cp "$BIN_DIR/backup/$bin.bak" "$BIN_DIR/$bin"
    done
    su - yantrik -c '
        WAYLAND_DISPLAY=wayland-0 \
        XDG_RUNTIME_DIR=/run/user/1000 \
        DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
        LD_PRELOAD="/lib/libgcompat.so.0 /usr/lib/libgcompat_shim.so" \
        nohup /opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml \
            >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
    '
    log "${AMBER}Rolled back to previous version${NC}"
    rm -rf "$STAGING"
    exit 1
fi
UPGRADE
chmod +x "$BIN_DIR/yantrik-upgrade"

ok "yantrik-upgrade installed at $BIN_DIR/yantrik-upgrade"
echo

# ═══════════════════════════════════════════════════════════════
# STEP 11: Fix Ownership & Final Checks
# ═══════════════════════════════════════════════════════════════
step "Finalizing..."

chown -R "$YANTRIK_USER:$YANTRIK_USER" "$YANTRIK_HOME" 2>/dev/null || true

# Store installed version
echo "$YANTRIK_VERSION" > "$YANTRIK_HOME/.version"

echo
echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Yantrik OS — Installation Complete${NC}"
echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
echo
echo -e "  ${BOLD}Binary:${NC}     $BIN_DIR/yantrik-ui"
echo -e "  ${BOLD}Config:${NC}     $YANTRIK_HOME/config.yaml"
echo -e "  ${BOLD}Data:${NC}       $DATA_DIR/"
echo -e "  ${BOLD}Models:${NC}     $MODEL_DIR/"
echo -e "  ${BOLD}Logs:${NC}       $LOG_DIR/"
echo -e "  ${BOLD}Upgrade:${NC}    $BIN_DIR/yantrik-upgrade"
echo
echo -e "  ${BOLD}User:${NC}       $USER_NAME"
echo -e "  ${BOLD}Companion:${NC}  $COMPANION_NAME"
echo -e "  ${BOLD}LLM:${NC}       $LLM_BACKEND / $LLM_MODEL"
echo -e "  ${BOLD}Platform:${NC}   $HYPERVISOR"
echo
if [ -x "$BIN_DIR/yantrik-ui" ] && [ -s "$BIN_DIR/yantrik-ui" ]; then
    echo -e "  ${GREEN}Reboot to start Yantrik OS:${NC}"
    echo -e "    ${BOLD}reboot${NC}"
else
    echo -e "  ${AMBER}Upload binaries first:${NC}"
    echo -e "    scp yantrik-ui yantrik root@this-machine:/tmp/"
    echo -e "    Then re-run: ./install.sh"
fi
echo
