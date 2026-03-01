#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# Yantrik OS — Deploy AI-Native Desktop Stack
# ═══════════════════════════════════════════════════════════════
#
# Run this ON the Alpine VM after OS installation.
# Deploys: labwc compositor, Yantrik UI binary, AI models, config.
#
# Everything runs in-process — no Python, no llama.cpp, no
# external servers. Single Rust binary with Candle LLM.
#
# Usage:
#   chmod +x deploy-stack.sh
#   ./deploy-stack.sh          (run as root)
#
# Prerequisites:
#   - Alpine Linux installed (via setup-alpine-vm.sh)
#   - Community repo enabled in /etc/apk/repositories
#   - SSH access (ssh -p 2222 root@localhost)
#   - Yantrik UI binary uploaded to /tmp/yantrik-ui
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

YANTRIK_HOME="/opt/yantrik"
YANTRIK_USER="yantrik"
BIN_DIR="$YANTRIK_HOME/bin"
DATA_DIR="$YANTRIK_HOME/data"
MODEL_DIR="$YANTRIK_HOME/models"
LOG_DIR="$YANTRIK_HOME/logs"
CONFIG_DIR="$YANTRIK_HOME"

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Desktop Stack Deployment"
echo "═══════════════════════════════════════════════"
echo
echo "  Mode: AI-native desktop (labwc + Slint)"
echo "  No Python, no llama.cpp, no external servers"
echo

# ── Verify Alpine ──
if [ ! -f /etc/alpine-release ]; then
    echo "Warning: Not Alpine Linux. Continuing anyway..."
fi
echo "OS: Alpine $(cat /etc/alpine-release 2>/dev/null || echo 'unknown')"

# ── Step 0: Enable community repo ──
echo
echo "[0/8] Enabling community repository..."
if grep -q "^#.*community" /etc/apk/repositories; then
    sed -i 's|^#\(.*community\)|\1|' /etc/apk/repositories
    echo "  Community repo enabled"
else
    echo "  Community repo already enabled"
fi

# ── Step 1: System packages ──
echo
echo "[1/8] Installing system packages..."
apk update -q
apk add -q \
    labwc \
    foot \
    dbus dbus-openrc \
    eudev eudev-openrc \
    mesa-dri-gallium \
    font-dejavu \
    seatd seatd-openrc \
    wlr-randr \
    alsa-utils alsa-lib \
    curl wget \
    openrc \
    gcompat \
    libinput \
    wayland-libs-egl wayland-libs-client wayland-libs-server wayland-libs-cursor \
    speech-dispatcher \
    ca-certificates \
    gcc musl-dev \
    grim slurp jq bc diffutils

echo "  Installed: labwc, foot, dbus, eudev, mesa, fonts, seatd, gcompat, wayland"

# Install optional desktop apps (non-fatal if unavailable)
apk add -q thunar 2>/dev/null && echo "  Installed: thunar (file manager)" || echo "  thunar not available, skipping"
apk add -q firefox-esr 2>/dev/null && echo "  Installed: firefox-esr (browser)" || echo "  firefox-esr not available, skipping"

# ── Step 1b: Build glibc compatibility shim ──
echo "  Building glibc shim..."
cat > /tmp/glibc_shim.c <<'SHIM'
#include <fcntl.h>
#include <stdarg.h>

/* fcntl64 is glibc 2.28+; musl fcntl is already 64-bit */
int fcntl64(int fd, int cmd, ...) {
    va_list ap;
    va_start(ap, cmd);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    return fcntl(fd, cmd, arg);
}

/* __res_init is glibc resolver init; musl handles this automatically */
int __res_init(void) {
    return 0;
}

/* gnu_get_libc_version placeholder */
const char *gnu_get_libc_version(void) {
    return "2.38";
}
SHIM
gcc -shared -o /usr/lib/libglibc_shim.so /tmp/glibc_shim.c
rm /tmp/glibc_shim.c
echo "  glibc shim installed at /usr/lib/libglibc_shim.so"

# ── Step 1c: Fix DRI driver path ──
echo "  Fixing DRI driver paths..."
if [ -d /usr/lib/xorg/modules/dri ] && [ ! -d /usr/lib/dri ]; then
    mkdir -p /usr/lib/dri
    cp /usr/lib/xorg/modules/dri/*.so /usr/lib/dri/ 2>/dev/null || true
    echo "  DRI drivers copied to /usr/lib/dri/"
else
    echo "  DRI paths OK"
fi

# ── Step 2: Create user and directories ──
echo "[2/8] Setting up directories..."
id "$YANTRIK_USER" &>/dev/null || adduser -D -h "/home/$YANTRIK_USER" "$YANTRIK_USER" 2>/dev/null || true
addgroup "$YANTRIK_USER" seat 2>/dev/null || true
addgroup "$YANTRIK_USER" video 2>/dev/null || true
addgroup "$YANTRIK_USER" audio 2>/dev/null || true
addgroup "$YANTRIK_USER" input 2>/dev/null || true

mkdir -p "$BIN_DIR" "$DATA_DIR" "$MODEL_DIR" "$LOG_DIR" \
    "$MODEL_DIR/llm" "$MODEL_DIR/embedder" "$MODEL_DIR/whisper"

# ── Step 3: Deploy binary ──
echo "[3/8] Deploying Yantrik binary..."
BINARY_SRC=""

for f in \
    "/tmp/yantrik-ui" \
    "$HOME/yantrik-ui" \
    "$YANTRIK_HOME/yantrik-ui"; do
    if [ -f "$f" ]; then
        BINARY_SRC="$f"
        break
    fi
done

if [ -n "$BINARY_SRC" ]; then
    cp "$BINARY_SRC" "$BIN_DIR/yantrik-ui"
    chmod +x "$BIN_DIR/yantrik-ui"
    echo "  Binary: $BIN_DIR/yantrik-ui (from $BINARY_SRC)"
else
    echo "  WARNING: No yantrik-ui binary found."
    echo "  Upload it to the VM:"
    echo "    scp -P 2222 target/release/yantrik-ui root@localhost:/tmp/"
    echo "  Then re-run this script."
    echo
    echo "  Creating placeholder for now..."
    touch "$BIN_DIR/yantrik-ui"
fi

# ── Step 4: Download models ──
echo "[4/8] Downloading AI models..."

# MiniLM embedder (~87MB)
if [ ! -f "$MODEL_DIR/embedder/model.safetensors" ]; then
    echo "  Downloading MiniLM embedder (~87MB)..."
    HF_EMB="https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main"
    for f in config.json tokenizer.json tokenizer_config.json special_tokens_map.json model.safetensors; do
        wget -q -O "$MODEL_DIR/embedder/$f" "$HF_EMB/$f"
    done
    echo "  Embedder downloaded"
else
    echo "  Embedder already present"
fi

# Qwen2.5-3B-Instruct GGUF (~1.7GB) — desktop-grade model
if [ ! -f "$MODEL_DIR/llm/qwen2.5-3b-instruct-q4_k_m.gguf" ]; then
    echo "  Downloading Qwen2.5-3B GGUF (~1.7GB)..."
    HF_LLM="https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main"
    HF_TOK="https://huggingface.co/Qwen/Qwen2.5-3B-Instruct/resolve/main"
    wget -q -O "$MODEL_DIR/llm/qwen2.5-3b-instruct-q4_k_m.gguf" "$HF_LLM/qwen2.5-3b-instruct-q4_k_m.gguf"
    wget -q -O "$MODEL_DIR/llm/tokenizer.json" "$HF_TOK/tokenizer.json"
    wget -q -O "$MODEL_DIR/llm/config.json" "$HF_TOK/config.json"
    echo "  LLM downloaded"
else
    echo "  LLM already present"
fi

# ── Step 5: Configuration ──
echo "[5/8] Writing configuration..."
cat > "$CONFIG_DIR/config.yaml" <<'CONFIG'
user_name: "Pranab"

personality:
  name: "Yantrik"
  system_prompt: >
    You are Yantrik, a personal AI companion running as the desktop shell.
    You remember everything the user tells you. You are warm, thoughtful,
    and occasionally curious. You are aware of the system state — battery,
    network, running apps, files. When you notice patterns or have concerns,
    you bring them up naturally. You never fabricate memories —
    if you don't know, say so.

llm:
  model_dir: "/opt/yantrik/models/llm"
  hub_repo: "Qwen/Qwen2.5-3B-Instruct-GGUF"
  hub_gguf: "qwen2.5-3b-instruct-q4_k_m.gguf"
  hub_tokenizer: "Qwen/Qwen2.5-3B-Instruct"
  max_tokens: 512
  temperature: 0.7
  max_context_tokens: 4096

server:
  host: "0.0.0.0"
  port: 8340

yantrikdb:
  db_path: "/opt/yantrik/data/memory.db"
  embedding_dim: 384
  embedder_model_dir: "/opt/yantrik/models/embedder"

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
CONFIG

# ── Step 6: labwc compositor config ──
echo "[6/8] Configuring labwc compositor..."

LABWC_DIR="/home/$YANTRIK_USER/.config/labwc"
mkdir -p "$LABWC_DIR"

# labwc environment — all QEMU/Wayland env vars
cat > "$LABWC_DIR/environment" <<'ENV'
# glibc compat shim (binary built on Ubuntu glibc, running on Alpine musl)
LD_PRELOAD=/usr/lib/libglibc_shim.so

# Use virtio-gpu (card0), not bochs VGA (card1)
WLR_DRM_DEVICES=/dev/dri/card0

# Allow software rendering fallback (QEMU virtio-gpu)
WLR_RENDERER_ALLOW_SOFTWARE=1

# Suppress libinput check (QEMU uses evdev)
WLR_LIBINPUT_NO_DEVICES=1

# Slint backend: 'winit' auto-detects GPU, falls back to software if needed.
# Use 'winit-software' only if GPU rendering fails completely.
SLINT_BACKEND=winit
ENV

# labwc autostart — launch yantrik-ui as fullscreen desktop shell
cat > "$LABWC_DIR/autostart" <<'AUTOSTART'
#!/bin/sh
# Start Yantrik OS desktop shell
/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
AUTOSTART
chmod +x "$LABWC_DIR/autostart"

# labwc rc.xml — window rules and key bindings
cat > "$LABWC_DIR/rc.xml" <<'RCXML'
<?xml version="1.0" encoding="UTF-8"?>
<labwc_config>
  <core>
    <gap>0</gap>
  </core>

  <theme>
    <name></name>
    <titlebar>
      <font name="DejaVu Sans" size="10" />
    </titlebar>
  </theme>

  <keyboard>
    <!-- Alt+Tab window switching -->
    <keybind key="A-Tab">
      <action name="NextWindow" />
    </keybind>

    <!-- Alt+F4 close window -->
    <keybind key="A-F4">
      <action name="Close" />
    </keybind>

    <!-- Super key launches terminal (quick access) -->
    <keybind key="Super_L">
      <action name="Execute">
        <command>foot</command>
      </action>
    </keybind>
  </keyboard>

  <windowRules>
    <!-- Yantrik shell starts maximized (acts as desktop) -->
    <windowRule identifier="yantrik-ui">
      <action name="Maximize" />
    </windowRule>
  </windowRules>
</labwc_config>
RCXML

chown -R "$YANTRIK_USER:$YANTRIK_USER" "$LABWC_DIR"

echo "  labwc config written to $LABWC_DIR/"

# ── Step 7: Enable system services ──
echo "[7/8] Configuring services..."

rc-update add dbus default 2>/dev/null || true
rc-update add seatd default 2>/dev/null || true
rc-update add udev sysinit 2>/dev/null || true

rc-service seatd start 2>/dev/null || true
rc-service dbus start 2>/dev/null || true

echo "  Services: dbus, seatd enabled"

# ── Step 8: Auto-login + labwc setup ──
echo "[8/8] Configuring auto-login..."

# Configure getty for auto-login on tty1
if [ -f /etc/inittab ]; then
    sed -i 's|^tty1::.*|tty1::respawn:/sbin/getty -n -l /opt/yantrik/bin/yantrik-login 38400 tty1|' /etc/inittab 2>/dev/null || true
fi

# Create login wrapper that starts labwc
cat > "$BIN_DIR/yantrik-login" <<'LOGIN'
#!/bin/sh
# Auto-login as yantrik user and start labwc compositor
if [ "$(whoami)" = "root" ]; then
    exec su -l yantrik -c "exec /opt/yantrik/bin/yantrik-start"
else
    exec /opt/yantrik/bin/yantrik-start
fi
LOGIN
chmod +x "$BIN_DIR/yantrik-login"

# Create startup script that launches labwc (which auto-starts yantrik-ui)
cat > "$BIN_DIR/yantrik-start" <<'START'
#!/bin/sh
# Start Yantrik OS desktop via labwc compositor
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
mkdir -p "$XDG_RUNTIME_DIR"

LOG=/opt/yantrik/logs/yantrik-os.log
echo "$(date): Starting Yantrik OS desktop..." >> "$LOG"

# labwc reads its config from ~/.config/labwc/
# It will auto-start yantrik-ui via the autostart file
exec labwc >> "$LOG" 2>&1
START
chmod +x "$BIN_DIR/yantrik-start"

# XDG runtime dir boot script (survives reboot)
cat > /etc/local.d/xdg-runtime.start <<'XDG'
#!/bin/sh
YANTRIK_UID=$(id -u yantrik 2>/dev/null || echo 1000)
mkdir -p "/run/user/$YANTRIK_UID"
chown yantrik:yantrik "/run/user/$YANTRIK_UID"
chmod 700 "/run/user/$YANTRIK_UID"
XDG
chmod +x /etc/local.d/xdg-runtime.start
rc-update add local default 2>/dev/null || true

# Fix ownership
chown -R "$YANTRIK_USER:$YANTRIK_USER" "$YANTRIK_HOME" 2>/dev/null || true

echo
echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Desktop Deployment Complete"
echo "═══════════════════════════════════════════════"
echo
echo "  Binary:     $BIN_DIR/yantrik-ui"
echo "  Config:     $CONFIG_DIR/config.yaml"
echo "  Data:       $DATA_DIR/"
echo "  Models:     $MODEL_DIR/"
echo "  Logs:       $LOG_DIR/"
echo "  Compositor: labwc ($LABWC_DIR/)"
echo
echo "  Auto-start: labwc → yantrik-ui on tty1"
echo "  Services:   seatd, dbus (OpenRC)"
echo
if [ -x "$BIN_DIR/yantrik-ui" ]; then
    echo "  Reboot to start:"
    echo "    reboot"
else
    echo "  NEXT: Upload the binary:"
    echo "    scp -P 2222 target/release/yantrik-ui root@localhost:/tmp/"
    echo "  Then re-run: ./deploy-stack.sh"
fi
echo
