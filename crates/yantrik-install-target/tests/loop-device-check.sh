#!/bin/bash
# loop-device-check.sh — the install-into-a-partition planner against real loop devices.
#
# Builds GPT disk images shaped like a Mac mini whose APFS container was shrunk in macOS: an EFI
# system partition with files on it, a stand-in APFS partition (type 7C3457EF-0000-11AA-AA11-
# 00306543ECAC, random bytes at its head and tail) and, after it, free space or a FAT32
# placeholder labelled YANTRIK. Then runs `yantrik-install-target` exactly as the installers do
# (scan, preselect, plan, apply) and checks:
#
#   - the APFS partition's head and tail are unchanged (sha256 of its first and last 64 MiB);
#   - the EFI partition is never formatted: its filesystem UUID and every file on it are unchanged;
#   - every partition that is not the target, before it and after it, keeps its exact extent,
#     type, GUID, name and flags; the new ones lie inside the target;
#   - the partition table is never rewritten (the disk's GPT GUID is unchanged);
#   - a stale fingerprint, the APFS partition, the EFI partition and the installer's own
#     partition are refused, each for its own reason, with no change.
#
# Section 4 is the Mac this is for, at its real size (a sparse 1 TB image): the 209.7 MB EFI
# partition at sector 40, APFS of 792 GB, the 8 GB YKINSTALL partition the installer runs from
# (mounted at /run/live/medium, as the live system mounts it), and the 199 GB FAT32 partition
# labelled YANTRIK; beside it a decoy disk whose FAT partitions, one labelled YANTRIK-BAK and one
# labelled exactly YANTRIK, hold files. 4a runs the unencrypted placeholder path (wipefs, then
# parted type), as the text installer does; 4b, on the same layout made again, the encrypted
# one the desktop installer takes by default (wipefs, rm, then /boot and root made inside the
# placeholder's extent). Both check the decoy is untouched byte for byte.
# 4a and 4b write the EFI fallback as both installers do (`yantrik-install-target efi-fallback
# write`, from an \EFI\yantrik filled as the named grub-install with shim fills it) and check
# that \EFI\BOOT holds exactly BOOTX64.EFI, grubx64.efi, mmx64.efi and grub.cfg, each listed by
# sha256 in YANTRIK.OWN, and nothing else: no BOOTX64.CSV, no fbx64.efi.
# Section 5 is the disks refused whole: a hybrid MBR, an EFI partition too full for a loader,
# and an empty partition that is not blank.
# Section 6 is \EFI\BOOT holding another system's grub.cfg, grubx64.efi, BOOTX64.EFI or
# fbx64.efi: nothing is written there and it stays byte for byte; and a re-install over
# Yantrik's own set, which is written over and recorded again.
#
# Run as root, on a machine with loop devices (sparse images, a few hundred MB of real writes):
#   sudo crates/yantrik-install-target/tests/loop-device-check.sh [path/to/yantrik-install-target]
set -euo pipefail

BIN="${1:-${CARGO_TARGET_DIR:-target}/debug/yantrik-install-target}"
[ "$(id -u)" = 0 ] || { echo "run as root (loop devices, parted)" >&2; exit 2; }
[ -x "$BIN" ] || { echo "no yantrik-install-target at $BIN; cargo build -p yantrik-install-target" >&2; exit 2; }
for tool in losetup sgdisk parted mkfs.fat mkfs.ext4 blkid sha256sum wipefs blockdev od jq; do
    command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 2; }
done
[ -e /dev/loop-control ] || { echo "no loop devices here" >&2; exit 2; }

WORK=$(mktemp -d /var/tmp/yantrik-partcheck.XXXXXX)
LOOP=""
DECOY=""
MNT="$WORK/mnt"
MEDIUM=/run/live/medium
mkdir -p "$MNT"
cleanup() {
    mountpoint -q "$MNT" && umount "$MNT"
    mountpoint -q "$MEDIUM" && umount "$MEDIUM"
    [ -n "$LOOP" ] && losetup -d "$LOOP" 2>/dev/null
    [ -n "$DECOY" ] && losetup -d "$DECOY" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }
APFS_TYPE=7C3457EF-0000-11AA-AA11-00306543ECAC
LINUX_TYPE=0FC63DAF-8483-4772-8E79-3D69D8477DE4
MIB=1048576

# The EFI partition's files, as a Mac's firmware and rEFInd leave them. $1: the partition.
fill_esp() {
    mkfs.fat -F32 -n EFI "$1" >/dev/null
    mount "$1" "$MNT"
    mkdir -p "$MNT/EFI/APPLE/FIRMWARE" "$MNT/EFI/refind"
    head -c 3000000 /dev/urandom > "$MNT/EFI/APPLE/FIRMWARE/MM61.scap"
    echo "macOS firmware log" > "$MNT/EFI/APPLE/log.txt"
    head -c 200000 /dev/urandom > "$MNT/EFI/refind/refind_x64.efi"
    umount "$MNT"
}

# A stand-in for APFS: random bytes at its head and its tail. $1: the partition.
fill_apfs() {
    local size; size=$(blockdev --getsize64 "$1")
    dd if=/dev/urandom of="$1" bs=1M count=64 status=none
    dd if=/dev/urandom of="$1" bs=1M count=64 seek=$((size - 64 * MIB)) oflag=seek_bytes status=none
}

# A disk image: the Mac's EFI partition at sector 40 (209.7 MB), a 256 MiB stand-in for APFS,
# then whatever "$2" adds. Leaves LOOP set and the image attached with partitions.
make_disk() {
    local img="$WORK/$1.img"
    rm -f "$img"
    truncate -s 32G "$img"
    sgdisk -o "$img" >/dev/null
    sgdisk -a 8 -n 1:40:409639 -t 1:EF00 -c 1:"EFI System Partition" "$img" >/dev/null
    sgdisk -a 8 -n 2:409640:933927 -t 2:$APFS_TYPE -c 2:"Container" "$img" >/dev/null
    shift
    [ $# -gt 0 ] && sgdisk "$@" "$img" >/dev/null
    LOOP=$(losetup -fP --show "$img")
    sleep 1
    fill_esp "${LOOP}p1"
    fill_apfs "${LOOP}p2"
    sync
}

part_info() {   # $1: partition number: everything GPT keeps about it
    sgdisk -i "$1" "$LOOP" | grep -E 'GUID code|First sector|Last sector|unique GUID|Partition name|Attribute flags'
}
first_sector() { sgdisk -i "$1" "$LOOP" | sed -n 's/^First sector: \([0-9]*\).*/\1/p'; }
last_sector() { sgdisk -i "$1" "$LOOP" | sed -n 's/^Last sector: \([0-9]*\).*/\1/p'; }
part_count() { sgdisk -p "$LOOP" | grep -cE '^ +[0-9]+ '; }

apfs_ends() {
    local size; size=$(blockdev --getsize64 "${LOOP}p2")
    dd if="${LOOP}p2" bs=1M count=64 status=none | sha256sum | cut -d' ' -f1
    dd if="${LOOP}p2" bs=1M count=64 skip=$((size - 64 * MIB)) iflag=skip_bytes status=none | sha256sum | cut -d' ' -f1
}

# Every file Yantrik writes in \EFI\BOOT (crates/yantrik-install-target/src/efi.rs, SHIM_SET),
# in C order.
FALLBACK_SET="BOOTX64.EFI grub.cfg grubx64.efi mmx64.efi"

# The EFI partition's files, by sha256, that an install must leave as they were: all but
# \EFI\yantrik's and the fallback's. The allowlist for \EFI\BOOT is exactly what YANTRIK.OWN
# records, and only when it records exactly FALLBACK_SET and each file matches its digest;
# otherwise a line saying so is printed, and any file in \EFI\BOOT it does not record
# (BOOTX64.CSV, say) is listed as itself, so either counts as a change.
esp_files() {
    mount -o ro "${LOOP}p1" "$MNT"
    local own="$MNT/EFI/BOOT/YANTRIK.OWN" recorded="" skip=""
    if [ -f "$own" ]; then
        recorded=$(awk '{ print $2 }' "$own" | LC_ALL=C sort | xargs)
        if [ "$recorded" = "$FALLBACK_SET" ] && (cd "$MNT/EFI/BOOT" && sha256sum --quiet --strict -c YANTRIK.OWN >/dev/null 2>&1); then
            skip="$FALLBACK_SET YANTRIK.OWN"
        else
            echo "YANTRIK.OWN records '$recorded', not exactly '$FALLBACK_SET' with matching digests" | tee /dev/stderr
        fi
    fi
    (cd "$MNT" && find . -type f ! -path './EFI/yantrik/*' -exec sha256sum {} + | sort) \
        | awk -v skip="$skip" 'BEGIN { n = split(skip, s, " "); for (i = 1; i <= n; i++) ok["./EFI/BOOT/" s[i]] = 1 } !($2 in ok)'
    umount "$MNT"
}

# \EFI\yantrik as the named grub-install with shim leaves it ($1: the mounted EFI partition),
# with BOOTX64.CSV and fbx64.efi, which the fallback must not copy.
fill_yantrik() {
    mkdir -p "$1/EFI/yantrik"
    for f in shimx64.efi grubx64.efi mmx64.efi fbx64.efi; do head -c 150000 /dev/urandom > "$1/EFI/yantrik/$f"; done
    printf 'search.fs_uuid %s root\nset prefix=($root)/grub\nconfigfile $prefix/grub.cfg\n' "$RANDOM$RANDOM" > "$1/EFI/yantrik/grub.cfg"
    printf 'shimx64.efi,Yantrik OS,,This is the boot entry for Yantrik OS\n' > "$1/EFI/yantrik/BOOTX64.CSV"
}

# \EFI\BOOT on the EFI partition mounted at $1 after `efi-fallback write` printed $2: exactly
# the set and YANTRIK.OWN, each file listed in it by sha256, BOOTX64.EFI shim. $3: what was done.
fallback_written() {
    [ "$(jq -r '[.written[].name] | sort | join(" ")' <<<"$2")" = "$FALLBACK_SET" ] || { echo "$2"; fail "$3: wrote other than $FALLBACK_SET"; }
    [ "$(ls -A "$1/EFI/BOOT" | LC_ALL=C sort | xargs)" = "$(printf '%s\n' $FALLBACK_SET YANTRIK.OWN | LC_ALL=C sort | xargs)" ] \
        || fail "$3: EFI/BOOT holds $(ls -A "$1/EFI/BOOT" | xargs)"
    [ "$(awk '{ print $2 }' "$1/EFI/BOOT/YANTRIK.OWN" | LC_ALL=C sort | xargs)" = "$FALLBACK_SET" ] || fail "$3: YANTRIK.OWN records other files"
    (cd "$1/EFI/BOOT" && sha256sum --quiet --strict -c YANTRIK.OWN) || fail "$3: a file in EFI/BOOT does not match YANTRIK.OWN"
    cmp -s "$1/EFI/yantrik/shimx64.efi" "$1/EFI/BOOT/BOOTX64.EFI" || fail "$3: BOOTX64.EFI is not shim"
    cmp -s "$1/EFI/yantrik/grubx64.efi" "$1/EFI/BOOT/grubx64.efi" || fail "$3: grubx64.efi is not \\EFI\\yantrik's"
}

# What the installers do to the EFI partition after the root is made: the named GRUB in
# \EFI\yantrik, then the fallback in \EFI\BOOT. $1: what was done.
install_boot() {
    mount "${LOOP}p1" "$MNT"
    fill_yantrik "$MNT"
    [ "$("$BIN" efi-fallback check --apple "$MNT" | jq -r .write)" = true ] || fail "$1: the fallback was not to be written"
    OUT=$("$BIN" efi-fallback write --apple "$MNT")
    fallback_written "$MNT" "$OUT" "$1"
    umount "$MNT"
}

snapshot() {   # what must never change. $1: the partitions that are not the target
    KEEP=$1
    APFS_ENDS=$(apfs_ends)
    ESP_UUID=$(blkid -p -s UUID -o value "${LOOP}p1")
    ESP_FILES=$(esp_files)
    DISK_GUID=$(sgdisk -p "$LOOP" | sed -n 's/^Disk identifier (GUID): //p')
    PARTS=$(for n in $KEEP; do echo "== $n"; part_info "$n"; done)
}

verify_kept() {   # $1: what was just done
    [ "$(apfs_ends)" = "$APFS_ENDS" ] || fail "$1: the APFS partition's head or tail changed"
    [ "$(blkid -p -s UUID -o value "${LOOP}p1")" = "$ESP_UUID" ] || fail "$1: the EFI partition was reformatted"
    [ "$(esp_files)" = "$ESP_FILES" ] || fail "$1: files on the EFI partition changed"
    [ "$(sgdisk -p "$LOOP" | sed -n 's/^Disk identifier (GUID): //p')" = "$DISK_GUID" ] || fail "$1: the partition table was rewritten"
    [ "$(for n in $KEEP; do echo "== $n"; part_info "$n"; done)" = "$PARTS" ] || fail "$1: a partition that was not the target changed"
}

# The fingerprint of disk $2 in scan $1, by jq; the script stops if there is none, so a refusal
# can never pass on a fingerprint that did not match.
fp_of() {
    local fp; fp=$(jq -r --arg d "$2" '.[] | select(.disk == $d) | .fingerprint // empty' "$1")
    [[ "$fp" =~ ^[0-9a-f]{16}$ ]] || { cat "$1" >&2; fail "no fingerprint for $2 in the scan"; }
    echo "$fp"
}

# Field $3 of segment $2 in scan $1, by jq.
seg() { jq -r --arg id "$2" --arg f "$3" '[.[] | .segments[] | select(.id == $id)] | if length == 1 then .[0][$f] | tostring else "NO SEGMENT" end' "$1"; }

# The first free run offered in scan $1.
first_free() { jq -r '[.[] | .segments[] | select(.kind == "free" and .eligible)][0].id // empty' "$1"; }

# $1: what, $2: words the refusal must give as its reason, rest: the command. A refusal for any
# other reason (a fingerprint mismatch, a usage error) is a failure of the check.
expect_refusal() {
    local what=$1 reason=$2; shift 2
    if "$@" >"$WORK/out" 2>&1; then cat "$WORK/out"; fail "$what was not refused"; fi
    grep -qF -- "$reason" "$WORK/out" || { cat "$WORK/out"; fail "$what was refused, but not because it $reason"; }
    echo "   refused: $(tail -1 "$WORK/out")"
}

# ── 1. Free space after the shrunk container ─────────────────────────────────────────────
echo "== free space beside APFS"
make_disk free
snapshot "1 2"
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
FP=$(fp_of "$WORK/scan.json" "$LOOP")
FREE_ID=$(first_free "$WORK/scan.json")
[ -n "$FREE_ID" ] || { cat "$WORK/scan.json"; fail "scan offered no free space"; }
[ "$(seg "$WORK/scan.json" "$(basename "$LOOP")p2" kind)" = macos ] || fail "the stand-in APFS partition was not read as macOS"
echo "   fingerprint $FP, target $FREE_ID"

expect_refusal "installing over APFS" "holds macOS (APFS)" "$BIN" apply --uefi "$(basename "$LOOP")p2" "$FP"
expect_refusal "installing over the EFI partition" "is the EFI system partition" "$BIN" apply --uefi "$(basename "$LOOP")p1" "$FP"
expect_refusal "a stale fingerprint" "has changed since it was shown" "$BIN" apply --uefi "$FREE_ID" 0123456789abcdef
verify_kept "the refusals"
pass "APFS, the EFI partition and a stale fingerprint are refused, each for its reason, and nothing changed"

"$BIN" plan --uefi "$FREE_ID" "$FP" > "$WORK/plan.json"
grep -q mklabel "$WORK/plan.json" && fail "the plan rewrites the table"
"$BIN" apply --uefi "$FREE_ID" "$FP" > "$WORK/apply.json"
ROOT=$(jq -r .root "$WORK/apply.json")
ESP=$(jq -r .esp "$WORK/apply.json")
[ "$ESP" = "${LOOP}p1" ] || fail "the EFI partition reported is $ESP"
[ -b "$ROOT" ] || fail "no root device $ROOT"
mkfs.ext4 -q -F -L YANTRIK "$ROOT"
# What the installer does next to the EFI partition: mount it and add \EFI\yantrik beside the rest.
mount "$ESP" "$MNT"
mkdir -p "$MNT/EFI/yantrik"
head -c 150000 /dev/urandom > "$MNT/EFI/yantrik/grubx64.efi"
umount "$MNT"
verify_kept "installing into free space"
FREE_START=${FREE_ID#*@}; FREE_START=${FREE_START%-*}
FREE_END=${FREE_ID##*-}
NEW_START=$(first_sector 3)
NEW_END=$(last_sector 3)
[ "$NEW_START" -ge "$FREE_START" ] && [ "$NEW_END" -le "$FREE_END" ] || fail "the new partition $NEW_START-$NEW_END is outside $FREE_START-$FREE_END"
[ $((NEW_START % 2048)) = 0 ] || fail "the new partition does not start on a MiB boundary"
sgdisk -i 3 "$LOOP" | grep -q "$LINUX_TYPE" || fail "the new partition is not typed Linux filesystem"
pass "free space: one partition at $NEW_START-$NEW_END inside $FREE_START-$FREE_END; APFS, the EFI partition and its files unchanged"
losetup -d "$LOOP"; LOOP=""

# ── 2. A FAT32 YANTRIK placeholder, encrypted layout (/boot and root inside it) ───────────
echo "== placeholder labelled YANTRIK, encrypted layout, with a partition after it"
make_disk placeholder -a 8 -n 3:1196072:44040191 -t 3:0700 -c 3:YANTRIK -n 4:44040192:46137343 -t 4:8300 -c 4:after
mkfs.fat -F32 -n YANTRIK "${LOOP}p3" >/dev/null
mkfs.ext4 -q -L AFTER "${LOOP}p4"
AFTER_SUM=$(sha256sum < "${LOOP}p4" | cut -d' ' -f1)
snapshot "1 2 4"
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
FP=$(fp_of "$WORK/scan.json" "$LOOP")
PH="$(basename "$LOOP")p3"
[ "$(seg "$WORK/scan.json" "$PH" kind)" = placeholder ] || { cat "$WORK/scan.json"; fail "the placeholder was not offered"; }
"$BIN" plan --uefi --encrypt "$PH" "$FP" > "$WORK/plan.json"
grep -q mklabel "$WORK/plan.json" && fail "the plan rewrites the table"
"$BIN" apply --uefi --encrypt "$PH" "$FP" > "$WORK/apply.json"
ROOT=$(jq -r .root "$WORK/apply.json")
BOOT=$(jq -r .boot "$WORK/apply.json")
[ -b "$ROOT" ] && [ -b "$BOOT" ] || fail "no root ($ROOT) or /boot ($BOOT)"
verify_kept "installing into the placeholder"
[ "$(sha256sum < "${LOOP}p4" | cut -d' ' -f1)" = "$AFTER_SUM" ] || fail "the partition after the placeholder changed"
for dev in "$BOOT" "$ROOT"; do
    n=${dev##*p}
    [ "$(first_sector "$n")" -ge 1196072 ] && [ "$(last_sector "$n")" -le 44040191 ] || fail "$dev is outside the placeholder 1196072-44040191"
done
pass "placeholder: /boot and root made inside its extent; the partitions before and after it, APFS and the EFI partition unchanged"
losetup -d "$LOOP"; LOOP=""

# ── 3. The table changes between showing it and installing ───────────────────────────────
echo "== the table changes after it was shown"
make_disk changed
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
FP=$(fp_of "$WORK/scan.json" "$LOOP")
FREE_ID=$(first_free "$WORK/scan.json")
# Something else adds a partition in the free space before Install is pressed.
sgdisk -n 3:2000000:3000000 -t 3:0700 "$LOOP" >/dev/null
partprobe "$LOOP" 2>/dev/null || true
snapshot "1 2 3"
BEFORE=$(sgdisk -p "$LOOP")
expect_refusal "installing after the table changed" "has changed since it was shown" "$BIN" apply --uefi "$FREE_ID" "$FP"
[ "$(sgdisk -p "$LOOP")" = "$BEFORE" ] || fail "the refused install changed the table"
verify_kept "the refused install"
pass "a changed table is refused before anything is written"
losetup -d "$LOOP"; LOOP=""

# ── 4. The Mac as it is, with a decoy disk beside it ─────────────────────────────────────
# The Mac's disk at its real geometry, in a sparse 1 TB image: LOOP set, YKINSTALL mounted at
# /run/live/medium, YANTRIK carrying what macOS leaves on a FAT volume it has mounted.
make_mac() {
    local img="$WORK/mac.img"
    rm -f "$img"
    truncate -s $((1953525168 * 512)) "$img"
    sgdisk -o "$img" >/dev/null
    sgdisk -a 8 -n 1:40:409639 -t 1:EF00 -c 1:"EFI System Partition" "$img" >/dev/null
    sgdisk -a 8 -n 2:409640:1547284639 -t 2:$APFS_TYPE -c 2:"Container" "$img" >/dev/null
    sgdisk -a 8 -n 3:1547546784:1563171783 -t 3:0700 -c 3:YKINSTALL "$img" >/dev/null
    sgdisk -a 8 -n 4:1563433928:1952105802 -t 4:0700 -c 4:YANTRIK "$img" >/dev/null
    LOOP=$(losetup -fP --show "$img")
    sleep 1
    fill_esp "${LOOP}p1"
    fill_apfs "${LOOP}p2"
    mkfs.fat -F32 -n YKINSTALL "${LOOP}p3" >/dev/null
    mount "${LOOP}p3" "$MNT"; mkdir -p "$MNT/live"; head -c 4000000 /dev/urandom > "$MNT/live/filesystem.squashfs"; umount "$MNT"
    mkfs.fat -F32 -n YANTRIK "${LOOP}p4" >/dev/null
    mount "${LOOP}p4" "$MNT"
    mkdir -p "$MNT/.fseventsd" "$MNT/.Spotlight-V100/Store-V2" "$MNT/.Trashes/501" "$MNT/.TemporaryItems/folders.501"
    echo x > "$MNT/.fseventsd/fseventsd-uuid"; echo x > "$MNT/.Spotlight-V100/Store-V2/store"
    echo x > "$MNT/.DS_Store"; echo x > "$MNT/._.DS_Store"; echo x > "$MNT/.VolumeIcon.icns"
    echo x > "$MNT/.TemporaryItems/folders.501/Cleanup At Startup"
    umount "$MNT"
    mkdir -p "$MEDIUM"
    mount -o ro "${LOOP}p3" "$MEDIUM"
    MEDIUM_FILES=$(cd "$MEDIUM" && find . -type f -exec sha256sum {} + | sort)
    M=$(basename "$LOOP")
    snapshot "1 2 3"
    sync
}

# Every way into the Mac or the decoy that must be refused, each for its own reason.
refuse_on_mac() {
    "$BIN" scan --uefi "$LOOP" "$DECOY" > "$WORK/scan.json"
    FP=$(fp_of "$WORK/scan.json" "$LOOP")
    DFP=$(fp_of "$WORK/scan.json" "$DECOY")
    [ "$FP" != "$DFP" ] || fail "the Mac and the decoy have the same fingerprint"
    expect_refusal "installing over the APFS stand-in" "holds macOS (APFS)" "$BIN" apply --uefi "${M}p2" "$FP"
    expect_refusal "installing over the EFI partition" "is the EFI system partition" "$BIN" apply --uefi "${M}p1" "$FP"
    expect_refusal "installing over YKINSTALL, the installer's own" "is what this installer is running from" "$BIN" apply --uefi "${M}p3" "$FP"
    expect_refusal "installing over the decoy's YANTRIK-BAK" "labelled YANTRIK-BAK" "$BIN" apply --uefi "${D}p2" "$DFP"
    expect_refusal "installing over the decoy's YANTRIK with files" "holds files (Photos, Photos/IMG_0001.HEIC)" "$BIN" apply --uefi "${D}p3" "$DFP"
    verify_kept "the refusals on the Mac"
}

# What must hold after any install into YANTRIK on the Mac.
mac_unchanged() {
    verify_kept "$1"
    [ "$(cd "$MEDIUM" && find . -type f -exec sha256sum {} + | sort)" = "$MEDIUM_FILES" ] || fail "$1: YKINSTALL's files changed"
    [ "$(sha256sum < "$DECOY" | cut -d' ' -f1)" = "$DECOY_SUM" ] || fail "$1: the decoy disk changed"
}

drop_mac() {
    umount "$MEDIUM"
    losetup -d "$LOOP"; LOOP=""
    rm -f "$WORK/mac.img"
}

echo "== the Mac mini's own layout (1 TB, sparse), and a decoy disk"
DIMG="$WORK/decoy.img"
truncate -s 2G "$DIMG"
sgdisk -o "$DIMG" >/dev/null
sgdisk -a 8 -n 1:40:204839 -t 1:EF00 -c 1:"EFI System Partition" "$DIMG" >/dev/null
sgdisk -a 8 -n 2:204840:2252839 -t 2:0700 -c 2:"YANTRIK-backup" "$DIMG" >/dev/null
sgdisk -a 8 -n 3:2252840:4194270 -t 3:0700 -c 3:"YANTRIK" "$DIMG" >/dev/null
DECOY=$(losetup -fP --show "$DIMG")
sleep 1
mkfs.fat -F32 -n EFI "${DECOY}p1" >/dev/null
# A FAT label holds 11 characters: YANTRIK-backup is YANTRIK-BAK on the volume.
mkfs.fat -F32 -n YANTRIK-BAK "${DECOY}p2" >/dev/null
mkfs.fat -F32 -n YANTRIK "${DECOY}p3" >/dev/null
for n in 2 3; do
    mount "${DECOY}p$n" "$MNT"; mkdir -p "$MNT/Photos"; head -c 500000 /dev/urandom > "$MNT/Photos/IMG_0001.HEIC"; umount "$MNT"
done
sync
DECOY_SUM=$(sha256sum < "$DECOY" | cut -d' ' -f1)
D=$(basename "$DECOY")

echo "== 4a. YANTRIK, unencrypted: wipefs, then parted type (the text installer's path)"
make_mac
P4_INFO=$(sgdisk -i 4 "$LOOP" | grep -E 'First sector|Last sector|unique GUID|Partition name')

# A file a person put on YANTRIK: refused, and named. Taken off again: offered.
mount "${LOOP}p4" "$MNT"; mkdir -p "$MNT/Photos"; echo photo > "$MNT/Photos/IMG_0002.HEIC"; umount "$MNT"
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
seg "$WORK/scan.json" "${M}p4" reason | grep -qF 'holds files (Photos, Photos/IMG_0002.HEIC)' \
    || { seg "$WORK/scan.json" "${M}p4" reason; fail "YANTRIK holding a person's file was not refused by name"; }
mount "${LOOP}p4" "$MNT"; rm -rf "$MNT/Photos"; umount "$MNT"
pass "YANTRIK holding a file is refused and names it; .fseventsd, .Spotlight-V100, .TemporaryItems, .Trashes, .VolumeIcon.icns, ._* and .DS_Store are not counted"

refuse_on_mac
[ "$(seg "$WORK/scan.json" "${M}p4" kind)" = placeholder ] && [ "$(seg "$WORK/scan.json" "${M}p4" eligible)" = true ] \
    || { seg "$WORK/scan.json" "${M}p4" reason; fail "YANTRIK on the Mac's disk is not offered"; }
[ "$(seg "$WORK/scan.json" "${M}p4" card)" = "Install into ${LOOP}p4 (199 GB, YANTRIK, FAT32); nothing else changes" ] \
    || fail "the card does not name the device: $(seg "$WORK/scan.json" "${M}p4" card)"
[ "$(seg "$WORK/scan.json" "${M}p3" kind)" = medium ] || fail "YKINSTALL was not seen as the installer's own medium"
[ "$(seg "$WORK/scan.json" "${M}p2" size)" = "792 GB" ] || fail "the APFS stand-in is not 792 GB"
for p in "${D}p1" "${D}p2" "${D}p3"; do
    [ "$(seg "$WORK/scan.json" "$p" eligible)" = false ] || fail "the decoy's $p is offered"
done
PRE=$("$BIN" preselect --uefi "$LOOP" "$DECOY" | jq -r '.preselect // empty')
[ "$PRE" = "${M}p4" ] || fail "preselected '$PRE', not ${M}p4"
echo "   fingerprint $FP (decoy $DFP), preselected $PRE"
pass "APFS, the EFI partition, YKINSTALL and both of the decoy's partitions are refused, each for its reason, and nothing changed"

"$BIN" plan --uefi "${M}p4" "$FP" > "$WORK/plan.json"
grep -q mklabel "$WORK/plan.json" && fail "the plan rewrites the table"
jq -e --arg d "${LOOP}p4" '.commands[0] == ["wipefs", "-a", $d]' "$WORK/plan.json" >/dev/null \
    || { cat "$WORK/plan.json"; fail "the plan does not start by wiping the placeholder"; }
"$BIN" apply --uefi "${M}p4" "$FP" > "$WORK/apply.json"
ROOT=$(jq -r .root "$WORK/apply.json")
[ "$ROOT" = "${LOOP}p4" ] || fail "the root reported is $ROOT, not ${LOOP}p4"
[ "$(jq -r .esp "$WORK/apply.json")" = "${LOOP}p1" ] || fail "the EFI partition reported is not ${LOOP}p1"
mkfs.ext4 -q -F -L YANTRIK "$ROOT"
install_boot "installing into YANTRIK, unencrypted"
sync
mac_unchanged "installing into YANTRIK, unencrypted"
[ "$(sgdisk -i 4 "$LOOP" | grep -E 'First sector|Last sector|unique GUID|Partition name')" = "$P4_INFO" ] \
    || fail "the target moved, or was recreated, instead of being reformatted where it is"
sgdisk -i 4 "$LOOP" | grep -q "$LINUX_TYPE" || fail "the target is not typed Linux filesystem"
[ "$(part_count)" = 4 ] || fail "the disk does not have exactly its four partitions"
[ "$(blkid -p -s TYPE -o value "$ROOT")" = ext4 ] || fail "the root is not ext4"
pass "4a: YANTRIK wiped, retyped and formatted where it is; EFI/BOOT exactly $FALLBACK_SET and YANTRIK.OWN; APFS head and tail, the EFI partition and its other files, the disk GUID, partitions 1-3, YKINSTALL and the decoy disk unchanged"
drop_mac

echo "== 4b. YANTRIK, encrypted: wipefs, rm, then /boot and root inside its extent (the desktop default)"
make_mac
refuse_on_mac
"$BIN" plan --uefi --encrypt "${M}p4" "$FP" > "$WORK/plan.json"
grep -q mklabel "$WORK/plan.json" && fail "the plan rewrites the table"
jq -e '.commands[1] | join(" ") == "parted -s '"$LOOP"' rm 4"' "$WORK/plan.json" >/dev/null \
    || { cat "$WORK/plan.json"; fail "the encrypted plan does not remove the placeholder second"; }
"$BIN" apply --uefi --encrypt "${M}p4" "$FP" > "$WORK/apply.json"
ROOT=$(jq -r .root "$WORK/apply.json")
BOOT=$(jq -r .boot "$WORK/apply.json")
[ -b "$ROOT" ] && [ -b "$BOOT" ] || fail "no root ($ROOT) or /boot ($BOOT)"
[ "$(jq -r .esp "$WORK/apply.json")" = "${LOOP}p1" ] || fail "the EFI partition reported is not ${LOOP}p1"
# What the desktop installer does next: /boot as ext4, the root as LUKS2.
mkfs.ext4 -q -F -L YANTRIK_BOOT "$BOOT"
if command -v cryptsetup >/dev/null; then
    head -c 32 /dev/urandom > "$WORK/key"
    cryptsetup luksFormat --type luks2 --batch-mode --pbkdf pbkdf2 --pbkdf-force-iterations 1000 --key-file "$WORK/key" "$ROOT"
    [ "$(blkid -p -s TYPE -o value "$ROOT")" = crypto_LUKS ] || fail "the root is not LUKS"
    LUKS="LUKS2 made on the root"
else
    mkfs.ext4 -q -F -L YANTRIK "$ROOT"
    LUKS="cryptsetup is not installed here, so the root was made ext4 in place of LUKS2"
    echo "   NOTE: $LUKS"
fi
install_boot "installing into YANTRIK, encrypted"
sync
mac_unchanged "installing into YANTRIK, encrypted"
[ "$(part_count)" = 5 ] || fail "the disk does not have exactly five partitions (the four, with YANTRIK made two)"
for dev in "$BOOT" "$ROOT"; do
    n=${dev##*p}
    s=$(first_sector "$n"); e=$(last_sector "$n")
    [ "$s" -ge 1563433928 ] && [ "$e" -le 1952105802 ] || fail "$dev ($s-$e) is outside YANTRIK's extent 1563433928-1952105802"
done
[ "$(first_sector "${BOOT##*p}")" = 1563433928 ] && [ "$(last_sector "${ROOT##*p}")" = 1952105802 ] \
    || fail "/boot and root do not fill exactly YANTRIK's old extent"
[ "$(blockdev --getsize64 "$BOOT")" -ge $((1024 * MIB)) ] || fail "/boot is under 1 GiB"
pass "4b: /boot and root made inside exactly YANTRIK's old extent ($LUKS); EFI/BOOT exactly $FALLBACK_SET and YANTRIK.OWN; APFS head and tail, the EFI partition and its other files, the disk GUID, partitions 1-3, YKINSTALL and the decoy disk unchanged"
drop_mac
losetup -d "$DECOY"; DECOY=""
rm -f "$DIMG"

# ── 5. Disks refused whole ───────────────────────────────────────────────────────────────
echo "== a hybrid MBR, a full EFI partition, an empty partition that is not blank"
make_disk hybrid -a 8 -n 3:1196072:44040191 -t 3:0700 -c 3:YANTRIK
mkfs.fat -F32 -n YANTRIK "${LOOP}p3" >/dev/null
sgdisk -h 3 "$LOOP" >/dev/null
partprobe "$LOOP" 2>/dev/null || true
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
[ "$(jq '[.[] | .segments[] | select(.eligible)] | length' "$WORK/scan.json")" = 0 ] || { cat "$WORK/scan.json"; fail "a disk with a hybrid MBR offered something"; }
jq -r '.[0].problem' "$WORK/scan.json" | grep -qF 'hybrid MBR' || fail "the hybrid MBR was not named"
losetup -d "$LOOP"; LOOP=""

make_disk fullesp -a 8 -n 3:1196072:44040191 -t 3:0700 -c 3:YANTRIK
mkfs.fat -F32 -n YANTRIK "${LOOP}p3" >/dev/null
mount "${LOOP}p1" "$MNT"; head -c $((180 * 1000 * 1000)) /dev/urandom > "$MNT/EFI/APPLE/big.bin" || true; umount "$MNT"
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
[ "$(jq '[.[] | .segments[] | select(.eligible)] | length' "$WORK/scan.json")" = 0 ] || { cat "$WORK/scan.json"; fail "a disk whose EFI partition is full offered something"; }
jq -r '.[0].problem' "$WORK/scan.json" | grep -qF 'needs 32 MB' || fail "the full EFI partition was not named"
losetup -d "$LOOP"; LOOP=""

make_disk dirty -a 8 -n 3:1196072:44040191 -t 3:0700 -c 3:data
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
[ "$(seg "$WORK/scan.json" "$(basename "$LOOP")p3" kind)" = unformatted ] \
    || { seg "$WORK/scan.json" "$(basename "$LOOP")p3" reason; fail "a blank partition was not read as empty"; }
dd if=/dev/urandom of="${LOOP}p3" bs=4096 count=1 seek=100 status=none
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
seg "$WORK/scan.json" "$(basename "$LOOP")p3" reason | grep -qF 'first MiB is not blank' \
    || { seg "$WORK/scan.json" "$(basename "$LOOP")p3" reason; fail "a partition with bytes in its first MiB was offered as empty"; }
losetup -d "$LOOP"; LOOP=""
pass "a hybrid MBR, an EFI partition with under 32 MB free, and an empty partition with bytes in it are refused"

# ── 6. Another system's files in \EFI\BOOT, and a re-install over Yantrik's own ────────────
echo "== EFI/BOOT holding another system's files; a re-install over Yantrik's own"
make_disk efiboot
boot_sums() { (cd "$MNT/EFI/BOOT" && find . -type f -exec sha256sum {} + | LC_ALL=C sort); }
mount "${LOOP}p1" "$MNT"
fill_yantrik "$MNT"
for foreign in grub.cfg grubx64.efi BOOTX64.EFI bootx64.efi fbx64.efi; do
    rm -rf "$MNT/EFI/BOOT"; mkdir -p "$MNT/EFI/BOOT"
    head -c 4096 /dev/urandom > "$MNT/EFI/BOOT/$foreign"
    echo "left by another system" > "$MNT/EFI/BOOT/BOOTX64.CSV"
    BEFORE=$(boot_sums)
    "$BIN" efi-fallback check --apple "$MNT" | jq -e '.kept' >/dev/null || fail "a foreign $foreign was not reported by the check"
    OUT=$("$BIN" efi-fallback write --apple "$MNT")
    jq -r '.kept // empty' <<<"$OUT" | grep -qF "not Yantrik's" || { echo "$OUT"; fail "a foreign $foreign did not stop the write"; }
    [ "$(boot_sums)" = "$BEFORE" ] || fail "a foreign $foreign: EFI/BOOT changed"
    [ ! -e "$MNT/EFI/BOOT/YANTRIK.OWN" ] || fail "a foreign $foreign: YANTRIK.OWN was written"
    echo "   kept: $(jq -r .kept <<<"$OUT")"
done
pass "another system's grub.cfg, grubx64.efi, BOOTX64.EFI (in either case) or fbx64.efi stops the fallback; EFI/BOOT unchanged byte for byte"

rm -rf "$MNT/EFI/BOOT"
OUT=$("$BIN" efi-fallback write --apple "$MNT")
fallback_written "$MNT" "$OUT" "the first install"
head -c 150000 /dev/urandom > "$MNT/EFI/yantrik/grubx64.efi"
OUT=$("$BIN" efi-fallback write --apple "$MNT")
fallback_written "$MNT" "$OUT" "a re-install with a newer GRUB"
echo "another system's" > "$MNT/EFI/BOOT/grub.cfg"
BEFORE=$(boot_sums)
OUT=$("$BIN" efi-fallback write --apple "$MNT")
jq -r '.kept // empty' <<<"$OUT" | grep -qF "grub.cfg that is not Yantrik's" || { echo "$OUT"; fail "our grub.cfg replaced by another was written over"; }
[ "$(boot_sums)" = "$BEFORE" ] || fail "EFI/BOOT changed after its grub.cfg was replaced"
umount "$MNT"
losetup -d "$LOOP"; LOOP=""
pass "a re-install over Yantrik's own set writes it again and records it; once another system replaces one file, nothing is written"

echo "PASS: install into a partition, checked on loop devices"
