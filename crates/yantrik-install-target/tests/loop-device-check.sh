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
#     partition are refused, with no change.
#
# Section 4 is the Mac this is for, at its real size (a sparse 1 TB image): the 209.7 MB EFI
# partition at sector 40, APFS of 792 GB, the 8 GB YKINSTALL partition the installer runs from
# (mounted at /run/live/medium, as the live system mounts it), and the 199 GB FAT32 partition
# labelled YANTRIK; beside it a decoy disk whose FAT partitions, one labelled YANTRIK-BAK and one
# labelled exactly YANTRIK, hold files. It runs the unencrypted placeholder path (wipefs, then
# parted type), as the text installer does, and checks the decoy is untouched byte for byte.
# Section 5 is the disks refused whole: a hybrid MBR, an EFI partition too full for a loader,
# and an empty partition that is not blank.
#
# Run as root, on a machine with loop devices (sparse images, a few hundred MB of real writes):
#   sudo crates/yantrik-install-target/tests/loop-device-check.sh [path/to/yantrik-install-target]
set -euo pipefail

BIN="${1:-${CARGO_TARGET_DIR:-target}/debug/yantrik-install-target}"
[ "$(id -u)" = 0 ] || { echo "run as root (loop devices, parted)" >&2; exit 2; }
[ -x "$BIN" ] || { echo "no yantrik-install-target at $BIN; cargo build -p yantrik-install-target" >&2; exit 2; }
for tool in losetup sgdisk parted mkfs.fat mkfs.ext4 blkid sha256sum wipefs blockdev od; do
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

apfs_ends() {
    local size; size=$(blockdev --getsize64 "${LOOP}p2")
    dd if="${LOOP}p2" bs=1M count=64 status=none | sha256sum | cut -d' ' -f1
    dd if="${LOOP}p2" bs=1M count=64 skip=$((size - 64 * MIB)) iflag=skip_bytes status=none | sha256sum | cut -d' ' -f1
}

esp_files() {
    mount -o ro "${LOOP}p1" "$MNT"
    (cd "$MNT" && find . -type f ! -path './EFI/yantrik/*' -exec sha256sum {} + | sort)
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

json() { sed -n "s/^ *\"$1\": \"\{0,1\}\([^\",]*\)\"\{0,1\},\{0,1\}$/\1/p" | head -1; }

expect_refusal() {   # $1: what, rest: the command
    local what=$1; shift
    if "$@" >"$WORK/out" 2>&1; then cat "$WORK/out"; fail "$what was not refused"; fi
    echo "   refused: $(tail -1 "$WORK/out")"
}

# The segment `$2` of the scan in file $1, on one line, for grep.
segment() { tr -d '\n' < "$1" | grep -o "{[^{}]*\"id\": \"$2\"[^{}]*}"; }

# ── 1. Free space after the shrunk container ─────────────────────────────────────────────
echo "== free space beside APFS"
make_disk free
snapshot "1 2"
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
FP=$(json fingerprint < "$WORK/scan.json")
FREE_ID=$(grep -o "\"id\": \"$(basename "$LOOP")@[0-9]*-[0-9]*\"" "$WORK/scan.json" | cut -d'"' -f4 | head -1)
[ -n "$FP" ] && [ -n "$FREE_ID" ] || { cat "$WORK/scan.json"; fail "scan offered no free space"; }
grep -q '"kind": "macos"' "$WORK/scan.json" || fail "the stand-in APFS partition was not read as macOS"
segment "$WORK/scan.json" "$FREE_ID" | grep -q '"eligible": true' || { cat "$WORK/scan.json"; fail "the free space is not offered"; }
echo "   fingerprint $FP, target $FREE_ID"

expect_refusal "installing over APFS" "$BIN" apply --uefi "$(basename "$LOOP")p2" "$FP"
expect_refusal "installing over the EFI partition" "$BIN" apply --uefi "$(basename "$LOOP")p1" "$FP"
expect_refusal "a stale fingerprint" "$BIN" apply --uefi "$FREE_ID" 0123456789abcdef
verify_kept "the refusals"
pass "APFS, the EFI partition and a stale fingerprint are refused, and nothing changed"

"$BIN" plan --uefi "$FREE_ID" "$FP" > "$WORK/plan.json"
grep -q mklabel "$WORK/plan.json" && fail "the plan rewrites the table"
"$BIN" apply --uefi "$FREE_ID" "$FP" > "$WORK/apply.json"
ROOT=$(json root < "$WORK/apply.json")
ESP=$(json esp < "$WORK/apply.json")
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
NEW_START=$(sgdisk -i 3 "$LOOP" | sed -n 's/^First sector: \([0-9]*\).*/\1/p')
NEW_END=$(sgdisk -i 3 "$LOOP" | sed -n 's/^Last sector: \([0-9]*\).*/\1/p')
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
FP=$(json fingerprint < "$WORK/scan.json")
grep -q '"kind": "placeholder"' "$WORK/scan.json" || { cat "$WORK/scan.json"; fail "the placeholder was not offered"; }
PH="$(basename "$LOOP")p3"
"$BIN" plan --uefi --encrypt "$PH" "$FP" > "$WORK/plan.json"
grep -q mklabel "$WORK/plan.json" && fail "the plan rewrites the table"
"$BIN" apply --uefi --encrypt "$PH" "$FP" > "$WORK/apply.json"
ROOT=$(json root < "$WORK/apply.json")
BOOT=$(json boot < "$WORK/apply.json")
[ -b "$ROOT" ] && [ -b "$BOOT" ] || fail "no root ($ROOT) or /boot ($BOOT)"
verify_kept "installing into the placeholder"
[ "$(sha256sum < "${LOOP}p4" | cut -d' ' -f1)" = "$AFTER_SUM" ] || fail "the partition after the placeholder changed"
for dev in "$BOOT" "$ROOT"; do
    n=${dev##*p}
    s=$(sgdisk -i "$n" "$LOOP" | sed -n 's/^First sector: \([0-9]*\).*/\1/p')
    e=$(sgdisk -i "$n" "$LOOP" | sed -n 's/^Last sector: \([0-9]*\).*/\1/p')
    [ "$s" -ge 1196072 ] && [ "$e" -le 44040191 ] || fail "$dev ($s-$e) is outside the placeholder 1196072-44040191"
done
pass "placeholder: /boot and root made inside its extent; the partitions before and after it, APFS and the EFI partition unchanged"
losetup -d "$LOOP"; LOOP=""

# ── 3. The table changes between showing it and installing ───────────────────────────────
echo "== the table changes after it was shown"
make_disk changed
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
FP=$(json fingerprint < "$WORK/scan.json")
FREE_ID=$(grep -o "\"id\": \"$(basename "$LOOP")@[0-9]*-[0-9]*\"" "$WORK/scan.json" | cut -d'"' -f4 | head -1)
# Something else adds a partition in the free space before Install is pressed.
sgdisk -n 3:2000000:3000000 -t 3:0700 "$LOOP" >/dev/null
partprobe "$LOOP" 2>/dev/null || true
snapshot "1 2 3"
BEFORE=$(sgdisk -p "$LOOP")
expect_refusal "installing after the table changed" "$BIN" apply --uefi "$FREE_ID" "$FP"
[ "$(sgdisk -p "$LOOP")" = "$BEFORE" ] || fail "the refused install changed the table"
verify_kept "the refused install"
pass "a changed table is refused before anything is written"
losetup -d "$LOOP"; LOOP=""

# ── 4. The Mac as it is, with a decoy disk beside it ─────────────────────────────────────
echo "== the Mac mini's own layout (1 TB, sparse), and a decoy disk"
IMG="$WORK/mac.img"
truncate -s $((1953525168 * 512)) "$IMG"
sgdisk -o "$IMG" >/dev/null
sgdisk -a 8 -n 1:40:409639 -t 1:EF00 -c 1:"EFI System Partition" "$IMG" >/dev/null
sgdisk -a 8 -n 2:409640:1547284639 -t 2:$APFS_TYPE -c 2:"Container" "$IMG" >/dev/null
sgdisk -a 8 -n 3:1547546784:1563171783 -t 3:0700 -c 3:YKINSTALL "$IMG" >/dev/null
sgdisk -a 8 -n 4:1563433928:1952105802 -t 4:0700 -c 4:YANTRIK "$IMG" >/dev/null
LOOP=$(losetup -fP --show "$IMG")
sleep 1
fill_esp "${LOOP}p1"
fill_apfs "${LOOP}p2"
mkfs.fat -F32 -n YKINSTALL "${LOOP}p3" >/dev/null
mount "${LOOP}p3" "$MNT"; mkdir -p "$MNT/live"; head -c 4000000 /dev/urandom > "$MNT/live/filesystem.squashfs"; umount "$MNT"
mkfs.fat -F32 -n YANTRIK "${LOOP}p4" >/dev/null
# What macOS leaves on a FAT volume it has mounted once: tolerated.
mount "${LOOP}p4" "$MNT"
mkdir -p "$MNT/.fseventsd" "$MNT/.Spotlight-V100/Store-V2" "$MNT/.Trashes/501"
echo x > "$MNT/.fseventsd/fseventsd-uuid"; echo x > "$MNT/.Spotlight-V100/Store-V2/store"; echo x > "$MNT/.DS_Store"
umount "$MNT"
# The installer runs from YKINSTALL, mounted where the live system mounts its medium.
mkdir -p "$MEDIUM"
mount -o ro "${LOOP}p3" "$MEDIUM"
MEDIUM_FILES=$(cd "$MEDIUM" && find . -type f -exec sha256sum {} + | sort)

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
M=$(basename "$LOOP"); D=$(basename "$DECOY")
snapshot "1 2 3"
P4_INFO=$(sgdisk -i 4 "$LOOP" | grep -E 'First sector|Last sector|unique GUID|Partition name')

# A file a person put on YANTRIK: refused, and named. Taken off again: offered.
mount "${LOOP}p4" "$MNT"; mkdir -p "$MNT/Photos"; echo photo > "$MNT/Photos/IMG_0002.HEIC"; umount "$MNT"
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
segment "$WORK/scan.json" "${M}p4" | grep -q 'holds files (Photos, Photos/IMG_0002.HEIC)' \
    || { segment "$WORK/scan.json" "${M}p4"; fail "YANTRIK holding a person's file was not refused by name"; }
mount "${LOOP}p4" "$MNT"; rm -rf "$MNT/Photos"; umount "$MNT"
pass "YANTRIK holding a file is refused and names it; macOS's .fseventsd, .Spotlight-V100, .Trashes and .DS_Store are not counted"

"$BIN" scan --uefi "$LOOP" "$DECOY" > "$WORK/scan.json"
FP=$(json fingerprint < "$WORK/scan.json")
Y=$(segment "$WORK/scan.json" "${M}p4")
echo "$Y" | grep -q '"kind": "placeholder"' && echo "$Y" | grep -q '"eligible": true' || { echo "$Y"; fail "YANTRIK on the Mac's disk is not offered"; }
echo "$Y" | grep -q "\"card\": \"Install into ${LOOP}p4 (199 GB, YANTRIK, FAT32); nothing else changes\"" || { echo "$Y"; fail "the card does not name the device"; }
segment "$WORK/scan.json" "${M}p3" | grep -q '"kind": "medium"' || fail "YKINSTALL was not seen as the installer's own medium"
segment "$WORK/scan.json" "${M}p2" | grep -q '"size": "792 GB"' || fail "the APFS stand-in is not 792 GB"
for p in "${D}p1" "${D}p2" "${D}p3"; do
    segment "$WORK/scan.json" "$p" | grep -q '"eligible": false' || { segment "$WORK/scan.json" "$p"; fail "the decoy's $p is offered"; }
done
segment "$WORK/scan.json" "${D}p2" | grep -q 'labelled YANTRIK-BAK' || fail "the decoy's YANTRIK-BAK was not refused for its label"
segment "$WORK/scan.json" "${D}p3" | grep -q 'holds files' || fail "the decoy's YANTRIK was not refused for its files"
PRE=$("$BIN" preselect --uefi "$LOOP" "$DECOY" | json preselect)
[ "$PRE" = "${M}p4" ] || fail "preselected $PRE, not ${M}p4"
echo "   fingerprint $FP, preselected $PRE"

expect_refusal "installing over the APFS stand-in" "$BIN" apply --uefi "${M}p2" "$FP"
expect_refusal "installing over the EFI partition" "$BIN" apply --uefi "${M}p1" "$FP"
expect_refusal "installing over YKINSTALL, the installer's own" "$BIN" apply --uefi "${M}p3" "$FP"
DFP=$(tr -d '\n' < "$WORK/scan.json" | grep -o "\"disk\": \"$DECOY\"[^\[]*\"fingerprint\": \"[0-9a-f]*\"" | sed 's/.*"fingerprint": "//; s/"$//')
expect_refusal "installing over the decoy's YANTRIK-BAK" "$BIN" apply --uefi "${D}p2" "$DFP"
expect_refusal "installing over the decoy's YANTRIK with files" "$BIN" apply --uefi "${D}p3" "$DFP"
verify_kept "the refusals on the Mac"
pass "APFS, the EFI partition, YKINSTALL and both of the decoy's partitions are refused, and nothing changed"

"$BIN" plan --uefi "${M}p4" "$FP" > "$WORK/plan.json"
grep -q mklabel "$WORK/plan.json" && fail "the plan rewrites the table"
tr -d ' \n' < "$WORK/plan.json" | grep -q "\[\"wipefs\",\"-a\",\"${LOOP}p4\"\]" \
    || { cat "$WORK/plan.json"; fail "the plan does not wipe the placeholder"; }
# The unencrypted path the text installer takes: wipefs, parted type, then mkfs.ext4 by the caller.
"$BIN" apply --uefi "${M}p4" "$FP" > "$WORK/apply.json"
ROOT=$(json root < "$WORK/apply.json")
[ "$ROOT" = "${LOOP}p4" ] || fail "the root reported is $ROOT, not ${LOOP}p4"
[ "$(json esp < "$WORK/apply.json")" = "${LOOP}p1" ] || fail "the EFI partition reported is not ${LOOP}p1"
mkfs.ext4 -q -F -L YANTRIK "$ROOT"
mount "${LOOP}p1" "$MNT"; mkdir -p "$MNT/EFI/yantrik"; head -c 150000 /dev/urandom > "$MNT/EFI/yantrik/grubx64.efi"; umount "$MNT"
sync
verify_kept "installing into YANTRIK"
[ "$(sgdisk -i 4 "$LOOP" | grep -E 'First sector|Last sector|unique GUID|Partition name')" = "$P4_INFO" ] \
    || fail "the target moved, or was recreated, instead of being reformatted where it is"
sgdisk -i 4 "$LOOP" | grep -q "$LINUX_TYPE" || fail "the target is not typed Linux filesystem"
[ "$(sgdisk -p "$LOOP" | grep -cE '^ +[0-9]+ ')" = 4 ] || fail "the disk does not have exactly its four partitions"
[ "$(blkid -p -s TYPE -o value "$ROOT")" = ext4 ] || fail "the root is not ext4"
[ "$(cd "$MEDIUM" && find . -type f -exec sha256sum {} + | sort)" = "$MEDIUM_FILES" ] || fail "YKINSTALL's files changed"
[ "$(sha256sum < "$DECOY" | cut -d' ' -f1)" = "$DECOY_SUM" ] || fail "the decoy disk changed"
pass "the Mac: YANTRIK wiped, retyped and formatted where it is; APFS head and tail, the EFI partition and its files, the disk GUID, YKINSTALL and the decoy disk unchanged"
umount "$MEDIUM"
losetup -d "$DECOY"; DECOY=""
losetup -d "$LOOP"; LOOP=""
rm -f "$IMG" "$DIMG"

# ── 5. Disks refused whole ───────────────────────────────────────────────────────────────
echo "== a hybrid MBR, a full EFI partition, an empty partition that is not blank"
make_disk hybrid -a 8 -n 3:1196072:44040191 -t 3:0700 -c 3:YANTRIK
mkfs.fat -F32 -n YANTRIK "${LOOP}p3" >/dev/null
sgdisk -h 3 "$LOOP" >/dev/null
partprobe "$LOOP" 2>/dev/null || true
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
grep -q '"eligible": true' "$WORK/scan.json" && { cat "$WORK/scan.json"; fail "a disk with a hybrid MBR offered something"; }
grep -q 'hybrid MBR' "$WORK/scan.json" || fail "the hybrid MBR was not named"
losetup -d "$LOOP"; LOOP=""

make_disk fullesp -a 8 -n 3:1196072:44040191 -t 3:0700 -c 3:YANTRIK
mkfs.fat -F32 -n YANTRIK "${LOOP}p3" >/dev/null
mount "${LOOP}p1" "$MNT"; head -c $((180 * 1000 * 1000)) /dev/urandom > "$MNT/EFI/APPLE/big.bin" || true; umount "$MNT"
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
grep -q '"eligible": true' "$WORK/scan.json" && { cat "$WORK/scan.json"; fail "a disk whose EFI partition is full offered something"; }
grep -q 'needs 32 MB' "$WORK/scan.json" || fail "the full EFI partition was not named"
losetup -d "$LOOP"; LOOP=""

make_disk dirty -a 8 -n 3:1196072:44040191 -t 3:0700 -c 3:data
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
segment "$WORK/scan.json" "$(basename "$LOOP")p3" | grep -q '"kind": "unformatted"' \
    || { segment "$WORK/scan.json" "$(basename "$LOOP")p3"; fail "a blank partition was not read as empty"; }
dd if=/dev/urandom of="${LOOP}p3" bs=4096 count=1 seek=100 status=none
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
segment "$WORK/scan.json" "$(basename "$LOOP")p3" | grep -q 'first MiB is not blank' \
    || { segment "$WORK/scan.json" "$(basename "$LOOP")p3"; fail "a partition with bytes in its first MiB was offered as empty"; }
losetup -d "$LOOP"; LOOP=""
pass "a hybrid MBR, an EFI partition with under 32 MB free, and an empty partition with bytes in it are refused"

echo "PASS: install into a partition, checked on loop devices"
