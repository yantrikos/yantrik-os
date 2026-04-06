#!/bin/bash
# Yantrik OS — Disk Installer
# Full install: disk partition, user creation, system copy, GRUB, post-config

set -euo pipefail

C='\033[0;36m'; G='\033[0;32m'; A='\033[0;33m'; R='\033[0;31m'; B='\033[1m'; N='\033[0m'
step() { echo -e "\n${C}::${N} ${B}$1${N}"; }
ok()   { echo -e "   ${G}✓${N} $1"; }

echo
echo -e "${C}╔═══════════════════════════════════════════════╗${N}"
echo -e "${C}║${N}  ${B}Yantrik OS${N} — Disk Installer                 ${C}║${N}"
echo -e "${C}║${N}  Your AI-native desktop, installed to disk.   ${C}║${N}"
echo -e "${C}╚═══════════════════════════════════════════════╝${N}"
echo

# ── 1. User setup ──
step "Create your account"
echo -n "  Full name: "; read -r FULLNAME
echo -n "  Username: "; read -r USERNAME
[ -z "$USERNAME" ] && USERNAME="yantrik"
echo -n "  Password: "; read -rs PASSWORD; echo
echo -n "  Confirm:  "; read -rs PASSWORD2; echo
if [ "$PASSWORD" != "$PASSWORD2" ]; then
    echo -e "${R}Passwords don't match. Aborting.${N}"; exit 1
fi
echo -n "  Hostname [yantrik]: "; read -r HOSTNAME
[ -z "$HOSTNAME" ] && HOSTNAME="yantrik"
ok "User: $USERNAME ($FULLNAME) @ $HOSTNAME"

# ── 2. Disk selection ──
step "Select installation disk"
echo -e "  ${B}Available disks:${N}"
lsblk -d -o NAME,SIZE,MODEL | grep -v loop | grep -v sr | sed 's/^/  /'
echo
echo -n "  Target disk (e.g., sda): "; read -r TARGET_DISK
[ -z "$TARGET_DISK" ] && { echo -e "${R}No disk specified.${N}"; exit 1; }
DISK="/dev/$TARGET_DISK"
[ -b "$DISK" ] || { echo -e "${R}$DISK is not a block device.${N}"; exit 1; }
echo
echo -e "  ${A}WARNING: ALL DATA on $DISK will be ERASED${N}"
echo -n "  Type 'yes' to continue: "; read -r CONFIRM
[ "$CONFIRM" = "yes" ] || exit 1

# ── 3. Partition ──
step "Partitioning $DISK (GPT)..."
IS_EFI=false; [ -d /sys/firmware/efi ] && IS_EFI=true
parted -s "$DISK" mklabel gpt
if $IS_EFI; then
    parted -s "$DISK" mkpart EFI fat32 1MiB 513MiB
    parted -s "$DISK" set 1 esp on
    parted -s "$DISK" mkpart root ext4 513MiB 100%
    partprobe "$DISK" 2>/dev/null; sleep 2
    EFI_PART="${DISK}1"; ROOT_PART="${DISK}2"
    [ -b "$EFI_PART" ] || EFI_PART="${DISK}p1"
    [ -b "$ROOT_PART" ] || ROOT_PART="${DISK}p2"
    mkfs.fat -F32 "$EFI_PART"
else
    parted -s "$DISK" mkpart biosboot "" 1MiB 2MiB
    parted -s "$DISK" set 1 bios_grub on
    parted -s "$DISK" mkpart root ext4 2MiB 100%
    partprobe "$DISK" 2>/dev/null; sleep 2
    EFI_PART=""; ROOT_PART="${DISK}2"
    [ -b "$ROOT_PART" ] || ROOT_PART="${DISK}p2"
fi
mkfs.ext4 -q -L YANTRIK "$ROOT_PART"
ok "Partitioned ($( $IS_EFI && echo 'EFI' || echo 'BIOS' ) mode)"

# ── 4. Mount ──
M="/mnt/yantrik-install"
mkdir -p "$M"
mount "$ROOT_PART" "$M"
if $IS_EFI && [ -n "$EFI_PART" ]; then
    mkdir -p "$M/boot/efi"
    mount "$EFI_PART" "$M/boot/efi"
fi

# ── 5. Copy system ──
step "Copying system files (this takes a few minutes)..."
rsync -aAXH --exclude='/proc/*' --exclude='/sys/*' --exclude='/dev/*' \
    --exclude='/run/*' --exclude='/tmp/*' --exclude='/mnt/*' \
    --exclude='/live/*' --exclude='/cdrom/*' \
    / "$M/" --info=progress2
ok "System copied"

# ── 6. Bind mounts for chroot ──
mount --bind /dev "$M/dev"
mount --bind /proc "$M/proc"
mount --bind /sys "$M/sys"

# ── 7. Remove live-boot (must happen AFTER bind mounts) ──
step "Configuring installed system..."
chroot "$M" apt-get remove -y --purge live-boot live-config live-config-systemd 2>/dev/null || true
chroot "$M" apt-get autoremove -y 2>/dev/null || true
ok "Live-boot removed"

# ── 8. Fstab ──
echo "LABEL=YANTRIK  /  ext4  defaults,noatime  0  1" > "$M/etc/fstab"
$IS_EFI && [ -n "$EFI_PART" ] && echo "$EFI_PART  /boot/efi  vfat  defaults  0  2" >> "$M/etc/fstab"

# ── 9. Hostname ──
echo "$HOSTNAME" > "$M/etc/hostname"
printf "127.0.0.1\tlocalhost\n127.0.1.1\t%s\n" "$HOSTNAME" > "$M/etc/hosts"

# ── 10. Create user ──
step "Creating user $USERNAME..."
if [ "$USERNAME" != "yantrik" ]; then
    chroot "$M" userdel -r yantrik 2>/dev/null || true
fi
chroot "$M" useradd -m -s /bin/bash -c "$FULLNAME" -G sudo,video,audio,input "$USERNAME" 2>/dev/null || true
HASH=$(openssl passwd -6 "$PASSWORD")
chroot "$M" usermod -p "$HASH" "$USERNAME"
echo "$USERNAME ALL=(ALL) NOPASSWD: ALL" > "$M/etc/sudoers.d/$USERNAME"
chmod 440 "$M/etc/sudoers.d/$USERNAME"
ok "User $USERNAME created"

# ── 11. Desktop config for new user ──
UHOME="$M/home/$USERNAME"
mkdir -p "$UHOME/.config/labwc" "$UHOME/.yantrik"

# labwc environment — use printf to avoid heredoc issues
printf "WLR_RENDERER_ALLOW_SOFTWARE=1\nWLR_NO_HARDWARE_CURSORS=1\nWLR_RENDERER=pixman\nXDG_SESSION_TYPE=wayland\nQT_QPA_PLATFORM=wayland\nMOZ_ENABLE_WAYLAND=1\nSLINT_BACKEND=winit\nLIBGL_ALWAYS_SOFTWARE=1\n" > "$UHOME/.config/labwc/environment"

# labwc autostart
printf '#!/bin/sh\nmako &\n/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml >> /opt/yantrik/logs/yantrik-os.log 2>&1 &\n' > "$UHOME/.config/labwc/autostart"
chmod +x "$UHOME/.config/labwc/autostart"

# labwc rc.xml (fullscreen, no decorations)
cp /home/yantrik/.config/labwc/rc.xml "$UHOME/.config/labwc/rc.xml" 2>/dev/null || true

# .bash_profile (auto-start labwc on tty1)
printf 'if [ "$(tty)" = "/dev/tty1" ] && [ -z "$WAYLAND_DISPLAY" ]; then\n    export XDG_RUNTIME_DIR="/run/user/$(id -u)"\n    mkdir -p "$XDG_RUNTIME_DIR"\n    if [ -f "$HOME/.config/labwc/environment" ]; then\n        set -a; . "$HOME/.config/labwc/environment"; set +a\n    fi\n    labwc 2>/opt/yantrik/logs/labwc.log\nfi\n' > "$UHOME/.bash_profile"

# Mark onboarding complete (boot to desktop, not wizard)
touch "$UHOME/.yantrik/.onboarding_complete"

# Fix ownership
UID_NUM=$(chroot "$M" id -u "$USERNAME" 2>/dev/null || echo 1000)
GID_NUM=$(chroot "$M" id -g "$USERNAME" 2>/dev/null || echo 1000)
chown -R "$UID_NUM:$GID_NUM" "$UHOME"
ok "Desktop configured"

# ── 12. Auto-login ──
mkdir -p "$M/etc/systemd/system/getty@tty1.service.d"
printf '[Service]\nExecStart=\nExecStart=-/sbin/agetty --autologin %s --noclear %%I $TERM\n' "$USERNAME" \
    > "$M/etc/systemd/system/getty@tty1.service.d/autologin.conf"
printf 'd /run/user/%s 0700 %s %s -\n' "$UID_NUM" "$USERNAME" "$USERNAME" \
    > "$M/etc/tmpfiles.d/yantrik-xdg.conf"

# ── 13. Update Yantrik config ──
sed -i "s/^user_name:.*/user_name: \"$USERNAME\"/" "$M/opt/yantrik/config.yaml"

# ── 14. OS branding ──
printf 'PRETTY_NAME="Yantrik OS"\nNAME="Yantrik OS"\nID=yantrik\nID_LIKE=debian\nVERSION_ID="0.3.0"\nHOME_URL="https://yantrikos.com"\n' > "$M/etc/os-release"

# ── 15. GRUB ──
step "Installing bootloader..."
printf 'GRUB_DEFAULT=0\nGRUB_TIMEOUT=3\nGRUB_DISTRIBUTOR="Yantrik OS"\nGRUB_CMDLINE_LINUX_DEFAULT="quiet splash"\nGRUB_CMDLINE_LINUX=""\n' > "$M/etc/default/grub"

if $IS_EFI; then
    chroot "$M" grub-install --target=x86_64-efi --efi-directory=/boot/efi \
        --bootloader-id=yantrik --no-nvram 2>/dev/null || true
else
    chroot "$M" grub-install --target=i386-pc "$DISK" 2>/dev/null || true
fi
chroot "$M" update-grub
ok "GRUB installed"

# ── 16. Regenerate initramfs (without live-boot hooks) ──
chroot "$M" update-initramfs -u 2>/dev/null || true

# ── 17. Cleanup ──
rm -f "$M/opt/yantrik/.installer-mode"
mkdir -p "$M/opt/yantrik/logs"; chmod 777 "$M/opt/yantrik/logs"
chown -R "$UID_NUM:$GID_NUM" "$M/opt/yantrik/data" 2>/dev/null || true

umount "$M/sys" "$M/proc" "$M/dev" 2>/dev/null || true
$IS_EFI && umount "$M/boot/efi" 2>/dev/null || true
umount "$M" 2>/dev/null || true
sync

echo
echo -e "${G}╔═══════════════════════════════════════════════╗${N}"
echo -e "${G}║  Installation complete!                       ║${N}"
echo -e "${G}║  Remove the installation media and reboot.    ║${N}"
echo -e "${G}╚═══════════════════════════════════════════════╝${N}"
echo
echo -n "Reboot now? [Y/n] "; read -r RB
[ "$RB" != "n" ] && reboot
