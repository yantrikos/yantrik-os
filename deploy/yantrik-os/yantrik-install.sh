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

# ── 2a. Beside what is there, if anything can take it ──
# Installing into one partition or run of free space changes nothing else on the disk: no new
# partition table, the EFI partition mounted and never formatted. On a Mac whose APFS container
# was shrunk in Disk Utility, that is the FAT32 partition named YANTRIK, or the free space. The
# planner (yantrik-install-target, crates/yantrik-install-target) decides what may be chosen and
# refuses macOS, Windows, Linux filesystems and the EFI partition; it reads the table again
# right before writing and refuses if it is not the one shown here.
MODE=erase
TARGET_BIN=/opt/yantrik/bin/yantrik-install-target
APPLE=false
case "$(cat /sys/class/dmi/id/sys_vendor 2>/dev/null)" in "Apple Inc."|"Apple Computer, Inc.") APPLE=true ;; esac
if [ -d /sys/firmware/efi ] && [ -x "$TARGET_BIN" ] && command -v jq >/dev/null 2>&1; then
    # shellcheck disable=SC2046
    SCAN=$("$TARGET_BIN" scan --uefi $(lsblk -dn -e 7,11 -o NAME,TYPE | awk '$2 == "disk" { print "/dev/" $1 }') 2>/dev/null || echo '[]')
    ELIGIBLE=$(echo "$SCAN" | jq -r '.[] | .segments[]? | select(.eligible) | .id')
    if [ -n "$ELIGIBLE" ]; then
        step "Install beside what is on a disk"
        echo "$SCAN" | jq -r '.[] | select(.segments | length > 0) | "  \(.disk) \(.model)",
            (.segments[] | "    \(if .eligible then "*" else " " end) \(.id | .[0:28] | . + (" " * (28 - length)))  \(.size | . + (" " * (9 - length))) \(.title)\(if .kept then " (kept)" elif .eligible then "" else " (\(.reason))" end)")'
        echo
        echo "  * may be installed into. Nothing else on that disk changes."
        echo -n "  Target to install into (e.g. $(echo "$ELIGIBLE" | head -1)), or Enter to erase a whole disk instead: "
        read -r INTO
        if [ -n "$INTO" ]; then
            INTO="${INTO#/dev/}"
            echo "$ELIGIBLE" | grep -qxF "$INTO" || { echo -e "${R}$INTO may not be installed into.${N}"; exit 1; }
            MODE=partition
            INTO_DISK=$(echo "$SCAN" | jq -r --arg id "$INTO" '.[] | select(any(.segments[]; .id == $id)) | .disk')
            TABLE_FP=$(echo "$SCAN" | jq -r --arg id "$INTO" '.[] | select(any(.segments[]; .id == $id)) | .fingerprint')
            SENTENCE=$(echo "$SCAN" | jq -r --arg id "$INTO" '.[] | .segments[] | select(.id == $id) | .sentence')
            KEEPS_MACOS=$(echo "$SCAN" | jq -r --arg d "$INTO_DISK" '.[] | select(.disk == $d) | any(.segments[]; .kind == "macos")')
            DISK="$INTO_DISK"
            EXTERNAL=false
            [ "$(lsblk -dno TRAN "$DISK" | tr -d ' ')" = "usb" ] && EXTERNAL=true
            ok "$SENTENCE"
        fi
    fi
fi

# ── 2. Disk selection ──
# TRAN says which disk is the external USB one: on a Mac kept on macOS, that is usually the one
# to install to. The stick this installer booted from is not offered at all.
if [ "$MODE" = erase ]; then
step "Select installation disk"
LIVE_DISK=""
LIVE_SRC=$(findmnt -n -o SOURCE /run/live/medium 2>/dev/null || true)
[ -n "$LIVE_SRC" ] && LIVE_DISK=$(lsblk -no PKNAME "$LIVE_SRC" 2>/dev/null | head -1)
[ -n "$LIVE_SRC" ] && [ -z "$LIVE_DISK" ] && LIVE_DISK=$(basename "$LIVE_SRC")
# A Mac's own system: HFS+ (macOS up to 10.12) or APFS (10.13 on).
holds_macos() { lsblk -nro FSTYPE "/dev/$1" 2>/dev/null | grep -qxE 'hfsplus|apfs'; }
echo -e "  ${B}Available disks:${N}"
printf "  %-10s %-8s %-6s %s\n" NAME SIZE BUS "MODEL / CONTENTS"
for d in $(lsblk -dn -e 7,11 -o NAME,TYPE | awk '$2 == "disk" { print $1 }'); do
    [ "$d" = "$LIVE_DISK" ] && continue
    note=""
    holds_macos "$d" && note="  <- macOS is on this disk"
    printf "  %-10s %-8s %-6s %s%s\n" "$d" "$(lsblk -dno SIZE "/dev/$d" | tr -d ' ')" \
        "$(lsblk -dno TRAN "/dev/$d" | tr -d ' ')" "$(lsblk -dno MODEL "/dev/$d" | sed 's/ *$//')" "$note"
done
echo
echo -n "  Target disk (e.g., sdb): "; read -r TARGET_DISK
[ -z "$TARGET_DISK" ] && { echo -e "${R}No disk specified.${N}"; exit 1; }
TARGET_DISK="${TARGET_DISK#/dev/}"
DISK="/dev/$TARGET_DISK"
[ -b "$DISK" ] || { echo -e "${R}$DISK is not a block device.${N}"; exit 1; }
[ "$TARGET_DISK" = "$LIVE_DISK" ] && { echo -e "${R}$DISK is the installer's own stick.${N}"; exit 1; }
# Erasing macOS is never something a typo does: the disk's name, typed again, is the answer.
if holds_macos "$TARGET_DISK"; then
    echo -e "  ${A}$DISK holds macOS. Installing here erases it. To keep macOS, choose another disk${N}"
    echo -e "  ${A}(an external USB SSD, for instance).${N}"
    echo -n "  Type the disk's name ($TARGET_DISK) to erase macOS on it: "; read -r MACOS_OK
    [ "$MACOS_OK" = "$TARGET_DISK" ] || { echo "  Nothing was changed."; exit 1; }
fi
# A disk on USB is a disk that may move between ports and machines: it gets no NVRAM boot entry
# on this machine, and boots through the removable-media path (\EFI\BOOT\BOOTX64.EFI) wherever
# it is plugged in. On a Mac, hold Option at the chime and pick "EFI Boot".
EXTERNAL=false
[ "$(lsblk -dno TRAN "$DISK" | tr -d ' ')" = "usb" ] && EXTERNAL=true
[ "$(cat "/sys/block/$TARGET_DISK/removable" 2>/dev/null)" = "1" ] && EXTERNAL=true
fi

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
if [ "$MODE" = partition ]; then
    printf "  %-12s %s\n" "Into" "/dev/$INTO, on $DISK"
    echo
    echo -e "  ${A}$SENTENCE${N}"
else
    printf "  %-12s %s\n" "Disk"      "$DISK ($(lsblk -dno SIZE "$DISK" 2>/dev/null | tr -d ' '))"
    echo
    echo -e "  ${A}Everything on $DISK will be erased. There is no recovery.${N}"
fi
echo -n "  Type 'yes' to install: "; read -r CONFIRM
[ "$CONFIRM" = "yes" ] || { echo "  Nothing was changed."; exit 1; }

# ── 3. Partition ──
IS_EFI=false; [ -d /sys/firmware/efi ] && IS_EFI=true
if [ "$MODE" = partition ]; then
    # No new table: the partition is made in the free space (or the placeholder wiped), after
    # the planner has read the table again and found it as it was shown above.
    step "Making room in /dev/$INTO (nothing else on $DISK changes)..."
    PLACED=$("$TARGET_BIN" apply --uefi "$INTO" "$TABLE_FP") \
        || { echo -e "${R}Refused; nothing was written.${N}"; exit 1; }
    ROOT_PART=$(echo "$PLACED" | jq -r .root)
    EFI_PART=$(echo "$PLACED" | jq -r .esp)
    [ -b "$ROOT_PART" ] && [ -b "$EFI_PART" ] || { echo -e "${R}The planner named no usable root ($ROOT_PART).${N}"; exit 1; }
    mkfs.ext4 -q -F -L YANTRIK "$ROOT_PART"
    ok "Installing into $ROOT_PART; the EFI partition $EFI_PART is kept as it is"
else
step "Partitioning $DISK (GPT)..."
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
fi

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
    --exclude='/live/*' --exclude='/cdrom/*' --exclude='/boot/efi/*' \
    / "$M/" --info=progress2
ok "System copied"
# The live image's blanket rule (`yantrik ALL=(ALL) NOPASSWD: ALL`) came over with the copy. It
# is the live session's alone, so it goes now rather than at the end: an install cut off before
# step 14b must not leave a disk that boots with it (#397; security review of #616).
rm -f "$M/etc/sudoers.d/yantrik"

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
# By UUID, never /dev/sdX: a USB disk is sdb in one port and sdc in another, and the EFI line used
# to name the device, so the same disk failed to mount /boot/efi after moving ports. Not by label
# either: the live stick or a second Yantrik disk can carry the same one.
ROOT_UUID=$(blkid -s UUID -o value "$ROOT_PART")
[ -n "$ROOT_UUID" ] || { echo -e "${R}Could not read the UUID of $ROOT_PART.${N}"; exit 1; }
echo "UUID=$ROOT_UUID  /  ext4  defaults,noatime  0  1" > "$M/etc/fstab"
if $IS_EFI && [ -n "$EFI_PART" ]; then
    EFI_UUID=$(blkid -s UUID -o value "$EFI_PART")
    [ -n "$EFI_UUID" ] || { echo -e "${R}Could not read the UUID of $EFI_PART.${N}"; exit 1; }
    echo "UUID=$EFI_UUID  /boot/efi  vfat  umask=0077  0  2" >> "$M/etc/fstab"
fi

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
# Subordinate ids for rootless podman (#401). useradd gives the account a range from the image's
# /etc/subuid; this checks, and adds one past every range already handed out when it did not.
for kind in uid gid; do
    [ -e "$M/etc/sub$kind" ] || install -m 0644 /dev/null "$M/etc/sub$kind"
    grep -q "^$USERNAME:" "$M/etc/sub$kind" && continue
    start=$(awk -F: -v t=100000 '$2 ~ /^[0-9]+$/ && $3 ~ /^[0-9]+$/ && $2 + $3 > t { t = $2 + $3 } END { printf "%.0f\n", t }' "$M/etc/sub$kind")
    chroot "$M" usermod "--add-sub${kind}s" "$start-$((start + 65535))" "$USERNAME" \
        || echo "  could not give $USERNAME subordinate ${kind}s; podman will pull few images until the first update adds them"
done
HASH=$(openssl passwd -6 "$PASSWORD")
chroot "$M" usermod -p "$HASH" "$USERNAME"
# No `NOPASSWD: ALL` for the account (#397). This used to write one, so anything running as the
# person was root without asking; the narrow rule is put in place at the end, by the updater.
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

# The environment file stays: yantrik-session does not set the renderer, and this is where a
# machine with no GPU is told to fall back to software rendering.
printf "WLR_RENDERER_ALLOW_SOFTWARE=1\nWLR_NO_HARDWARE_CURSORS=1\nWLR_RENDERER=pixman\nXDG_SESSION_TYPE=wayland\nQT_QPA_PLATFORM=wayland\nMOZ_ENABLE_WAYLAND=1\nSLINT_BACKEND=winit\nLIBGL_ALWAYS_SOFTWARE=1\n" > "$UHOME/.config/labwc/environment"

# No autostart and no rc.xml written here. yantrik-session copies the shipped ones out of
# /opt/yantrik/share at every login, so a machine installed today picks up a theme fix
# published tomorrow by rebooting. Writing them here would shadow that permanently.

# .bash_profile — the same one the live image uses, minus the installer-mode branch.
printf 'if [ "$(tty)" = "/dev/tty1" ] && [ -z "$WAYLAND_DISPLAY" ]; then\n    export XDG_RUNTIME_DIR="/run/user/$(id -u)"\n    mkdir -p "$XDG_RUNTIME_DIR"\n    if [ -f "$HOME/.config/labwc/environment" ]; then\n        set -a; . "$HOME/.config/labwc/environment"; set +a\n    fi\n    /opt/yantrik/bin/yantrik-session 2>>/opt/yantrik/logs/labwc.log\nfi\n' > "$UHOME/.bash_profile"

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

# ── 14b. Whose files these are (#397) ──
#
# /opt/yantrik is root's on an installed machine: the desktop's account keeps logs/, data/ and
# config.yaml, and may run the updater and a few helpers as root through one narrow rule. The
# updater's `migrate-ownership` writes that layout and that rule; yantrik-lockdown runs it, checks
# the result rather than trusting its exit code, and locks the tree down itself when it is not
# right. --fresh: the live session's logs are not the person's. Before the bootloader, so an
# install that cannot do this leaves a disk that does not boot rather than one that boots open
# (security review of #614 and #616, 4 October 2026).
rm -f "$M/opt/yantrik/.installer-mode"
step "Making the system's files root's..."
LOCKDOWN_RC=0
chroot "$M" /opt/yantrik/bin/yantrik-lockdown secure --fresh "$USERNAME" || LOCKDOWN_RC=$?
case "$LOCKDOWN_RC" in
    0) ok "/opt/yantrik is root's; $USERNAME keeps logs/, data/ and config.yaml" ;;
    3) echo -e "   ${A}Yantrik could not finish securing its system files, so the installer locked them down itself.${N}"
       echo -e "   ${A}If updates ask for your password, run: sudo yantrik-update migrate-ownership${N}" ;;
    *) echo -e "   ${R}Could not make /opt/yantrik root's. Stopping before the bootloader: this disk will not boot.${N}" >&2
       exit 1 ;;
esac

# ── 15. GRUB ──
step "Installing bootloader..."
printf 'GRUB_DEFAULT=0\nGRUB_TIMEOUT=3\nGRUB_DISTRIBUTOR="Yantrik OS"\nGRUB_CMDLINE_LINUX_DEFAULT="quiet splash"\nGRUB_CMDLINE_LINUX=""\n' > "$M/etc/default/grub"

if $IS_EFI; then
    # Two installs, as the desktop installer does (crates/yantrik-ui/src/wire/installer.rs). The
    # named one, with a firmware boot entry for an internal disk; then \EFI\BOOT\BOOTX64.EFI, the
    # removable-media path every UEFI tries when it has no entry, and the only one Apple's firmware
    # needs. This used to run only the first, with --no-nvram, which left a disk no firmware would
    # boot.
    #
    # A Mac's NVRAM is never written (macOS stays what starts; Option picks Yantrik), nor a USB
    # disk's. Installed beside another system on a PC, the entry is added last, so what started
    # before still starts (crates/yantrik-install-target/src/efi.rs has the same rules).
    NVRAM_FLAG=""
    { $EXTERNAL || $APPLE || [ "$MODE" = partition ]; } && NVRAM_FLAG="--no-nvram"
    chroot "$M" grub-install --target=x86_64-efi --efi-directory=/boot/efi \
        --bootloader-id=yantrik $NVRAM_FLAG \
        || chroot "$M" grub-install --target=x86_64-efi --efi-directory=/boot/efi \
            --bootloader-id=yantrik --no-nvram \
        || true
    if [ "$MODE" = partition ] && ! $APPLE && ! $EXTERNAL && command -v efibootmgr >/dev/null 2>&1; then
        # A failure here costs the entry, not the install: the firmware's menu still finds it.
        ORDER=$(efibootmgr 2>/dev/null | sed -n 's/^BootOrder: //p') || ORDER=""
        ESP_NUM=$(cat "/sys/class/block/$(basename "$EFI_PART")/partition" 2>/dev/null) || ESP_NUM=""
        NEW=""
        if [ -n "$ESP_NUM" ]; then
            NEW=$(efibootmgr -C -d "$DISK" -p "$ESP_NUM" -L "Yantrik OS" -l '\EFI\yantrik\grubx64.efi' 2>/dev/null \
                | sed -n 's/^Boot\([0-9A-Fa-f]\{4\}\)\*\{0,1\} Yantrik OS.*/\1/p' | tail -1) || NEW=""
        fi
        if [ -n "$NEW" ] && efibootmgr -o "${ORDER:+$ORDER,}$NEW" >/dev/null 2>&1; then
            ok "Firmware entry Boot$NEW added, last in the boot order"
        else
            echo -e "   ${A}No firmware entry was added; choose Yantrik OS from the firmware's boot menu.${N}"
        fi
    fi
    # \EFI\BOOT\BOOTX64.EFI: another system's is never replaced. Yantrik's own is known by the
    # sha256 recorded beside it in YANTRIK.OWN.
    EXISTING=$(find "$M/boot/efi" -maxdepth 3 -ipath '*/EFI/BOOT/BOOTX64.EFI' 2>/dev/null | head -1)
    OURS=false
    if [ -n "$EXISTING" ] && [ -f "$M/boot/efi/EFI/BOOT/YANTRIK.OWN" ] \
        && [ "sha256=$(sha256sum "$EXISTING" | cut -d' ' -f1)" = "$(cat "$M/boot/efi/EFI/BOOT/YANTRIK.OWN")" ]; then
        OURS=true
    fi
    if [ "$MODE" != partition ] || [ -z "$EXISTING" ] || $OURS; then
        chroot "$M" grub-install --target=x86_64-efi --efi-directory=/boot/efi --removable
        [ -f "$M/boot/efi/EFI/BOOT/BOOTX64.EFI" ] \
            || { echo -e "${R}No EFI/BOOT/BOOTX64.EFI on the EFI partition; this disk would not boot.${N}" >&2; exit 1; }
        echo "sha256=$(sha256sum "$M/boot/efi/EFI/BOOT/BOOTX64.EFI" | cut -d' ' -f1)" > "$M/boot/efi/EFI/BOOT/YANTRIK.OWN"
        if $APPLE && [ "$MODE" = partition ]; then
            BOOT_NOTE="macOS still starts by default. Hold Option at the chime and choose EFI Boot to start Yantrik OS."
        fi
    else
        BOOT_NOTE="The EFI partition already has a \\EFI\\BOOT\\BOOTX64.EFI that is not Yantrik's, so it was left alone. Start Yantrik OS from rEFInd or the firmware's boot menu (\\EFI\\yantrik\\grubx64.efi)."
    fi
    # Beside macOS on a Mac, a boot menu entry that gets back to it.
    if $APPLE && [ "$MODE" = partition ] && [ "${KEEPS_MACOS:-false}" = true ]; then
        "$TARGET_BIN" grub-macos-entry > "$M/etc/grub.d/35_yantrik_macos" && chmod 0755 "$M/etc/grub.d/35_yantrik_macos"
    fi
else
    chroot "$M" grub-install --target=i386-pc "$DISK"
fi
# The kernel line must find the root by UUID too, or the disk boots in one USB port and not the
# next. grub-mkconfig falls back to root=/dev/sdX when udev has not made the by-uuid link yet.
udevadm settle 2>/dev/null || true
chroot "$M" update-grub
if grep -qE 'root=/dev/(sd|nvme|vd|hd|mmcblk)' "$M/boot/grub/grub.cfg"; then
    echo -e "${R}grub.cfg names the root by device, not UUID; it would not boot from another port.${N}" >&2
    exit 1
fi
ok "GRUB installed$($IS_EFI && echo ' (EFI, with the removable-media fallback)')"

# ── 16. Regenerate initramfs (without live-boot hooks) ──
chroot "$M" update-initramfs -u 2>/dev/null || true

# ── 17. Cleanup ──
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
if [ -n "${BOOT_NOTE:-}" ]; then
    echo -e "  ${A}${BOOT_NOTE}${N}"
    echo
fi
echo -n "Reboot now? [Y/n] "; read -r RB
[ "$RB" != "n" ] && reboot
