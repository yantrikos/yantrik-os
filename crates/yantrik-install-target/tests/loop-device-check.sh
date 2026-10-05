#!/bin/bash
# loop-device-check.sh — the install-into-a-partition planner against real loop devices.
#
# Builds GPT disk images shaped like a Mac mini whose APFS container was shrunk in macOS: an EFI
# system partition with files on it, a stand-in APFS partition (type 7C3457EF-0000-11AA-AA11-
# 00306543ECAC, random bytes) and, after it, free space or a FAT32 placeholder labelled YANTRIK.
# Then runs `yantrik-install-target` exactly as the installers do (scan, plan, apply) and checks:
#
#   - the APFS partition's bytes are unchanged (sha256 before and after);
#   - the EFI partition is never formatted: its filesystem UUID and every file on it are unchanged;
#   - partitions 1 and 2 keep their exact extents and types; the new ones lie inside the target;
#   - the partition table is never rewritten (the disk's GPT GUID is unchanged);
#   - a stale fingerprint, the APFS partition and the EFI partition are refused, with no change.
#
# Run as root, on a machine with loop devices (sparse images, a few hundred MB of real writes):
#   sudo crates/yantrik-install-target/tests/loop-device-check.sh [path/to/yantrik-install-target]
set -euo pipefail

BIN="${1:-${CARGO_TARGET_DIR:-target}/debug/yantrik-install-target}"
[ "$(id -u)" = 0 ] || { echo "run as root (loop devices, parted)" >&2; exit 2; }
[ -x "$BIN" ] || { echo "no yantrik-install-target at $BIN; cargo build -p yantrik-install-target" >&2; exit 2; }
for tool in losetup sgdisk parted mkfs.fat mkfs.ext4 blkid sha256sum wipefs; do
    command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 2; }
done
[ -e /dev/loop-control ] || { echo "no loop devices here" >&2; exit 2; }

WORK=$(mktemp -d /var/tmp/yantrik-partcheck.XXXXXX)
LOOP=""
MNT="$WORK/mnt"
mkdir -p "$MNT"
cleanup() {
    mountpoint -q "$MNT" && umount "$MNT"
    [ -n "$LOOP" ] && losetup -d "$LOOP" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }
APFS_TYPE=7C3457EF-0000-11AA-AA11-00306543ECAC

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
    mkfs.fat -F32 -n EFI "${LOOP}p1" >/dev/null
    mount "${LOOP}p1" "$MNT"
    mkdir -p "$MNT/EFI/APPLE/FIRMWARE" "$MNT/EFI/refind"
    head -c 3000000 /dev/urandom > "$MNT/EFI/APPLE/FIRMWARE/MM61.scap"
    echo "macOS firmware log" > "$MNT/EFI/APPLE/log.txt"
    head -c 200000 /dev/urandom > "$MNT/EFI/refind/refind_x64.efi"
    umount "$MNT"
    dd if=/dev/urandom of="${LOOP}p2" bs=1M count=256 status=none
    sync
}

snapshot() {   # what must never change
    APFS_SUM=$(sha256sum < "${LOOP}p2" | cut -d' ' -f1)
    ESP_UUID=$(blkid -p -s UUID -o value "${LOOP}p1")
    mount -o ro "${LOOP}p1" "$MNT"
    ESP_FILES=$(cd "$MNT" && find . -type f -exec sha256sum {} + | sort)
    umount "$MNT"
    DISK_GUID=$(sgdisk -p "$LOOP" | sed -n 's/^Disk identifier (GUID): //p')
    P1=$(sgdisk -i 1 "$LOOP" | grep -E 'GUID code|First sector|Last sector|unique GUID')
    P2=$(sgdisk -i 2 "$LOOP" | grep -E 'GUID code|First sector|Last sector|unique GUID')
}

verify_kept() {   # $1: what was just done
    [ "$(sha256sum < "${LOOP}p2" | cut -d' ' -f1)" = "$APFS_SUM" ] || fail "$1: the APFS partition's bytes changed"
    [ "$(blkid -p -s UUID -o value "${LOOP}p1")" = "$ESP_UUID" ] || fail "$1: the EFI partition was reformatted"
    mount -o ro "${LOOP}p1" "$MNT"
    local now; now=$(cd "$MNT" && find . -type f ! -path './EFI/yantrik/*' -exec sha256sum {} + | sort)
    umount "$MNT"
    [ "$now" = "$ESP_FILES" ] || fail "$1: files on the EFI partition changed"
    [ "$(sgdisk -p "$LOOP" | sed -n 's/^Disk identifier (GUID): //p')" = "$DISK_GUID" ] || fail "$1: the partition table was rewritten"
    [ "$(sgdisk -i 1 "$LOOP" | grep -E 'GUID code|First sector|Last sector|unique GUID')" = "$P1" ] || fail "$1: partition 1 moved"
    [ "$(sgdisk -i 2 "$LOOP" | grep -E 'GUID code|First sector|Last sector|unique GUID')" = "$P2" ] || fail "$1: partition 2 moved"
}

json() { sed -n "s/^ *\"$1\": \"\{0,1\}\([^\",]*\)\"\{0,1\},\{0,1\}$/\1/p" | head -1; }

expect_refusal() {   # $1: what, rest: the command
    local what=$1; shift
    if "$@" >"$WORK/out" 2>&1; then cat "$WORK/out"; fail "$what was not refused"; fi
    echo "   refused: $(tail -1 "$WORK/out")"
}

# ── 1. Free space after the shrunk container ─────────────────────────────────────────────
echo "== free space beside APFS"
make_disk free
snapshot
"$BIN" scan --uefi "$LOOP" > "$WORK/scan.json"
FP=$(json fingerprint < "$WORK/scan.json")
FREE_ID=$(grep -o "\"id\": \"$(basename "$LOOP")@[0-9]*-[0-9]*\"" "$WORK/scan.json" | cut -d'"' -f4 | head -1)
[ -n "$FP" ] && [ -n "$FREE_ID" ] || { cat "$WORK/scan.json"; fail "scan offered no free space"; }
grep -q '"kind": "macos"' "$WORK/scan.json" || fail "the stand-in APFS partition was not read as macOS"
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
sgdisk -i 3 "$LOOP" | grep -q '0FC63DAF-8483-4772-8E79-3D69D8477DE4' || fail "the new partition is not typed Linux filesystem"
pass "free space: one partition at $NEW_START-$NEW_END inside $FREE_START-$FREE_END; APFS bytes, the EFI partition and its files unchanged"
losetup -d "$LOOP"; LOOP=""

# ── 2. A FAT32 YANTRIK placeholder, encrypted layout (/boot and root inside it) ───────────
echo "== placeholder labelled YANTRIK, encrypted layout"
make_disk placeholder -a 8 -n 3:1196072:44040191 -t 3:0700 -c 3:YANTRIK
mkfs.fat -F32 -n YANTRIK "${LOOP}p3" >/dev/null
snapshot
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
for dev in "$BOOT" "$ROOT"; do
    n=${dev##*p}
    s=$(sgdisk -i "$n" "$LOOP" | sed -n 's/^First sector: \([0-9]*\).*/\1/p')
    e=$(sgdisk -i "$n" "$LOOP" | sed -n 's/^Last sector: \([0-9]*\).*/\1/p')
    [ "$s" -ge 1196072 ] && [ "$e" -le 44040191 ] || fail "$dev ($s-$e) is outside the placeholder 1196072-44040191"
done
pass "placeholder: /boot and root made inside its extent; APFS bytes, the EFI partition and its files unchanged"
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
snapshot
BEFORE=$(sgdisk -p "$LOOP")
expect_refusal "installing after the table changed" "$BIN" apply --uefi "$FREE_ID" "$FP"
[ "$(sgdisk -p "$LOOP")" = "$BEFORE" ] || fail "the refused install changed the table"
verify_kept "the refused install"
pass "a changed table is refused before anything is written"

echo "PASS: install into a partition, checked on loop devices"
