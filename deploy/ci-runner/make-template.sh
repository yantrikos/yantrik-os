#!/usr/bin/env bash
# Build the CI runner template: VM 9540 `yantrik-ci-template` on a Proxmox node (#1234, #1252).
#
# Run as root on the node (node2), with the controller stopped. Every CI job runs in a throwaway linked
# clone of this template (see yantrik-ci-controller.sh), so what is baked in here is the only thing a
# job inherits: Debian 13's own kernel, the toolchain, the apt packages and the GitHub Actions runner,
# all root-owned, and an empty `runner` account. README.md says why it is built this way.
#
# The build: bring up the bridge yci0 and its filter (lib/net.sh), import Debian's genericcloud image
# once (checksum-checked, the download deleted), start it on yci0 with a throwaway ssh key, provision it,
# run the proofs, seal it (no sshd, no sudo, no default user), and convert it. The build VM is on the
# same filtered network as a job. Re-running rebuilds from scratch (idempotent): it refuses while any
# clone in 9541-9549 exists, and destroys the old template 9540 first.
#
# It touches VM 9540 only, the bridge yci0 and its table (removed at the end, unless already there), and
# reads the build host's address (CT 530's net0) to prove the LAN is unreachable.

set -euo pipefail

TEMPLATE_ID=9540
ID_MIN=9541
ID_MAX=9549
NAME=yantrik-ci-template
SRC_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
BUILD_CT=530 # the build host: read-only here, to learn an address a job must not reach
STORAGE=local-lvm
POOL=pve/data
DISK_GB=80 # measured: a cold workspace build and the whole test run peak at 23 GB (#1252)
CPUUNITS=50 # CT 530 and the VMs keep the default 100: the build host wins on CPU (review N4)
CORES=32
MEMORY_MB=24576 # 24 GB: the trial that decides whether the runner and the build container can share the node (18 Nov)
BUILD_CORES=8 # the build itself needs less
BUILD_MEMORY_MB=8192
RUST_VERSION=1.92.0
IMAGE_BASE=https://cloud.debian.org/images/cloud/trixie/latest
IMAGE_NAME=debian-13-genericcloud-amd64.qcow2
OWN_PUBLIC_FILE=${YANTRIK_CI_OWN_PUBLIC:-/etc/yantrik-ci/own-public-ips} # our own public addresses, kept on the node
RUN_DIR=${YANTRIK_CI_RUN_DIR:-/run/yantrik-ci}
LIB_DIR=$SRC_DIR/lib
NET_NFT=$SRC_DIR/host/yci.nft
NET_NFT_L2=$SRC_DIR/host/yci-l2.nft

say() { printf '== %s\n' "$*"; }
log() { printf 'make-template: %s\n' "$*" >&2; }
die() { log "$*"; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run as root on the Proxmox node"
command -v qm > /dev/null || die "qm not found: this is not a Proxmox node"
command -v nft > /dev/null || die "nft not found"
systemctl is-active --quiet yantrik-ci-controller && die "the controller is running: stop it first"
install -d -m 700 "$RUN_DIR"
# shellcheck source=lib/trust.sh
. "$SRC_DIR/lib/trust.sh"
# shellcheck source=lib/host.sh
. "$SRC_DIR/lib/host.sh"
# shellcheck source=lib/net.sh
. "$SRC_DIR/lib/net.sh"
net_own || die "list our own public addresses (the router's WAN address, any other), one per line, in $OWN_PUBLIC_FILE"

for ((id = ID_MIN; id <= ID_MAX; id++)); do
  if id_in_use "$id"; then die "clone $id exists: stop the controller and destroy $ID_MIN-$ID_MAX first"; fi
done
if id_in_use "$TEMPLATE_ID"; then
  qm config "$TEMPLATE_ID" 2> /dev/null | grep -q "^name: $NAME\$" || die "id $TEMPLATE_ID is in use and is not $NAME: not touching it"
fi

WORK=$(mktemp -d /var/tmp/yci-template.XXXXXX)
KEY=$WORK/key
NET_WAS_UP=0
net_present && NET_WAS_UP=1
cleanup() {
  rm -rf "$WORK"
  net_unpin "$TEMPLATE_ID" 2> /dev/null || true
  if [ "$NET_WAS_UP" -eq 0 ] && ! vm_running "$TEMPLATE_ID"; then net_down 2> /dev/null || true; fi
}
trap cleanup EXIT

if id_in_use "$TEMPLATE_ID"; then
  say "removing the old template $TEMPLATE_ID"
  qm stop "$TEMPLATE_ID" > /dev/null 2>&1 || true
  qm unlock "$TEMPLATE_ID" > /dev/null 2>&1 || true
  qm destroy "$TEMPLATE_ID" --purge 1 --destroy-unreferenced-disks 1
fi

say "the network: $NET_BRIDGE (no physical port) and the table $NET_TABLE"
net_up || die "the network could not be brought up"

say "the image: $IMAGE_NAME (once, checked, then deleted)"
curl -fsSL -o "$WORK/SHA512SUMS" "$IMAGE_BASE/SHA512SUMS"
curl -fL --progress-bar -o "$WORK/$IMAGE_NAME" "$IMAGE_BASE/$IMAGE_NAME"
want=$(awk -v n="$IMAGE_NAME" '$2 == n {print $1}' "$WORK/SHA512SUMS")
[ -n "$want" ] && [ "$(sha512sum "$WORK/$IMAGE_NAME" | awk '{print $1}')" = "$want" ] || die "the image's checksum does not match"

say "creating VM $TEMPLATE_ID"
# No fixed MAC: Proxmox draws one for this VM and again for every clone. One card, on yci0.
qm create "$TEMPLATE_ID" \
  --name "$NAME" --ostype l26 --machine q35 --cpu x86-64-v2-AES \
  --cores "$BUILD_CORES" --memory "$BUILD_MEMORY_MB" --balloon 0 --cpuunits "$CPUUNITS" \
  --scsihw virtio-scsi-single --net0 "virtio,bridge=$NET_BRIDGE,firewall=0" \
  --agent enabled=1 --serial0 socket --vga serial0 --tablet 0 --onboot 0 \
  --description "Template for throwaway CI runners (#1252). Built by deploy/ci-runner/make-template.sh. Do not edit by hand."
qm importdisk "$TEMPLATE_ID" "$WORK/$IMAGE_NAME" "$STORAGE" > /dev/null
rm -f "$WORK/$IMAGE_NAME"
vol=$(qm config "$TEMPLATE_ID" | sed -n 's/^unused0: //p')
[ -n "$vol" ] || die "the imported disk is not there"
qm set "$TEMPLATE_ID" --scsi0 "$vol,discard=on,ssd=1,iothread=1" --boot order=scsi0 > /dev/null
qm resize "$TEMPLATE_ID" scsi0 "${DISK_GB}G" > /dev/null
ssh-keygen -q -t ed25519 -N '' -C yantrik-ci-build -f "$KEY"
qm set "$TEMPLATE_ID" --scsi1 "$STORAGE:cloudinit" --ciuser debian --ciupgrade 0 --sshkeys "$KEY.pub" \
  --ipconfig0 "ip=$(net_ip "$TEMPLATE_ID")/$NET_LEN,gw=$NET_GW" --nameserver "$NET_RESOLVER" --searchdomain yantrik-ci.invalid > /dev/null

say "starting it (pinned on $NET_BRIDGE like a clone)"
net_pin "$TEMPLATE_ID" || die "cannot pin the build VM"
qm start "$TEMPLATE_ID"
net_lock "$TEMPLATE_ID" || die "cannot isolate and lock the build VM's port"
BUILD_IP=$(net_ip "$TEMPLATE_ID")
SSH=(ssh -i "$KEY" -o IdentitiesOnly=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o BatchMode=yes -o ConnectTimeout=5 -o LogLevel=ERROR "debian@$BUILD_IP")
ok=0
for _ in $(seq 1 90); do
  if "${SSH[@]}" true < /dev/null 2> /dev/null; then ok=1; break; fi
  sleep 3
done
[ "$ok" -eq 1 ] || die "the build VM never answered ssh on $BUILD_IP"
run() { "${SSH[@]}" "$@" < /dev/null; }

say "provisioning inside the VM"
scp -q -i "$KEY" -o IdentitiesOnly=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
  -r "$SRC_DIR/guest" "$OWN_PUBLIC_FILE" "debian@$BUILD_IP:/tmp/"
run sudo env RUST_VERSION="$RUST_VERSION" bash /tmp/guest/provision.sh

say "installing the launcher, the egress filter and the job-started hook"
run sudo bash /tmp/guest/install.sh "/tmp/$(basename "$OWN_PUBLIC_FILE")"

say "proofs before it becomes a template"
NODE_LAN_IP=$(ip -4 -o addr show "$(net_uplink)" | awk '{sub(/\/.*/, "", $4); print $4; exit}')
LAN_IP=$(pct config "$BUILD_CT" 2> /dev/null | sed -n '/^net0:/s/.*ip=\([0-9.]*\).*/\1/p')
[ -n "$NODE_LAN_IP" ] && [ -n "$LAN_IP" ] || die "cannot work out the LAN addresses for the proofs"
proofs=$(run sudo env NODE_LAN_IP="$NODE_LAN_IP" NODE_BR_IP="$NET_GW" LAN_IP="$LAN_IP" bash /tmp/guest/template-proofs.sh) || {
  printf '%s\n' "$proofs"
  die "a proof failed: the build VM $TEMPLATE_ID is left stopped for inspection, not a template"
}
printf '%s\n' "$proofs"

say "sealing (no sshd, no sudo, no default user, no machine id)"
"${SSH[@]}" sudo systemd-run --quiet --no-block bash /tmp/guest/seal.sh < /dev/null
for _ in $(seq 1 60); do
  vm_running "$TEMPLATE_ID" || break
  sleep 3
done
vm_running "$TEMPLATE_ID" && { qm stop "$TEMPLATE_ID" > /dev/null; die "the build VM did not power off after sealing"; }

say "converting to a template"
qm set "$TEMPLATE_ID" --cores "$CORES" --memory "$MEMORY_MB" --delete sshkeys --ciuser root --ciupgrade 0 > /dev/null
# No kernel same-page merging for these VMs: KSM across guests is a cross-VM side channel (review L9).
qm set "$TEMPLATE_ID" --allow-ksm 0 > /dev/null 2>&1 || log "this PVE has no --allow-ksm: KSM stays as the node has it"
qm cloudinit update "$TEMPLATE_ID" > /dev/null 2>&1 || true
qm template "$TEMPLATE_ID"
qm config "$TEMPLATE_ID" | grep -E '^(template|name|cores|cpu|cpuunits|memory|scsi0|scsi1|net0|agent|machine|allow-ksm|ciupgrade)'
say "template $TEMPLATE_ID ready"
