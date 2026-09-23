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
echo -n "  Full name (used for git commits): "; read -r FULLNAME
echo -n "  Username: "; read -r USERNAME
[ -z "$USERNAME" ] && USERNAME="yantrik"

# Re-prompt rather than abort. Throwing away every answer over one mistyped password
# is a punishment for a typo, and it is the last thing anyone wants from an installer.
while :; do
    echo -n "  Password (for login and sudo): "; read -rs PASSWORD; echo
    echo -n "  Confirm:  "; read -rs PASSWORD2; echo
    [ -n "$PASSWORD" ] || { echo -e "  ${A}Password cannot be empty.${N}"; continue; }
    [ "$PASSWORD" = "$PASSWORD2" ] && break
    echo -e "  ${A}Those did not match. Try again.${N}"
done

echo -n "  Hostname [yantrik]: "; read -r HOSTNAME
[ -z "$HOSTNAME" ] && HOSTNAME="yantrik"

# ── 1b. Timezone, guessed from the network ──
# Asking someone to scroll four hundred zones when the network already knows is work
# we can do for them. If there is no network, or the answer looks wrong, this is still
# one keystroke to accept and one line to override.
TZ_GUESS=""
if command -v curl >/dev/null 2>&1; then
    TZ_GUESS=$(curl -fsS -m 5 http://ip-api.com/line/?fields=timezone 2>/dev/null | head -1)
fi
case "$TZ_GUESS" in
    */*) : ;;
    *) TZ_GUESS="" ;;
esac
[ -n "$TZ_GUESS" ] && [ ! -f "/usr/share/zoneinfo/$TZ_GUESS" ] && TZ_GUESS=""
[ -z "$TZ_GUESS" ] && TZ_GUESS="UTC"
echo -n "  Timezone [$TZ_GUESS]: "; read -r TIMEZONE
[ -z "$TIMEZONE" ] && TIMEZONE="$TZ_GUESS"
if [ ! -f "/usr/share/zoneinfo/$TIMEZONE" ]; then
    echo -e "  ${A}Unknown timezone '$TIMEZONE' — using $TZ_GUESS.${N}"
    TIMEZONE="$TZ_GUESS"
fi

ok "User: $USERNAME ($FULLNAME) @ $HOSTNAME, $TIMEZONE"

# ── 2. Disk selection ──
step "Select installation disk"
echo -e "  ${B}Available disks:${N}"
lsblk -d -o NAME,SIZE,MODEL | grep -v loop | grep -v sr | sed 's/^/  /'
echo
echo -n "  Target disk (e.g., sda): "; read -r TARGET_DISK
[ -z "$TARGET_DISK" ] && { echo -e "${R}No disk specified.${N}"; exit 1; }
DISK="/dev/$TARGET_DISK"
[ -b "$DISK" ] || { echo -e "${R}$DISK is not a block device.${N}"; exit 1; }

# ── 2b. Everything, once, before anything is destroyed ──
# The disk was the only thing confirmed before this, so a mistyped username was
# discovered after the install rather than before it.
echo
step "Does this look right?"
printf "  %-12s %s\n" "Username"  "$USERNAME"
printf "  %-12s %s\n" "Full name" "${FULLNAME:-[skipped]}"
printf "  %-12s %s\n" "Hostname"  "$HOSTNAME"
printf "  %-12s %s\n" "Timezone"  "$TIMEZONE"
printf "  %-12s %s\n" "Password"  "$(printf '%*s' "${#PASSWORD}" '' | tr ' ' '*')"
printf "  %-12s %s\n" "Disk"      "$DISK ($(lsblk -dno SIZE "$DISK" 2>/dev/null | tr -d ' '))"
echo
echo -e "  ${A}Everything on $DISK will be erased. There is no recovery.${N}"
echo -n "  Type 'yes' to install: "; read -r CONFIRM
[ "$CONFIRM" = "yes" ] || { echo "  Nothing was changed."; exit 1; }

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

# Apply the timezone that was asked for. Collecting an answer and then ignoring it is
# the same defect as an action reporting success it never performed.
if [ -n "${TIMEZONE:-}" ] && [ -f "$M/usr/share/zoneinfo/$TIMEZONE" ]; then
    ln -sf "/usr/share/zoneinfo/$TIMEZONE" "$M/etc/localtime"
    echo "$TIMEZONE" > "$M/etc/timezone"
    chroot "$M" dpkg-reconfigure -f noninteractive tzdata >/dev/null 2>&1 || true
    ok "Timezone: $TIMEZONE"
fi
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
#
# The session is `yantrik-session`, the same program the live image and every cloud-init
# machine start. This used to hand-write an autostart and then run bare `labwc`, which meant
# an installed machine was the ONE kind of Yantrik machine that did not run the shipped
# session: no XDG_DATA_DIRS entry for /opt/yantrik/share, so the launcher's "all applications"
# listed Chromium and Vim and none of this OS's own fourteen apps; no shipped rc.xml, so the
# shell drew inside a titlebar; no fonts installed for fontconfig, so the compositor drew its
# chrome in a typeface this OS does not use. Every one of those was fixed in yantrik-session
# and the fix reached everything except the machines people actually install.
UHOME="$M/home/$USERNAME"
mkdir -p "$UHOME/.config/labwc" "$UHOME/.yantrik"

# The environment file carries no renderer. It used to force WLR_RENDERER=pixman and
# LIBGL_ALWAYS_SOFTWARE=1 on every machine, GPU or not; yantrik-session decides at every login
# now, and falls back to software by itself when a GPU fails in use. The first line is the
# session's mark (GRAPHICS_ENV_MARK in yantrik-session): without it the session would take this
# file for an older image's.
printf '%s\n' \
    "# yantrik-graphics: yantrik-session decides the renderer" \
    "# To force one, add a line: YANTRIK_GRAPHICS=software or YANTRIK_GRAPHICS=gpu." \
    "# \`yantrik-session graphics\` says what the session would choose and why." \
    WLR_NO_HARDWARE_CURSORS=1 XDG_SESSION_TYPE=wayland QT_QPA_PLATFORM=wayland MOZ_ENABLE_WAYLAND=1 \
    > "$UHOME/.config/labwc/environment"

# No autostart and no rc.xml written here. yantrik-session copies the shipped ones out of
# /opt/yantrik/share at every login, so a machine installed today picks up a theme fix
# published tomorrow by rebooting. Writing them here would shadow that permanently.

# .bash_profile — the same one the live image uses, minus the installer-mode branch.
# It does not source the labwc environment: labwc reads that itself, and sourcing it is how the
# old software lines reached everything the session started.
printf 'if [ "$(tty)" = "/dev/tty1" ] && [ -z "$WAYLAND_DISPLAY" ]; then\n    export XDG_RUNTIME_DIR="/run/user/$(id -u)"\n    mkdir -p "$XDG_RUNTIME_DIR"\n    /opt/yantrik/bin/yantrik-session 2>>/opt/yantrik/logs/labwc.log\nfi\n' > "$UHOME/.bash_profile"

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

# ── 13b. The update channel, stated rather than assumed ──
#
# The rsync above copies the live image's /opt/yantrik/update.conf onto the disk, and the ISO
# build writes one — so in the normal case this changes nothing. It exists for the case where
# it is missing: without update.conf the updater falls back to the channel recorded in BUILD
# and then to its own hard default, and a machine that silently guesses which software it will
# install is exactly the thing this whole path was untangled to stop. If it is not there, write
# it, deriving the channel from the build that was just installed.
#
# Owned by the desktop user, because the About screen's channel picker writes this file through
# `yantrik-update set-channel` as that user. A root-owned update.conf makes the picker inert —
# it says so rather than failing, but an inert control is still a control nobody can use.
UPDATE_CONF="$M/opt/yantrik/update.conf"
if [ ! -f "$UPDATE_CONF" ]; then
    INSTALL_CHANNEL=$(sed -n 's/^channel=//p' "$M/opt/yantrik/BUILD" 2>/dev/null | head -1)
    [ -n "$INSTALL_CHANNEL" ] || INSTALL_CHANNEL="nightly"
    printf '%s\n' \
        "# Read by yantrik-update, and by nothing else. This file is the single owner of which" \
        "# channel this machine follows, which server it follows it on, and over which scheme." \
        "#" \
        "# Change it with: yantrik-update set-channel nightly|beta|stable" \
        "# or from the desktop: About -> UPDATES -> the channel chips." \
        "CHANNEL=$INSTALL_CHANNEL" \
        "HOST=releases.yantrikos.com" \
        "SCHEME=https" > "$UPDATE_CONF"
    ok "Update channel: $INSTALL_CHANNEL (update.conf was missing from the image)"
fi

# ── 14. OS branding ──
# The version comes from the build that is being installed, not from a literal written here.
# "0.3.0" was hardcoded for five months, so `cat /etc/os-release` on any installed machine
# named a version that had not been true since spring — and that file is the first thing
# anyone reads off a machine they are asked to debug.
VERSION_ID=$(sed -n 's/^version=//p' "$M/opt/yantrik/BUILD" 2>/dev/null | head -1)
[ -n "$VERSION_ID" ] || VERSION_ID="unknown"
printf 'PRETTY_NAME="Yantrik OS"\nNAME="Yantrik OS"\nID=yantrik\nID_LIKE=debian\nVERSION_ID="%s"\nHOME_URL="https://yantrikos.com"\n' "$VERSION_ID" > "$M/etc/os-release"

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
# The desktop user owns its own logs. `chmod 777` made this world-writable on every installed
# machine: any process any user runs could rewrite the log that says what the OS did.
mkdir -p "$M/opt/yantrik/logs"
chmod 755 "$M/opt/yantrik/logs"
chown "$UID_NUM:$GID_NUM" "$M/opt/yantrik/logs"
chown -R "$UID_NUM:$GID_NUM" "$M/opt/yantrik/data" 2>/dev/null || true
# The installer can rename the desktop user, and a renamed user can land on a different uid
# than the 1000 the image chowned /opt/yantrik to. update.conf and BUILD are the two files the
# updater writes as that user — set-channel writes the first, apply writes the second — so they
# follow the account that actually exists on this machine.
chown "$UID_NUM:$GID_NUM" "$M/opt/yantrik/update.conf" 2>/dev/null || true
chown "$UID_NUM:$GID_NUM" "$M/opt/yantrik/BUILD" 2>/dev/null || true

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
