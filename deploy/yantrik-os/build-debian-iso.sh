#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# build-debian-iso.sh — Create a bootable Yantrik OS ISO (Debian-based)
# ═══════════════════════════════════════════════════════════════
#
# Builds a Debian Trixie-based live/installer ISO containing:
#   - Debian 13 (testing) minimal (glibc, systemd, NetworkManager)
#   - labwc Wayland compositor (Yantrik runs fullscreen)
#   - Yantrik UI + CLI binaries (pre-compiled)
#   - MiniLM embedder model (~87MB)
#   - Whisper tiny model (~146MB)
#   - Qwen 3.5 4B GGUF offline LLM (~2.5GB)
#   - Calamares installer for disk installation
#   - All packages baked in — NO internet needed for install
#
# Output: yantrik-os-<version>.iso
#
# Requirements (run on Ubuntu/WSL2):
#   - debootstrap, xorriso, squashfs-tools, grub-pc-bin, grub-efi-amd64-bin
#   - sudo access
#   - yantrik-ui + yantrik binaries already built (release)
#
# Usage:
#   ./build-debian-iso.sh                     # default (Ollama backend)
#   ./build-debian-iso.sh --with-llm          # include offline LLM (~2.5GB)
#   ./build-debian-iso.sh --skip-models       # skip model downloads
#   YANTRIK_BINARY=/path/to/bin ./build-debian-iso.sh
#
# Test:
#   qemu-system-x86_64 -m 4G -cdrom yantrik-os-*.iso -boot d \
#     -enable-kvm -display gtk -device virtio-vga
#
# VirtualBox:
#   Import ISO as boot media, 4GB RAM, EFI enabled
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-/home/yantrik/target-yantrik}"

# ── Configuration ──
YANTRIK_VERSION="0.3.0"
DEBIAN_SUITE="trixie"
DEBIAN_MIRROR="http://deb.debian.org/debian"
ARCH="amd64"

WORK_DIR="/tmp/yantrik-debian-iso"
ROOTFS="$WORK_DIR/rootfs"
ISO_DIR="$WORK_DIR/iso"
OUTPUT="yantrik-os-${YANTRIK_VERSION}.iso"

BINARY_UI="${YANTRIK_BINARY:-$TARGET_DIR/release/yantrik-ui}"
BINARY_CLI="${YANTRIK_CLI_BINARY:-$TARGET_DIR/release/yantrik}"

INCLUDE_LLM=false
INCLUDE_WHISPER=false
# Embedder is ALWAYS included — essential for CognitiveRouter (Core Mode)
# Without it, 80% of functionality is lost. Only 22MB.

# ── Parse flags ──
for arg in "$@"; do
    case "$arg" in
        --with-llm)     INCLUDE_LLM=true; INCLUDE_WHISPER=true ;;
        --with-models)  INCLUDE_WHISPER=true ;;
        --no-embedder)  NO_EMBEDDER=true ;;
        --help|-h)
            echo "Usage: $0 [--with-models] [--with-llm]"
            echo "  (default)      Include embedder for Core Mode (~22MB, always)"
            echo "  --with-models  Also include whisper voice model (+146MB)"
            echo "  --with-llm    Include all models + offline LLM (+2.7GB)"
            echo ""
            echo "Embedder is always included (essential for CognitiveRouter)."
            echo "Without it, tool routing, recipe matching, and most"
            echo "interactions require an external LLM."
            exit 0
            ;;
    esac
done

# ── Colors ──
CYAN='\033[0;36m'
GREEN='\033[0;32m'
AMBER='\033[0;33m'
RED='\033[0;31m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

step()  { echo -e "\n${CYAN}::${NC} ${BOLD}$1${NC}"; }
info()  { echo -e "   ${DIM}$1${NC}"; }
ok()    { echo -e "   ${GREEN}✓${NC} $1"; }
warn()  { echo -e "   ${AMBER}!${NC} $1"; }
fail()  { echo -e "   ${RED}✗${NC} $1"; exit 1; }

echo
echo -e "${CYAN}╔═══════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║${NC}  ${BOLD}Yantrik OS${NC} — Debian ISO Builder v${YANTRIK_VERSION}         ${CYAN}║${NC}"
echo -e "${CYAN}║${NC}  ${DIM}Fully offline-capable installation ISO${NC}           ${CYAN}║${NC}"
echo -e "${CYAN}╚═══════════════════════════════════════════════════╝${NC}"
echo
echo -e "  Base:       ${BOLD}Debian ${DEBIAN_SUITE}${NC} (${ARCH})"
echo -e "  Core Mode:   ${BOLD}always (embedder baked in)${NC}"
echo -e "  Voice:       ${BOLD}${INCLUDE_WHISPER}${NC}"
echo -e "  Offline LLM: ${BOLD}${INCLUDE_LLM}${NC}"
echo -e "  Output:     ${BOLD}${OUTPUT}${NC}"
echo

# ── Verify prerequisites ──
MISSING=""
for cmd in debootstrap xorriso mksquashfs grub-mkrescue; do
    command -v "$cmd" &>/dev/null || MISSING="$MISSING $cmd"
done
if [ -n "$MISSING" ]; then
    fail "Missing tools:$MISSING\n  Install: sudo apt install debootstrap xorriso squashfs-tools grub-pc-bin grub-efi-amd64-bin mtools"
fi

for bin in "$BINARY_UI" "$BINARY_CLI"; do
    if [ ! -f "$bin" ]; then
        fail "Binary not found: $bin\n  Build first: cargo build --release -p yantrik-ui -p yantrik"
    fi
done

# ── Cleanup function ──
cleanup() {
    echo "Cleaning up mounts..."
    for mp in proc sys dev/pts dev; do
        sudo umount "$ROOTFS/$mp" 2>/dev/null || true
    done
}
trap cleanup EXIT

# ═══════════════════════════════════════════════════════════════
# STEP 1: Bootstrap Debian rootfs
# ═══════════════════════════════════════════════════════════════
step "[1/10] Bootstrapping Debian ${DEBIAN_SUITE} rootfs..."

sudo rm -rf "$WORK_DIR"
mkdir -p "$ROOTFS" "$ISO_DIR/boot/grub" "$ISO_DIR/live" "$ISO_DIR/install"

sudo debootstrap \
    --arch="$ARCH" \
    --variant=minbase \
    --include=systemd,systemd-sysv,dbus,udev,sudo,ca-certificates,locales,wget,curl \
    "$DEBIAN_SUITE" "$ROOTFS" "$DEBIAN_MIRROR"

ok "Base system bootstrapped"

# Mount for chroot
sudo mount -t proc proc "$ROOTFS/proc"
sudo mount -t sysfs sysfs "$ROOTFS/sys"
sudo mount --bind /dev "$ROOTFS/dev"
sudo mount --bind /dev/pts "$ROOTFS/dev/pts"
sudo cp /etc/resolv.conf "$ROOTFS/etc/resolv.conf"

# ═══════════════════════════════════════════════════════════════
# STEP 2: Configure apt sources + install packages
# ═══════════════════════════════════════════════════════════════
step "[2/10] Installing system packages (all cached for offline)..."

# Full sources.list with contrib + non-free for firmware
sudo tee "$ROOTFS/etc/apt/sources.list" > /dev/null <<APT
deb ${DEBIAN_MIRROR} ${DEBIAN_SUITE} main contrib non-free non-free-firmware
APT

sudo chroot "$ROOTFS" /bin/bash <<'CHROOT_PACKAGES'
set -e
export DEBIAN_FRONTEND=noninteractive

apt-get update -qq

# ── Kernel + boot ──
apt-get install -y -qq \
    linux-image-amd64 \
    grub-pc grub-efi-amd64-bin \
    initramfs-tools \
    live-boot live-config live-config-systemd

# ── Wayland desktop (installer-only minimal) ──
apt-get install -y -qq \
    labwc foot \
    wl-clipboard \
    mesa-utils libgl1-mesa-dri libegl-mesa0 \
    libinput-tools \
    fonts-dejavu-core \
    xwayland || true

# ── Network + hardware ──
apt-get install -y -qq \
    network-manager \
    wpasupplicant \
    pciutils usbutils || true

# ── Firmware (non-free, for real hardware) ──
for pkg in firmware-linux-free firmware-misc-nonfree firmware-realtek \
           firmware-iwlwifi firmware-amd-graphics; do
    apt-get install -y -qq "$pkg" 2>/dev/null || true
done

# ── VirtualBox guest support ──
apt-get install -y -qq virtualbox-guest-utils 2>/dev/null || true

# ── Yantrik UI runtime dependencies ──
apt-get install -y -qq \
    speech-dispatcher libspeechd2 \
    2>/dev/null || true

# ── Browser (for API key setup during onboarding) ──
apt-get install -y -qq epiphany-browser 2>/dev/null || true

# ── Utilities (installer essentials) ──
apt-get install -y -qq \
    jq parted rsync openssh-server \
    dosfstools e2fsprogs grub-efi-amd64-bin grub-pc-bin \
    initramfs-tools || true

# ── Calamares installer ──
apt-get install -y -qq \
    calamares calamares-settings-debian \
    2>/dev/null || {
        echo "Calamares not in repos — will use text installer"
    }

# ── Locale ──
echo "en_US.UTF-8 UTF-8" > /etc/locale.gen
locale-gen
update-locale LANG=en_US.UTF-8

# ── Regenerate initramfs with live-boot hooks ──
# CRITICAL: live-boot was installed after the kernel, so the initrd
# doesn't contain the live-boot hooks yet. Without this, the ISO
# will kernel panic (can't find root) and reboot loop.
update-initramfs -u

# ── Clean apt cache but keep .deb files for offline install ──
# We keep /var/cache/apt/archives/ populated so the installed system
# can reinstall packages without internet
apt-get clean

echo "Package installation complete"
CHROOT_PACKAGES

ok "All packages installed"

# ═══════════════════════════════════════════════════════════════
# STEP 3: Create yantrik user + directory structure
# ═══════════════════════════════════════════════════════════════
step "[3/10] Creating user and directories..."

sudo chroot "$ROOTFS" /bin/bash <<'CHROOT_USER'
set -e

# Create yantrik user
useradd -m -s /bin/bash -G sudo,video,audio,input yantrik
echo "yantrik:yantrik" | chpasswd
echo "root:root" | chpasswd

# Passwordless sudo for yantrik
echo "yantrik ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/yantrik
chmod 440 /etc/sudoers.d/yantrik

# Directory structure
mkdir -p /opt/yantrik/{bin,data,logs,models/{embedder,whisper,llm},skills,i18n}

# Hostname
echo "yantrik" > /etc/hostname
cat > /etc/hosts <<HOSTS
127.0.0.1   localhost
127.0.1.1   yantrik
HOSTS
CHROOT_USER

ok "User yantrik created"

# ═══════════════════════════════════════════════════════════════
# STEP 4: Install Yantrik binaries
# ═══════════════════════════════════════════════════════════════
step "[4/10] Installing Yantrik OS binaries..."

sudo cp "$BINARY_UI" "$ROOTFS/opt/yantrik/bin/yantrik-ui"
sudo cp "$BINARY_CLI" "$ROOTFS/opt/yantrik/bin/yantrik"
sudo chmod +x "$ROOTFS/opt/yantrik/bin/yantrik-ui" "$ROOTFS/opt/yantrik/bin/yantrik"

# Copy i18n files if they exist
if [ -d "$PROJECT_ROOT/crates/yantrik-ui/i18n" ]; then
    sudo cp -r "$PROJECT_ROOT/crates/yantrik-ui/i18n/"* "$ROOTFS/opt/yantrik/i18n/" 2>/dev/null || true
fi

# Copy skill manifests if they exist
if [ -d "$PROJECT_ROOT/skills" ]; then
    sudo cp -r "$PROJECT_ROOT/skills/"* "$ROOTFS/opt/yantrik/skills/" 2>/dev/null || true
fi

ok "Binaries installed ($(du -h "$BINARY_UI" | cut -f1) + $(du -h "$BINARY_CLI" | cut -f1))"

# ═══════════════════════════════════════════════════════════════
# STEP 5: Download AI models (baked into ISO)
# ═══════════════════════════════════════════════════════════════
step "[5/10] Downloading AI models (baked into ISO)..."

# ── MiniLM embedder (~22MB) — ALWAYS included ──
# Essential for CognitiveRouter (Core Mode). Without this, the OS
# can't route queries to tools/recipes and needs an external LLM for everything.
if [ -z "${NO_EMBEDDER:-}" ]; then
    EMB_DIR="$ROOTFS/opt/yantrik/models/embedder"
    HF_EMB="https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main"
    info "Embedder model (~22MB) — required for Core Mode..."
    for f in config.json tokenizer.json tokenizer_config.json special_tokens_map.json model.safetensors; do
        if [ ! -f "$EMB_DIR/$f" ]; then
            sudo wget -q -O "$EMB_DIR/$f" "$HF_EMB/$f"
        fi
    done
    ok "Embedder model (CognitiveRouter enabled — 245 tools, 50 recipes)"
else
    warn "Embedder skipped (--no-embedder) — Core Mode will NOT work"
fi

# ── Whisper tiny (~146MB) — optional, for voice ──
if $INCLUDE_WHISPER; then
    WHISPER_DIR="$ROOTFS/opt/yantrik/models/whisper"
    HF_WHISPER="https://huggingface.co/openai/whisper-tiny/resolve/main"
    info "Whisper voice model (~146MB)..."
    for f in config.json tokenizer.json model.safetensors; do
        if [ ! -f "$WHISPER_DIR/$f" ]; then
            sudo wget -q -O "$WHISPER_DIR/$f" "$HF_WHISPER/$f"
        fi
    done
    ok "Whisper voice model"
else
    info "Whisper skipped (use --with-models to include voice)"
fi

# ── Offline LLM (~2.6GB) — optional, for Enhanced Mode ──
if $INCLUDE_LLM; then
    LLM_DIR="$ROOTFS/opt/yantrik/models/llm"
    LLM_GGUF="yantrik-4b.gguf"
    # Use our fine-tuned model from Ollama if available on the host
    OLLAMA_BLOB="/mnt/c/Users/sync/.ollama/models/blobs/sha256-485cf5f063803ddb1c8b3f6e48186e2908840855e914952ea242fbe18ac56bb4"
    if [ -f "$OLLAMA_BLOB" ]; then
        info "Copying fine-tuned Yantrik-4B model (~2.6GB)..."
        sudo cp "$OLLAMA_BLOB" "$LLM_DIR/$LLM_GGUF"
        ok "Yantrik-4B fine-tuned model (Enhanced Mode)"
    elif [ ! -f "$LLM_DIR/$LLM_GGUF" ]; then
        info "Fine-tuned model not found, downloading base Qwen3.5-4B (~2.5GB)..."
        sudo wget -q --show-progress -O "$LLM_DIR/$LLM_GGUF" \
            "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf"
        ok "Base Qwen3.5-4B (Enhanced Mode fallback)"
    fi
else
    info "LLM skipped (use --with-llm for offline Enhanced Mode)"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 6: Generate default config
# ═══════════════════════════════════════════════════════════════
step "[6/10] Generating default configuration..."

sudo tee "$ROOTFS/opt/yantrik/config.yaml" > /dev/null <<'CONFIG'
# Yantrik OS — Default Configuration
# This will be customized during first-boot onboarding

user_name: "User"

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
  backend: "api"
  api_base_url: "http://localhost:8341/v1"
  api_model: "qwen3.5-4b"
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

updates:
  channel: "beta"
  server: "http://releases.yantrikos.com"
  check_on_boot: true
CONFIG

ok "Default config generated"

# ═══════════════════════════════════════════════════════════════
# STEP 7: Configure desktop session (labwc + auto-login)
# ═══════════════════════════════════════════════════════════════
step "[7/10] Configuring desktop session..."

# ── labwc config ──
LABWC_DIR="$ROOTFS/home/yantrik/.config/labwc"
sudo mkdir -p "$LABWC_DIR"

# Environment — software rendering fallback ensures it always works
sudo tee "$LABWC_DIR/environment" > /dev/null <<'ENV'
# Yantrik OS — Wayland environment
WLR_RENDERER_ALLOW_SOFTWARE=1
WLR_NO_HARDWARE_CURSORS=1
WLR_RENDERER=pixman
XDG_SESSION_TYPE=wayland
QT_QPA_PLATFORM=wayland
MOZ_ENABLE_WAYLAND=1
SLINT_BACKEND=winit
LIBGL_ALWAYS_SOFTWARE=1
ENV

# Autostart — Yantrik is the shell
sudo tee "$LABWC_DIR/autostart" > /dev/null <<'AUTOSTART'
#!/bin/sh
# Start notification daemon
mako &

# Start Yantrik OS as the desktop shell
/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
AUTOSTART
sudo chmod +x "$LABWC_DIR/autostart"

# Window rules — Yantrik fullscreen, no decorations
sudo tee "$LABWC_DIR/rc.xml" > /dev/null <<'RCXML'
<?xml version="1.0" encoding="UTF-8"?>
<labwc_config>
  <core><gap>0</gap></core>
  <theme><titlebar><height>0</height></titlebar></theme>
  <keyboard>
    <keybind key="A-Tab"><action name="NextWindow" /></keybind>
    <keybind key="A-F4"><action name="Close" /></keybind>
    <keybind key="W-t">
      <action name="Execute"><command>foot</command></action>
    </keybind>
    <keybind key="Print">
      <action name="Execute">
        <command>sh -c 'mkdir -p ~/Pictures/Screenshots &amp;&amp; grim ~/Pictures/Screenshots/$(date +%Y%m%d_%H%M%S).png'</command>
      </action>
    </keybind>
  </keyboard>
  <windowRules>
    <windowRule title="Yantrik*">
      <action name="ToggleDecorations" />
      <action name="ToggleFullscreen" />
    </windowRule>
  </windowRules>
</labwc_config>
RCXML

# ── foot terminal config ──
FOOT_DIR="$ROOTFS/home/yantrik/.config/foot"
sudo mkdir -p "$FOOT_DIR"
sudo tee "$FOOT_DIR/foot.ini" > /dev/null <<'FOOTINI'
[main]
font=DejaVu Sans Mono:size=11
pad=8x4
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

# ── mako config ──
MAKO_DIR="$ROOTFS/home/yantrik/.config/mako"
sudo mkdir -p "$MAKO_DIR"
sudo tee "$MAKO_DIR/config" > /dev/null <<'MAKO'
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
anchor=top-right
MAKO

# Fix ownership
sudo chown -R 1000:1000 "$ROOTFS/home/yantrik"
sudo chown -R 1000:1000 "$ROOTFS/opt/yantrik"

ok "labwc + foot + mako configured"

# ── Auto-login via systemd ──
# Override getty@tty1 to auto-login as yantrik and start labwc
sudo mkdir -p "$ROOTFS/etc/systemd/system/getty@tty1.service.d"
sudo tee "$ROOTFS/etc/systemd/system/getty@tty1.service.d/autologin.conf" > /dev/null <<'AUTOLOGIN'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin yantrik --noclear %I $TERM
AUTOLOGIN

# .bash_profile — auto-start labwc on tty1, with installer mode + crash guard
sudo tee "$ROOTFS/home/yantrik/.bash_profile" > /dev/null <<'PROFILE'
# Auto-start Yantrik desktop on tty1
if [ "$(tty)" = "/dev/tty1" ] && [ -z "$WAYLAND_DISPLAY" ]; then
    export XDG_RUNTIME_DIR="/run/user/$(id -u)"
    mkdir -p "$XDG_RUNTIME_DIR"

    # Source labwc environment (WLR_RENDERER=pixman etc.) so wlroots
    # uses software rendering when nomodeset disables GPU/DRM
    if [ -f "$HOME/.config/labwc/environment" ]; then
        set -a
        . "$HOME/.config/labwc/environment"
        set +a
    fi

    # Check if installer mode was requested via kernel param
    if grep -q 'yantrik.install=true' /proc/cmdline 2>/dev/null; then
        if command -v calamares >/dev/null 2>&1; then
            # Launch Calamares graphical installer through labwc
            mkdir -p ~/.config/labwc
            cp ~/.config/labwc/autostart ~/.config/labwc/autostart.bak 2>/dev/null || true
            # Write installer autostart (no nested heredoc — use printf)
            printf '#!/bin/sh\nmako &\nexport QT_QPA_PLATFORM=wayland\nexport QT_WAYLAND_DISABLE_WINDOWDECORATION=1\nsudo env XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WAYLAND_DISPLAY="$WAYLAND_DISPLAY" QT_QPA_PLATFORM=wayland calamares >> /opt/yantrik/logs/calamares.log 2>&1 &\n' > ~/.config/labwc/autostart
            chmod +x ~/.config/labwc/autostart
            labwc 2>/opt/yantrik/logs/labwc.log
            # Restore original autostart after installer exits
            cp ~/.config/labwc/autostart.bak ~/.config/labwc/autostart 2>/dev/null || true
        elif [ -x /opt/yantrik/bin/yantrik-install ]; then
            # Fallback: text-based installer
            clear
            sudo /opt/yantrik/bin/yantrik-install
        fi
        exec /bin/bash --login
    fi

    # Crash guard: if labwc crashed recently, don't loop — drop to shell
    CRASH_FILE="/tmp/.yantrik-labwc-crash"
    if [ -f "$CRASH_FILE" ]; then
        LAST_CRASH=$(cat "$CRASH_FILE" 2>/dev/null || echo 0)
        NOW=$(date +%s)
        # If crashed less than 10 seconds ago, stop looping
        if [ $((NOW - LAST_CRASH)) -lt 10 ]; then
            echo
            echo "============================================="
            echo "  Yantrik OS — Desktop failed to start"
            echo "============================================="
            echo
            echo "  labwc (Wayland compositor) crashed on startup."
            echo "  Common fixes:"
            echo "    1. Reboot and select 'Safe Mode' from the menu"
            echo "    2. Check GPU: lspci | grep -i vga"
            echo "    3. Try: WLR_RENDERER_ALLOW_SOFTWARE=1 labwc"
            echo "    4. View logs: cat /opt/yantrik/logs/labwc.log"
            echo
            exec /bin/bash --login
        fi
    fi

    # Start labwc, record crash timestamp if it exits quickly
    START_TIME=$(date +%s)
    labwc 2>/opt/yantrik/logs/labwc.log
    EXIT_TIME=$(date +%s)

    # If labwc ran less than 5 seconds, it probably crashed
    if [ $((EXIT_TIME - START_TIME)) -lt 5 ]; then
        echo "$EXIT_TIME" > "$CRASH_FILE"
    else
        rm -f "$CRASH_FILE"
    fi
fi
PROFILE
sudo chown 1000:1000 "$ROOTFS/home/yantrik/.bash_profile"

# Ensure XDG runtime dir exists on boot
sudo tee "$ROOTFS/etc/tmpfiles.d/yantrik-xdg.conf" > /dev/null <<'TMPFILES'
d /run/user/1000 0700 yantrik yantrik -
TMPFILES

ok "Auto-login → labwc → Yantrik configured"

# ═══════════════════════════════════════════════════════════════
# STEP 8: Configure offline LLM server (systemd)
# ═══════════════════════════════════════════════════════════════
step "[8/10] Configuring offline LLM server..."

if $INCLUDE_LLM; then
    # ── Install pre-built llama-server variants ──
    # Cache directory contains pre-built binaries:
    #   llama-server-debian-amd64       — CPU-only (always works)
    #   llama-server-debian-amd64-cuda  — NVIDIA GPU (CUDA)
    #   llama-server-debian-amd64-rocm  — AMD GPU (ROCm)
    #   llama-server-debian-amd64-vulkan — Intel Arc / generic Vulkan
    #
    # Build these with deploy/yantrik-os/cache/build-llama-variants.sh
    CACHE_DIR="$SCRIPT_DIR/cache"
    LLAMA_DIR="$ROOTFS/usr/local/lib/llama"
    sudo mkdir -p "$LLAMA_DIR"

    # CPU variant (required — always the fallback)
    CACHED_CPU="$CACHE_DIR/llama-server-debian-amd64"
    if [ -f "$CACHED_CPU" ]; then
        sudo cp "$CACHED_CPU" "$LLAMA_DIR/llama-server-cpu"
        sudo chmod +x "$LLAMA_DIR/llama-server-cpu"
        # Default symlink to CPU
        sudo ln -sf /usr/local/lib/llama/llama-server-cpu "$ROOTFS/usr/local/bin/llama-server"
        ok "llama-server CPU (pre-built cache)"
    else
        info "No cached CPU binary — building from source..."
        sudo chroot "$ROOTFS" /bin/bash <<'CHROOT_LLAMA'
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get install -y -qq build-essential cmake git
cd /tmp
git clone --depth 1 https://github.com/ggerganov/llama.cpp.git 2>/dev/null || true
if [ -d llama.cpp ]; then
    cd llama.cpp
    cmake -B build -DGGML_BLAS=OFF -DGGML_CUDA=OFF \
        -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF \
        -DLLAMA_BUILD_SERVER=ON 2>/dev/null
    cmake --build build --target llama-server -j$(nproc) 2>/dev/null && {
        mkdir -p /usr/local/lib/llama
        cp build/bin/llama-server /usr/local/lib/llama/llama-server-cpu
        ln -sf /usr/local/lib/llama/llama-server-cpu /usr/local/bin/llama-server
    } || echo "llama-server build failed"
    cd /tmp && rm -rf llama.cpp
fi
apt-get remove -y -qq build-essential cmake git
apt-get autoremove -y -qq
CHROOT_LLAMA
    fi

    # GPU variants (optional — installed alongside CPU)
    for variant in cuda rocm vulkan; do
        CACHED_GPU="$CACHE_DIR/llama-server-debian-amd64-${variant}"
        if [ -f "$CACHED_GPU" ]; then
            sudo cp "$CACHED_GPU" "$LLAMA_DIR/llama-server-${variant}"
            sudo chmod +x "$LLAMA_DIR/llama-server-${variant}"
            ok "llama-server ${variant} (pre-built cache)"
        fi
    done

    # ── GPU auto-detect script (runs at boot) ──
    sudo tee "$ROOTFS/usr/local/bin/llama-select-gpu" > /dev/null <<'GPUSELECT'
#!/bin/sh
# Detect GPU and symlink the best llama-server variant
LLAMA_DIR="/usr/local/lib/llama"
TARGET="/usr/local/bin/llama-server"

# Default to CPU
BEST="$LLAMA_DIR/llama-server-cpu"

if command -v lspci >/dev/null 2>&1; then
    GPU=$(lspci 2>/dev/null | grep -iE 'vga|3d|display' || true)

    if echo "$GPU" | grep -qi nvidia; then
        # Check for NVIDIA driver
        if command -v nvidia-smi >/dev/null 2>&1 && [ -f "$LLAMA_DIR/llama-server-cuda" ]; then
            BEST="$LLAMA_DIR/llama-server-cuda"
            echo "llama-select-gpu: NVIDIA GPU detected, using CUDA variant"
        fi
    elif echo "$GPU" | grep -qi 'amd\|radeon'; then
        if [ -f "$LLAMA_DIR/llama-server-rocm" ]; then
            BEST="$LLAMA_DIR/llama-server-rocm"
            echo "llama-select-gpu: AMD GPU detected, using ROCm variant"
        fi
    elif echo "$GPU" | grep -qi 'intel.*arc'; then
        if [ -f "$LLAMA_DIR/llama-server-vulkan" ]; then
            BEST="$LLAMA_DIR/llama-server-vulkan"
            echo "llama-select-gpu: Intel Arc detected, using Vulkan variant"
        fi
    fi
fi

if [ -f "$BEST" ]; then
    ln -sf "$BEST" "$TARGET"
    echo "llama-select-gpu: active → $(basename "$BEST")"
else
    echo "llama-select-gpu: no suitable variant found, keeping CPU"
fi
GPUSELECT
    sudo chmod +x "$ROOTFS/usr/local/bin/llama-select-gpu"

    # Run GPU detection before llama-server starts
    sudo mkdir -p "$ROOTFS/etc/systemd/system/llama-server.service.d"
    sudo tee "$ROOTFS/etc/systemd/system/llama-server.service.d/gpu-detect.conf" > /dev/null <<'GPUCONF'
[Service]
ExecStartPre=/usr/local/bin/llama-select-gpu
GPUCONF

    # Create systemd service for llama-server
    sudo tee "$ROOTFS/etc/systemd/system/llama-server.service" > /dev/null <<'LLAMA_SVC'
[Unit]
Description=Yantrik Offline LLM Server (Qwen 3.5 4B)
After=network.target

[Service]
Type=simple
User=yantrik
ExecStart=/usr/local/bin/llama-server \
    --model /opt/yantrik/models/llm/yantrik-4b.gguf \
    --host 127.0.0.1 --port 8341 \
    --ctx-size 4096 --threads 2 --no-mmap
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
LLAMA_SVC

    # Enable llama-server on boot
    sudo chroot "$ROOTFS" systemctl enable llama-server 2>/dev/null || true

    ok "llama-server configured (systemd, port 8341)"
else
    info "Skipping LLM server (no offline model)"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 9: Configure Calamares installer (optional)
# ═══════════════════════════════════════════════════════════════
step "[9/10] Configuring installer..."

# Check if Calamares was installed
if sudo chroot "$ROOTFS" dpkg -l calamares 2>/dev/null | grep -q '^ii'; then
    info "Calamares found — configuring graphical installer"

    CALA_DIR="$ROOTFS/etc/calamares"
    sudo mkdir -p "$CALA_DIR/branding/yantrik" "$CALA_DIR/modules"

    # Main settings
    sudo tee "$CALA_DIR/settings.conf" > /dev/null <<'CALA_SETTINGS'
modules-search: [ local, /usr/lib/calamares/modules ]

sequence:
  - show:
    - welcome
    - locale
    - keyboard
    - partition
    - users
    - summary
  - exec:
    - partition
    - mount
    - unpackfs
    - machineid
    - fstab
    - locale
    - keyboard
    - localecfg
    - users
    - networkcfg
    - hwclock
    - services-systemd
    - grubcfg
    - bootloader
    - umount
  - show:
    - finished

branding: yantrik
CALA_SETTINGS

    # Branding
    sudo tee "$CALA_DIR/branding/yantrik/branding.desc" > /dev/null <<'BRANDING'
componentName: yantrik

strings:
    productName:         "Yantrik OS"
    shortProductName:    "Yantrik"
    version:             "0.3.0 Beta"
    shortVersion:        "0.3"
    versionedName:       "Yantrik OS 0.3"
    shortVersionedName:  "Yantrik 0.3"
    bootloaderEntryName: "yantrik"
    productUrl:          "https://yantrikos.com"
    supportUrl:          "https://github.com/yantrikos/yantrik-os/issues"
    knownIssuesUrl:      "https://github.com/yantrikos/yantrik-os/issues"
    releaseNotesUrl:     "https://yantrikos.com/releases"

images:
    productLogo:         "logo.png"
    productIcon:         "logo.png"

style:
    sidebarBackground:   "#0c0b10"
    sidebarText:         "#c8c8d0"
    sidebarTextSelect:   "#5ac8d4"

slideshow: "show.qml"
BRANDING

    # ── Module configs ──

    # unpackfs — tells Calamares what to copy to the target disk
    sudo tee "$CALA_DIR/modules/unpackfs.conf" > /dev/null <<'UNPACKFS'
---
unpack:
  - source: /run/live/medium/live/filesystem.squashfs
    sourcefs: squashfs
    destination: ""
UNPACKFS

    # partition — automatic or manual partitioning
    sudo tee "$CALA_DIR/modules/partition.conf" > /dev/null <<'PARTITION'
---
efiSystemPartition: /boot/efi
efiSystemPartitionSize: 512M
userSwapChoices:
  - none
  - small
  - file
drawNestedPartitions: false
alwaysShowPartitionLabels: true
defaultPartitionTableType: gpt
defaultFileSystemType: ext4
PARTITION

    # users — user creation with password
    sudo tee "$CALA_DIR/modules/users.conf" > /dev/null <<'USERS'
---
defaultGroups:
  - name: sudo
    must_exist: true
  - name: video
    must_exist: false
  - name: audio
    must_exist: false
  - name: input
    must_exist: false
  - name: netdev
    must_exist: false
autologinGroup: autologin
doAutologin: false
setRootPassword: true
doReusePassword: true
passwordRequirements:
  minLength: 4
  maxLength: -1
allowWeakPasswords: true
USERS

    # bootloader — GRUB config
    sudo tee "$CALA_DIR/modules/bootloader.conf" > /dev/null <<'BOOTLOADER'
---
efiBootLoader: grub
kernel: /vmlinuz
img: /initrd.img
timeout: 3
grubInstall: "grub-install"
grubMkconfig: "grub-mkconfig"
grubCfg: "/boot/grub/grub.cfg"
BOOTLOADER

    # welcome — requirements check
    sudo tee "$CALA_DIR/modules/welcome.conf" > /dev/null <<'WELCOME'
---
showSupportUrl: true
showKnownIssuesUrl: true
showReleaseNotesUrl: false
requirements:
  requiredStorage: 8
  requiredRam: 1.0
  check:
    - storage
    - ram
  required:
    - storage
WELCOME

    # finished — what to do after install
    sudo tee "$CALA_DIR/modules/finished.conf" > /dev/null <<'FINISHED'
---
restartNowEnabled: true
restartNowChecked: true
restartNowCommand: "systemctl reboot"
FINISHED

    # Simple slideshow (placeholder)
    sudo tee "$CALA_DIR/branding/yantrik/show.qml" > /dev/null <<'QML'
import QtQuick 2.0

Rectangle {
    color: "#0c0b10"

    Text {
        anchors.centerIn: parent
        text: "Installing Yantrik OS...\n\nYour AI-native desktop is being set up."
        color: "#c8c8d0"
        font.pixelSize: 20
        horizontalAlignment: Text.AlignHCenter
    }
}
QML

    # Create a placeholder logo
    # (In production, replace with actual PNG)
    sudo touch "$CALA_DIR/branding/yantrik/logo.png"

    # Desktop entry for installer
    sudo tee "$ROOTFS/usr/share/applications/yantrik-installer.desktop" > /dev/null <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Install Yantrik OS
Comment=Install Yantrik OS to your hard drive
Exec=sudo calamares
Icon=calamares
Terminal=false
Categories=System;
DESKTOP

    ok "Calamares installer configured"
else
    info "Calamares not available — creating text-based installer"

    # Fallback: simple text installer script
    sudo tee "$ROOTFS/opt/yantrik/bin/yantrik-install" > /dev/null <<'TEXT_INSTALLER'
#!/bin/bash
# Yantrik OS — Text-based Installer
# Copies the live system to a target disk

set -euo pipefail

CYAN='\033[0;36m'
GREEN='\033[0;32m'
AMBER='\033[0;33m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'

echo
echo -e "${CYAN}╔═══════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║${NC}  ${BOLD}Yantrik OS${NC} — Disk Installer             ${CYAN}║${NC}"
echo -e "${CYAN}╚═══════════════════════════════════════════╝${NC}"
echo

# List available disks
echo -e "${BOLD}Available disks:${NC}"
lsblk -d -o NAME,SIZE,MODEL | grep -v loop | grep -v sr
echo

echo -n "Target disk (e.g., sda): "
read -r TARGET_DISK

if [ -z "$TARGET_DISK" ]; then
    echo -e "${RED}No disk specified. Aborting.${NC}"
    exit 1
fi

DISK="/dev/$TARGET_DISK"
if [ ! -b "$DISK" ]; then
    echo -e "${RED}$DISK is not a valid block device.${NC}"
    exit 1
fi

echo
echo -e "${AMBER}WARNING: This will ERASE ALL DATA on $DISK${NC}"
echo -n "Type 'yes' to continue: "
read -r CONFIRM
[ "$CONFIRM" = "yes" ] || exit 1

echo
echo -e "${CYAN}::${NC} Partitioning $DISK..."

# GPT partition table: EFI (512M) + root (rest)
parted -s "$DISK" mklabel gpt
parted -s "$DISK" mkpart EFI fat32 1MiB 513MiB
parted -s "$DISK" set 1 esp on
parted -s "$DISK" mkpart root ext4 513MiB 100%

# Format
mkfs.fat -F32 "${DISK}1"
mkfs.ext4 -q -L YANTRIK "${DISK}2"

# Mount
MOUNT_DIR="/mnt/yantrik-install"
mkdir -p "$MOUNT_DIR"
mount "${DISK}2" "$MOUNT_DIR"
mkdir -p "$MOUNT_DIR/boot/efi"
mount "${DISK}1" "$MOUNT_DIR/boot/efi"

echo -e "${CYAN}::${NC} Copying system (this takes a few minutes)..."
rsync -aAXH --exclude='/proc/*' --exclude='/sys/*' --exclude='/dev/*' \
    --exclude='/run/*' --exclude='/tmp/*' --exclude='/mnt/*' \
    --exclude='/live/*' --exclude='/cdrom/*' \
    / "$MOUNT_DIR/" --info=progress2

# Fix fstab
cat > "$MOUNT_DIR/etc/fstab" <<FSTAB
LABEL=YANTRIK  /           ext4  defaults,noatime  0  1
${DISK}1       /boot/efi   vfat  defaults          0  2
FSTAB

# Install GRUB
mount --bind /dev "$MOUNT_DIR/dev"
mount --bind /proc "$MOUNT_DIR/proc"
mount --bind /sys "$MOUNT_DIR/sys"

chroot "$MOUNT_DIR" grub-install --target=x86_64-efi \
    --efi-directory=/boot/efi --bootloader-id=yantrik 2>/dev/null || \
chroot "$MOUNT_DIR" grub-install --target=i386-pc "$DISK" 2>/dev/null || true
chroot "$MOUNT_DIR" update-grub

# Cleanup
umount "$MOUNT_DIR/sys" "$MOUNT_DIR/proc" "$MOUNT_DIR/dev"
umount "$MOUNT_DIR/boot/efi"
umount "$MOUNT_DIR"

echo
echo -e "${GREEN}Installation complete!${NC}"
echo -e "Remove the installation media and reboot."
echo -n "Reboot now? [y/N] "
read -r REBOOT
[ "$REBOOT" = "y" ] && reboot
TEXT_INSTALLER
    sudo chmod +x "$ROOTFS/opt/yantrik/bin/yantrik-install"

    ok "Text-based installer created"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 10: Build ISO image
# ═══════════════════════════════════════════════════════════════
step "[10/10] Building ISO image..."

# Enable NetworkManager
sudo chroot "$ROOTFS" systemctl enable NetworkManager 2>/dev/null || true
# Enable SSH with password auth for debugging
sudo chroot "$ROOTFS" systemctl enable ssh 2>/dev/null || true
sudo sed -i 's/^#*PasswordAuthentication.*/PasswordAuthentication yes/' "$ROOTFS/etc/ssh/sshd_config"
sudo mkdir -p "$ROOTFS/etc/ssh/sshd_config.d"
echo -e "PasswordAuthentication yes\nPermitRootLogin yes" | sudo tee "$ROOTFS/etc/ssh/sshd_config.d/yantrik.conf" > /dev/null

# Load VM display drivers at boot (VBox vmwgfx, virtio-gpu, etc.)
echo -e "vmwgfx\nvirtio-gpu\ndrm" | sudo tee "$ROOTFS/etc/modules-load.d/yantrik-display.conf" > /dev/null

# Ensure live-config uses our yantrik user instead of creating "user"
sudo mkdir -p "$ROOTFS/etc/live/config.conf.d"
sudo tee "$ROOTFS/etc/live/config.conf.d/yantrik.conf" > /dev/null <<'LIVECONF'
LIVE_USERNAME="yantrik"
LIVE_USER_FULLNAME="Yantrik"
LIVE_USER_DEFAULT_GROUPS="audio video sudo input"
LIVE_NOCONFIGS="user-setup"
LIVECONF

# Clean up chroot
sudo chroot "$ROOTFS" apt-get clean
sudo rm -rf "$ROOTFS/tmp/"*
sudo rm -f "$ROOTFS/etc/resolv.conf"
echo "nameserver 8.8.8.8" | sudo tee "$ROOTFS/etc/resolv.conf" > /dev/null

# Store version
echo "$YANTRIK_VERSION" | sudo tee "$ROOTFS/opt/yantrik/.version" > /dev/null

# Unmount chroot filesystems
sudo umount "$ROOTFS/dev/pts" 2>/dev/null || true
sudo umount "$ROOTFS/dev" 2>/dev/null || true
sudo umount "$ROOTFS/proc" 2>/dev/null || true
sudo umount "$ROOTFS/sys" 2>/dev/null || true
trap - EXIT

# ── Copy kernel + initrd to ISO ──
VMLINUZ=$(ls "$ROOTFS/boot/vmlinuz-"* 2>/dev/null | head -1)
INITRD=$(ls "$ROOTFS/boot/initrd.img-"* 2>/dev/null | head -1)

if [ -z "$VMLINUZ" ] || [ -z "$INITRD" ]; then
    fail "Kernel or initrd not found in rootfs"
fi

sudo cp "$VMLINUZ" "$ISO_DIR/live/vmlinuz"
sudo cp "$INITRD" "$ISO_DIR/live/initrd"

# ── Create squashfs ──
info "Compressing rootfs (this takes a while)..."
sudo mksquashfs "$ROOTFS" "$ISO_DIR/live/filesystem.squashfs" \
    -comp xz -Xbcj x86 -noappend -quiet \
    -e "$ROOTFS/boot/vmlinuz-*" \
    -e "$ROOTFS/boot/initrd.img-*"

ok "Squashfs created ($(du -h "$ISO_DIR/live/filesystem.squashfs" | cut -f1))"

# ── GRUB config (BIOS + EFI) ──
sudo tee "$ISO_DIR/boot/grub/grub.cfg" > /dev/null <<'GRUBCFG'
set timeout=5
set default=0

insmod all_video
insmod gfxterm
set gfxmode=auto
terminal_output gfxterm

set menu_color_normal=cyan/black
set menu_color_highlight=white/blue

menuentry "Install Yantrik OS" {
    linux /live/vmlinuz boot=live yantrik.install=true live-config.username=yantrik live-config.user-fullname=yantrik console=tty1 console=ttyS0,115200 quiet
    initrd /live/initrd
}

menuentry "Yantrik OS — Live Desktop (Try without installing)" {
    linux /live/vmlinuz boot=live live-config.username=yantrik live-config.user-fullname=yantrik console=tty1 console=ttyS0,115200 quiet
    initrd /live/initrd
}

menuentry "Install Yantrik OS (Safe Mode — software rendering)" {
    linux /live/vmlinuz boot=live yantrik.install=true live-config.username=yantrik live-config.user-fullname=yantrik console=tty1 console=ttyS0,115200 nomodeset quiet
    initrd /live/initrd
}
GRUBCFG

# ── Build ISO with grub-mkrescue (BIOS + EFI hybrid) ──
info "Creating hybrid ISO (BIOS + EFI)..."
grub-mkrescue -o "$OUTPUT" "$ISO_DIR" \
    -- -volid "YANTRIK_OS" 2>/dev/null

ISO_SIZE=$(du -h "$OUTPUT" | cut -f1)

echo
echo -e "${CYAN}═══════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  ISO built: ${BOLD}$OUTPUT${NC} ${GREEN}($ISO_SIZE)${NC}"
echo -e "${CYAN}═══════════════════════════════════════════════════${NC}"
echo
echo -e "  ${BOLD}Test with QEMU:${NC}"
echo -e "    qemu-system-x86_64 -m 4G -cdrom $OUTPUT -boot d \\"
echo -e "      -enable-kvm -display gtk -device virtio-vga"
echo
echo -e "  ${BOLD}Test with VirtualBox:${NC}"
echo -e "    1. New VM → Linux/Debian 64-bit → 4GB RAM"
echo -e "    2. Storage → IDE → Add optical → Select $OUTPUT"
echo -e "    3. Start"
echo
echo -e "  ${BOLD}Write to USB:${NC}"
echo -e "    sudo dd if=$OUTPUT of=/dev/sdX bs=4M status=progress"
echo
echo -e "  ${DIM}All packages are baked in — no internet needed for installation.${NC}"
echo -e "  ${DIM}Updates can be checked via: yantrik-upgrade --check${NC}"
echo
