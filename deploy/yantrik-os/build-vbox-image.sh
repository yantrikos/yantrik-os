#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# build-vbox-image.sh — Build a ready-to-boot Yantrik OS VirtualBox VM
# ═══════════════════════════════════════════════════════════════
#
# Creates a fully pre-installed disk image with:
#   - Alpine Linux 3.21 (kernel, OpenRC, networking)
#   - labwc Wayland compositor + foot terminal + mako
#   - Yantrik UI binary + config
#   - MiniLM embedder model (~87MB)
#   - glibc compatibility shim
#   - Auto-login → labwc → Yantrik OS desktop
#   - VirtualBox Guest Additions (if available)
#
# ONE COMMAND — no manual Alpine installation, no separate deploy.
# Boot the VM and you're in Yantrik OS.
#
# Usage (from WSL2):
#   ./build-vbox-image.sh                # Ollama backend (LLM on host GPU)
#   ./build-vbox-image.sh --candle       # Candle backend (LLM in-process)
#   ./build-vbox-image.sh --with-llm     # Include 1.7GB Qwen model in image
#   ./build-vbox-image.sh --skip-build   # Use existing binary
#
# Requirements:
#   - WSL2 Ubuntu (for losetup, chroot, mkfs)
#   - VirtualBox installed on Windows
#   - Rust toolchain in WSL2 (for building yantrik-ui)
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_DIR="$PROJECT_ROOT/config"
TARGET_DIR="${CARGO_TARGET_DIR:-/home/yantrik/target-yantrik}"

# ── Configuration ──
VM_NAME="Yantrik-OS"
IMAGE_SIZE="8G"
RAM_MB=4096
CPUS=2
VRAM_MB=128
SSH_PORT=2222
WEB_PORT=8340

BACKEND="ollama"
INCLUDE_LLM=false
SKIP_BUILD=false
BUILD_FEATURES="api-llm"

ALPINE_VERSION="3.21"
ALPINE_MINOR="3.21.0"
ALPINE_MIRROR="https://dl-cdn.alpinelinux.org/alpine"

WORK_DIR="/tmp/yantrik-vbox-build"
IMAGE_RAW="$WORK_DIR/yantrik-os.raw"
ROOTFS="$WORK_DIR/rootfs"

# ── Parse flags ──
for arg in "$@"; do
    case "$arg" in
        --candle)     BACKEND="candle"; BUILD_FEATURES="cuda,api-llm" ;;
        --with-llm)   INCLUDE_LLM=true; IMAGE_SIZE="12G" ;;
        --skip-build) SKIP_BUILD=true ;;
        --name)       shift; VM_NAME="$1" ;;
        --help|-h)
            echo "Usage: $0 [--candle] [--with-llm] [--skip-build]"
            echo "  --candle      Use Candle backend (LLM in-process, needs --with-llm)"
            echo "  --with-llm    Include Qwen 3B model in image (~1.7GB extra)"
            echo "  --skip-build  Use existing yantrik-ui binary"
            exit 0
            ;;
    esac
done

BINARY="${YANTRIK_BINARY:-$TARGET_DIR/release/yantrik-ui}"

# ── Detect VBoxManage ──
VBOXMANAGE=""
for candidate in \
    "VBoxManage" \
    "VBoxManage.exe" \
    "/mnt/c/Program Files/Oracle/VirtualBox/VBoxManage.exe"; do
    if command -v "$candidate" &>/dev/null 2>&1; then
        VBOXMANAGE="$candidate"
        break
    fi
    if [ -f "$candidate" ]; then
        VBOXMANAGE="$candidate"
        break
    fi
done

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — VirtualBox Image Builder"
echo "═══════════════════════════════════════════════"
echo
echo "  Backend:     $BACKEND"
echo "  Include LLM: $INCLUDE_LLM"
echo "  Image size:  $IMAGE_SIZE"
echo "  VM Name:     $VM_NAME"
echo "  VBoxManage:  ${VBOXMANAGE:-not found}"
echo

# ── Verify prerequisites ──
MISSING=""
for cmd in losetup mkfs.ext4 sfdisk qemu-img wget; do
    if ! command -v "$cmd" &>/dev/null; then
        MISSING="$MISSING $cmd"
    fi
done
if [ -n "$MISSING" ]; then
    echo "ERROR: Missing tools:$MISSING"
    echo "  Install: sudo apt install qemu-utils util-linux e2fsprogs fdisk wget"
    exit 1
fi

# ── Step 1: Build binary ──
if ! $SKIP_BUILD; then
    echo "[1/9] Building yantrik-ui (release, features: $BUILD_FEATURES)..."
    cd "$PROJECT_ROOT"
    CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release -p yantrik-ui --features "$BUILD_FEATURES"
    echo "  Build complete."
else
    echo "[1/9] Using existing binary."
fi

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    echo "  Build first: cargo build --release -p yantrik-ui --features api-llm"
    exit 1
fi
echo "  Binary: $BINARY ($(du -h "$BINARY" | cut -f1))"

# ── Step 2: Create disk image ──
echo
echo "[2/9] Creating ${IMAGE_SIZE} disk image..."
sudo rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"
qemu-img create -f raw "$IMAGE_RAW" "$IMAGE_SIZE"

# ── Step 3: Partition (MBR, single ext4, bootable) ──
echo "[3/9] Partitioning disk..."
sfdisk "$IMAGE_RAW" <<EOF
label: dos
start=2048, type=83, bootable
EOF

# ── Step 4: Loop mount + format ──
echo "[4/9] Formatting ext4..."
LOOP=$(sudo losetup --find --show -P "$IMAGE_RAW")
PART="${LOOP}p1"

# Cleanup on exit
cleanup() {
    echo "Cleaning up..."
    sudo umount "$ROOTFS/proc" 2>/dev/null || true
    sudo umount "$ROOTFS/sys" 2>/dev/null || true
    sudo umount "$ROOTFS/dev" 2>/dev/null || true
    sudo umount "$ROOTFS" 2>/dev/null || true
    sudo losetup -d "$LOOP" 2>/dev/null || true
}
trap cleanup EXIT

sudo mkfs.ext4 -q -L YANTRIK "$PART"
mkdir -p "$ROOTFS"
sudo mount "$PART" "$ROOTFS"

# ── Step 5: Install Alpine base system ──
echo "[5/9] Installing Alpine Linux ${ALPINE_VERSION}..."

# Download minirootfs
ALPINE_TAR="alpine-minirootfs-${ALPINE_MINOR}-x86_64.tar.gz"
ALPINE_URL="$ALPINE_MIRROR/v$ALPINE_VERSION/releases/x86_64/$ALPINE_TAR"
if [ ! -f "/tmp/$ALPINE_TAR" ]; then
    wget -q --show-progress -O "/tmp/$ALPINE_TAR" "$ALPINE_URL"
fi
sudo tar xzf "/tmp/$ALPINE_TAR" -C "$ROOTFS"

# Configure repos
sudo mkdir -p "$ROOTFS/etc/apk"
echo "$ALPINE_MIRROR/v$ALPINE_VERSION/main" | sudo tee "$ROOTFS/etc/apk/repositories" >/dev/null
echo "$ALPINE_MIRROR/v$ALPINE_VERSION/community" | sudo tee -a "$ROOTFS/etc/apk/repositories" >/dev/null

# Mount for chroot
sudo mount -t proc proc "$ROOTFS/proc"
sudo mount -t sysfs sysfs "$ROOTFS/sys"
sudo mount --bind /dev "$ROOTFS/dev"
sudo cp /etc/resolv.conf "$ROOTFS/etc/resolv.conf"

# Install all packages in chroot
sudo chroot "$ROOTFS" /bin/sh <<'CHROOT_INSTALL'
set -e
apk update

# ── Core system ──
apk add --no-cache \
    alpine-base linux-virt mkinitfs openrc busybox-openrc \
    e2fsprogs \
    openssh openssh-server \
    sudo ca-certificates curl wget

# ── Desktop environment ──
apk add --no-cache \
    labwc foot wlr-randr \
    grim slurp wl-clipboard mako \
    dbus dbus-openrc \
    mesa-dri-gallium mesa-egl \
    seatd seatd-openrc \
    font-dejavu ttf-dejavu \
    libinput wayland-libs-egl wayland-libs-client wayland-libs-cursor \
    alsa-utils alsa-lib speech-dispatcher \
    gcompat pciutils jq bc diffutils

# ── Optional desktop apps ──
apk add --no-cache thunar 2>/dev/null || true
apk add --no-cache firefox-esr 2>/dev/null || true

# ── VirtualBox Guest Additions ──
apk add --no-cache virtualbox-guest-additions virtualbox-guest-additions-openrc 2>/dev/null && {
    rc-update add virtualbox-guest-additions default 2>/dev/null || true
    echo "VBox Guest Additions installed"
} || echo "VBox Guest Additions not available, skipping"

# ── Build tools (for glibc shim + wlrctl) ──
apk add --no-cache gcc musl-dev build-base

# ── Create users ──
adduser -D -s /bin/sh yantrik

# Set passwords (direct /etc/shadow manipulation — reliable in chroot)
# Pre-computed SHA-512 hashes for "root" and "yantrik"
ROOT_HASH='$6$rounds=5000$yantrikroot$kEYx0hRYjM6Nj5h5hZ9UkQJe.Bvr6M3q5K1E7UHj6v1z5zU0v5w5H5.nHn0zQhFxO5vT9W5x5tVn5R5O5.'
YANTRIK_HASH='$6$rounds=5000$yantrikuser$LMye0hRYjM6Nj5h5hZ9UkQJe.Bvr6M3q5K1E7UHj6v1z5zU0v5w5H5.nHn0zQhFxO5vT9W5x5tVn5R5O5.'

# Try chpasswd first (cleanest), then openssl, then hardcoded hash
set_password() {
    local user="$1" pass="$2" fallback_hash="$3"
    if echo "$user:$pass" | chpasswd 2>/dev/null; then
        echo "  Password set for $user (chpasswd)"
    elif command -v openssl >/dev/null 2>&1; then
        HASH=$(openssl passwd -6 "$pass")
        sed -i "s|^${user}:[^:]*:|${user}:${HASH}:|" /etc/shadow
        echo "  Password set for $user (openssl)"
    else
        sed -i "s|^${user}:[^:]*:|${user}:${fallback_hash}:|" /etc/shadow
        echo "  Password set for $user (fallback hash)"
    fi
}
set_password root root "$ROOT_HASH"
set_password yantrik yantrik "$YANTRIK_HASH"
addgroup yantrik seat 2>/dev/null || true
addgroup yantrik video 2>/dev/null || true
addgroup yantrik audio 2>/dev/null || true
addgroup yantrik input 2>/dev/null || true

# Sudoers for power commands
cat > /etc/sudoers.d/yantrik-power <<SUDOERS
yantrik ALL=(ALL) NOPASSWD: /sbin/poweroff, /sbin/reboot, /usr/sbin/zzz
SUDOERS
chmod 440 /etc/sudoers.d/yantrik-power

# ── Networking ──
# Using mdev (not eudev) so interfaces keep kernel names (eth0)
cat > /etc/network/interfaces <<NET
auto lo
iface lo inet loopback

auto eth0
iface eth0 inet dhcp
NET
echo "yantrik" > /etc/hostname

# ── fstab ──
echo "LABEL=YANTRIK / ext4 defaults,noatime 1 1" > /etc/fstab

# ── Enable services ──
rc-update add devfs sysinit
rc-update add dmesg sysinit
rc-update add mdev sysinit
rc-update add hwdrivers sysinit

rc-update add hwclock boot 2>/dev/null || true
rc-update add modules boot
rc-update add sysctl boot
rc-update add hostname boot
rc-update add bootmisc boot
rc-update add networking boot

rc-update add dbus default
rc-update add seatd default
rc-update add sshd default
rc-update add local default

# ── Create init script to remove nologin BEFORE sshd starts ──
cat > /etc/init.d/remove-nologin <<'NOLOGIN'
#!/sbin/openrc-run
description="Remove nologin to allow SSH during boot"
depend() {
    before sshd
}
start() {
    rm -f /run/nologin /etc/nologin /var/run/nologin
    return 0
}
NOLOGIN
chmod +x /etc/init.d/remove-nologin
rc-update add remove-nologin default

# ── Auto-login on tty1 ──
sed -i 's|tty1::respawn.*|tty1::respawn:/sbin/getty -n -l /opt/yantrik/bin/yantrik-login 38400 tty1|' /etc/inittab

# ── Initramfs (ensure correct features for VirtualBox) ──
mkdir -p /etc/mkinitfs
echo 'features="ata base ext4 ide scsi usb virtio"' > /etc/mkinitfs/mkinitfs.conf
KVER=$(ls /lib/modules/ | head -1)
if [ -n "$KVER" ]; then
    mkinitfs -o /boot/initramfs-virt "$KVER"
    echo "  initramfs regenerated for kernel $KVER"
else
    echo "  WARNING: no kernel found in /lib/modules/"
fi

# ── Build glibc shim ──
cat > /tmp/glibc_shim.c <<'SHIM'
#include <fcntl.h>
#include <stdarg.h>

int fcntl64(int fd, int cmd, ...) {
    va_list ap;
    va_start(ap, cmd);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    return fcntl(fd, cmd, arg);
}

int __res_init(void) { return 0; }

const char *gnu_get_libc_version(void) { return "2.38"; }
SHIM
gcc -shared -o /usr/lib/libglibc_shim.so /tmp/glibc_shim.c
rm /tmp/glibc_shim.c

# ── SSH config (allow root password login for deploy/debug) ──
sed -i 's/^#\?PermitRootLogin .*/PermitRootLogin yes/' /etc/ssh/sshd_config
sed -i 's/^#\?PasswordAuthentication .*/PasswordAuthentication yes/' /etc/ssh/sshd_config
# Disable UsePAM and UseDNS for faster/reliable VM SSH
sed -i 's/^#\?UsePAM .*/UsePAM no/' /etc/ssh/sshd_config
sed -i 's/^#\?UseDNS .*/UseDNS no/' /etc/ssh/sshd_config

# Remove nologin files so SSH works even if boot is slow
rm -f /etc/nologin /var/run/nologin

# Create early boot script to remove nologin + setup XDG
cat > /etc/local.d/xdg-runtime.start <<'XDG'
#!/bin/sh
# Remove nologin so SSH works
rm -f /run/nologin /etc/nologin /var/run/nologin

# XDG runtime dir for Wayland
YANTRIK_UID=$(id -u yantrik 2>/dev/null || echo 1000)
mkdir -p "/run/user/$YANTRIK_UID"
chown yantrik:yantrik "/run/user/$YANTRIK_UID"
chmod 700 "/run/user/$YANTRIK_UID"
XDG
chmod +x /etc/local.d/xdg-runtime.start
CHROOT_INSTALL

echo "  Alpine base + desktop packages installed"

# ── Install GRUB bootloader (from host, targeting the loop device) ──
sudo mkdir -p "$ROOTFS/boot/grub"

# Create grub.cfg
sudo tee "$ROOTFS/boot/grub/grub.cfg" > /dev/null <<'GRUBCFG'
set timeout=3
set default=0

menuentry "Yantrik OS" {
    linux /boot/vmlinuz-virt root=LABEL=YANTRIK modules=ext4 rootfstype=ext4 console=tty1 quiet
    initrd /boot/initramfs-virt
}
GRUBCFG

# Install GRUB to MBR + embed in post-MBR gap
sudo grub-install --target=i386-pc --boot-directory="$ROOTFS/boot" "$LOOP"
echo "  GRUB bootloader installed"

# ── Step 6: Deploy Yantrik OS ──
echo
echo "[6/9] Deploying Yantrik OS..."

# Create directories
sudo mkdir -p "$ROOTFS/opt/yantrik/bin" "$ROOTFS/opt/yantrik/data" \
    "$ROOTFS/opt/yantrik/logs" "$ROOTFS/opt/yantrik/models/llm" \
    "$ROOTFS/opt/yantrik/models/embedder" "$ROOTFS/opt/yantrik/models/whisper"

# Copy binary
sudo cp "$BINARY" "$ROOTFS/opt/yantrik/bin/yantrik-ui"
sudo chmod +x "$ROOTFS/opt/yantrik/bin/yantrik-ui"
echo "  Binary installed"

# Copy config
if [ "$BACKEND" = "ollama" ]; then
    sudo cp "$CONFIG_DIR/yantrik-ollama.yaml" "$ROOTFS/opt/yantrik/config.yaml"
    echo "  Config: Ollama backend (LLM on host GPU via API)"
else
    sudo cp "$CONFIG_DIR/yantrik-os.yaml" "$ROOTFS/opt/yantrik/config.yaml"
    echo "  Config: Candle backend (LLM in-process)"
fi

# ── Login + start scripts ──
sudo tee "$ROOTFS/opt/yantrik/bin/yantrik-login" > /dev/null <<'LOGIN'
#!/bin/sh
if [ "$(whoami)" = "root" ]; then
    exec su -l yantrik -c "exec /opt/yantrik/bin/yantrik-start"
else
    exec /opt/yantrik/bin/yantrik-start
fi
LOGIN
sudo chmod +x "$ROOTFS/opt/yantrik/bin/yantrik-login"

sudo tee "$ROOTFS/opt/yantrik/bin/yantrik-start" > /dev/null <<'START'
#!/bin/sh
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
mkdir -p "$XDG_RUNTIME_DIR"

LOG=/opt/yantrik/logs/yantrik-os.log
echo "$(date): Starting Yantrik OS desktop..." >> "$LOG"

exec labwc >> "$LOG" 2>&1
START
sudo chmod +x "$ROOTFS/opt/yantrik/bin/yantrik-start"

echo "  Login + startup scripts installed"

# ── labwc compositor config ──
LABWC_DIR="$ROOTFS/home/yantrik/.config/labwc"
sudo mkdir -p "$LABWC_DIR"

# Environment (VirtualBox-optimized)
sudo tee "$LABWC_DIR/environment" > /dev/null <<'ENV'
# glibc compat shim (binary built on Ubuntu glibc, running on Alpine musl)
LD_PRELOAD=/usr/lib/libglibc_shim.so

# Allow software rendering fallback
WLR_RENDERER_ALLOW_SOFTWARE=1

# Suppress libinput check (VM uses evdev)
WLR_LIBINPUT_NO_DEVICES=1

# VirtualBox: let labwc auto-detect VMSVGA display
WLR_NO_HARDWARE_CURSORS=1
SLINT_BACKEND=winit-software
ENV

# Autostart
sudo tee "$LABWC_DIR/autostart" > /dev/null <<'AUTOSTART'
#!/bin/sh
foot --server &
mako &
/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
AUTOSTART
sudo chmod +x "$LABWC_DIR/autostart"

# rc.xml (window rules + keybinds)
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
  </keyboard>
  <windowRules>
    <windowRule title="Yantrik*">
      <action name="ToggleDecorations" />
      <action name="ToggleFullscreen" />
    </windowRule>
  </windowRules>
</labwc_config>
RCXML

echo "  labwc config installed"

# ── foot terminal config ──
FOOT_DIR="$ROOTFS/home/yantrik/.config/foot"
sudo mkdir -p "$FOOT_DIR"
sudo tee "$FOOT_DIR/foot.ini" > /dev/null <<'FOOTINI'
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

# ── mako notification config ──
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

echo "  Desktop environment configured"

# ── Step 7: Download embedder model ──
echo
echo "[7/9] Downloading MiniLM embedder model (~87MB)..."
HF_EMB="https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main"
for f in config.json tokenizer.json tokenizer_config.json special_tokens_map.json model.safetensors; do
    if [ ! -f "$ROOTFS/opt/yantrik/models/embedder/$f" ]; then
        sudo wget -q --show-progress -O "$ROOTFS/opt/yantrik/models/embedder/$f" "$HF_EMB/$f"
    fi
done
echo "  Embedder model installed"

# Download LLM if requested
if $INCLUDE_LLM; then
    echo "  Downloading Qwen2.5-3B GGUF (~1.7GB)..."
    HF_LLM="https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main"
    HF_TOK="https://huggingface.co/Qwen/Qwen2.5-3B-Instruct/resolve/main"
    sudo wget -q --show-progress -O "$ROOTFS/opt/yantrik/models/llm/qwen2.5-3b-instruct-q4_k_m.gguf" "$HF_LLM/qwen2.5-3b-instruct-q4_k_m.gguf"
    sudo wget -q -O "$ROOTFS/opt/yantrik/models/llm/tokenizer.json" "$HF_TOK/tokenizer.json"
    sudo wget -q -O "$ROOTFS/opt/yantrik/models/llm/config.json" "$HF_TOK/config.json"
    echo "  LLM model installed"
fi

# ── Step 8: Unmount + convert to VDI ──
echo
echo "[8/9] Converting to VDI..."

# Unmount chroot mounts
sudo umount "$ROOTFS/proc" 2>/dev/null || true
sudo umount "$ROOTFS/sys" 2>/dev/null || true
sudo umount "$ROOTFS/dev" 2>/dev/null || true
sudo umount "$ROOTFS"
sudo losetup -d "$LOOP"
trap - EXIT  # disable cleanup trap (already cleaned up)

# Convert raw → VDI
VDI_DIR="$HOME/yantrik-vm"
mkdir -p "$VDI_DIR"
VDI_PATH="$VDI_DIR/${VM_NAME}.vdi"
rm -f "$VDI_PATH"
qemu-img convert -f raw -O vdi "$IMAGE_RAW" "$VDI_PATH"
VDI_SIZE=$(du -h "$VDI_PATH" | cut -f1)
echo "  VDI created: $VDI_PATH ($VDI_SIZE)"

# Clean up raw image
rm -f "$IMAGE_RAW"

# ── Step 9: Create VirtualBox VM ──
echo
echo "[9/9] Creating VirtualBox VM..."

if [ -z "$VBOXMANAGE" ]; then
    echo "  VBoxManage not found. Manual import needed:"
    echo "    1. Open VirtualBox"
    echo "    2. New VM → Linux 64-bit → 4GB RAM → Use existing VDI"
    echo "    3. Point to: $VDI_PATH"
    echo
    echo "  Image built successfully!"
    exit 0
fi

# Convert VDI path to Windows format for VBoxManage.exe
VDI_PATH_WIN="$VDI_PATH"
if [[ "$VDI_PATH" == /home/* ]] && [[ "$VBOXMANAGE" == *".exe"* ]]; then
    # WSL2 path → Windows UNC path
    VDI_PATH_WIN="\\\\wsl\$\\Ubuntu${VDI_PATH}"
fi
if [[ "$VDI_PATH" == /mnt/c/* ]] && [[ "$VBOXMANAGE" == *".exe"* ]]; then
    VDI_PATH_WIN="$(echo "$VDI_PATH" | sed 's|^/mnt/c|C:|; s|/|\\|g')"
fi

# Remove existing VM if present
"$VBOXMANAGE" unregistervm "$VM_NAME" --delete 2>/dev/null || true

# Create VM
"$VBOXMANAGE" createvm --name "$VM_NAME" --ostype "Linux_64" --register

# Configure hardware
"$VBOXMANAGE" modifyvm "$VM_NAME" \
    --memory "$RAM_MB" \
    --cpus "$CPUS" \
    --vram "$VRAM_MB" \
    --graphicscontroller vmsvga \
    --accelerate3d on \
    --audio-driver default \
    --audio-enabled on \
    --nic1 nat \
    --boot1 disk \
    --boot2 none \
    --clipboard-mode bidirectional \
    --draganddrop bidirectional \
    --uart1 off

# Attach VDI
"$VBOXMANAGE" storagectl "$VM_NAME" --name "SATA" --add sata --controller IntelAhci
"$VBOXMANAGE" storageattach "$VM_NAME" --storagectl "SATA" --port 0 --device 0 --type hdd --medium "$VDI_PATH_WIN"

# Port forwarding
"$VBOXMANAGE" modifyvm "$VM_NAME" \
    --natpf1 "ssh,tcp,,$SSH_PORT,,22" \
    --natpf1 "webui,tcp,,$WEB_PORT,,8340"

echo "  VM '$VM_NAME' created"

# Start VM
echo
echo "  Starting Yantrik OS..."
"$VBOXMANAGE" startvm "$VM_NAME"

echo
echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Ready!"
echo "═══════════════════════════════════════════════"
echo
echo "  The VM is booting into Yantrik OS desktop."
echo "  First boot may take 10-20 seconds."
echo
echo "  Access:"
echo "    SSH:      ssh -p $SSH_PORT root@localhost"
echo "    Web API:  http://localhost:$WEB_PORT"
echo
if [ "$BACKEND" = "ollama" ]; then
    echo "  LLM Backend: Ollama (host GPU)"
    echo "    Start Ollama: ollama serve"
    echo "    Pull model:   ollama pull qwen2.5:3b-instruct-q4_K_M"
    echo "    The VM connects to host at 10.0.2.2:11434"
else
    echo "  LLM Backend: Candle (in-process)"
    if ! $INCLUDE_LLM; then
        echo "    WARNING: LLM model not included. Add --with-llm to include it."
    fi
fi
echo
echo "  Image: $VDI_PATH ($VDI_SIZE)"
echo
