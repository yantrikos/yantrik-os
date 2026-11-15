#!/usr/bin/env bash
# Build the CI runner template: CT 9530 `yantrik-ci-template` on a Proxmox node (#1234).
#
# Run as root on the node (node2). Every CI job runs in a throwaway clone of this template
# (see yantrik-ci-controller.sh), so what is baked in here is the only thing a job inherits:
# the toolchain, the apt packages and the GitHub Actions runner, all root-owned, and an
# empty `runner` account. See README.md for why it is built this way.
#
# Re-running rebuilds the template from scratch. It refuses to run while any clone in the
# 9600-9699 range exists (a clone is linked to its template and would block the rebuild).
#
# It touches CT 9530 only (and reads CT 530's address for the firewall proofs).

set -euo pipefail

TEMPLATE_ID=9530
CLONE_MIN=9600
CLONE_MAX=9699
HOSTNAME_CT=yantrik-ci-template
SRC_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
BUILD_CT=530 # the build host: read-only here, to learn the addresses a job must not reach
STORAGE=local-lvm
ROOTFS_GB=150
CPUUNITS=50 # CT 530 and the VMs keep the default 100: the build host wins on CPU (review N4)
PIDS_MAX=32768
CORES=32
MEMORY_MB=32768
SWAP_MB=8192
BRIDGE=vmbr0
IMAGE=/var/lib/vz/template/cache/ubuntu-24.04-standard_24.04-2_amd64.tar.zst
RUST_VERSION=1.92.0

say() { printf '== %s\n' "$*"; }
die() { printf 'make-template: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run as root on the Proxmox node"
command -v pct > /dev/null || die "pct not found: this is not a Proxmox node"
[ -f "$IMAGE" ] || die "missing image $IMAGE"

# A CT id lives in the cluster-wide config tree; a clone anywhere in the range blocks us.
ct_conf_exists() { compgen -G "/etc/pve/nodes/*/lxc/$1.conf" > /dev/null; }

for ((id = CLONE_MIN; id <= CLONE_MAX; id++)); do
  if ct_conf_exists "$id"; then
    die "clone $id exists: stop the controller and destroy $CLONE_MIN-$CLONE_MAX first"
  fi
done

if ct_conf_exists "$TEMPLATE_ID"; then
  say "removing the old template $TEMPLATE_ID"
  pct stop "$TEMPLATE_ID" > /dev/null 2>&1 || true
  pct destroy "$TEMPLATE_ID" --purge
fi

say "creating CT $TEMPLATE_ID"
# No hwaddr: Proxmox draws a fresh MAC for this CT and again for every clone. DHCP, on the
# same bridge as the build host (CT 530).
pct create "$TEMPLATE_ID" "$IMAGE" \
  --hostname "$HOSTNAME_CT" \
  --ostype ubuntu \
  --unprivileged 1 \
  --features nesting=1 \
  --cores "$CORES" \
  --memory "$MEMORY_MB" \
  --swap "$SWAP_MB" \
  --rootfs "$STORAGE:$ROOTFS_GB" \
  --net0 "name=eth0,bridge=$BRIDGE,ip=dhcp" \
  --onboot 0 \
  --cpuunits "$CPUUNITS" \
  --description "Template for throwaway CI runners (#1234). Built by deploy/ci-runner/make-template.sh. Do not edit by hand."
# A fork bomb in a job is bounded by this, not by the node's pid_max (review N4). Clones copy it.
echo "lxc.cgroup2.pids.max: $PIDS_MAX" >> "/etc/pve/lxc/$TEMPLATE_ID.conf"
pct start "$TEMPLATE_ID"

say "waiting for the network"
ok=0
for _ in $(seq 1 60); do
  if pct exec "$TEMPLATE_ID" -- getent hosts archive.ubuntu.com > /dev/null 2>&1; then ok=1; break; fi
  sleep 2
done
[ "$ok" -eq 1 ] || die "the template CT never got a network (DHCP?)"

say "provisioning inside the CT"
pct exec "$TEMPLATE_ID" -- env RUST_VERSION="$RUST_VERSION" bash -s <<'PROVISION'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

apt-get update
# The native libraries from ci.yml's "System dependencies" step (found by looking for the
# -sys crates in Cargo.lock), then what a hosted runner's image brings that this one must be
# given: a compiler, git, jq, python, bubblewrap. cmake, clang and the archive tools are
# insurance for build scripts that expect the hosted image.
apt-get install -y --no-install-recommends \
  pkg-config libasound2-dev libudev-dev libssl-dev libspeechd-dev libwayland-dev \
  libxkbcommon-dev libfontconfig1-dev libfreetype6-dev libinput-dev libgbm-dev \
  libegl1-mesa-dev bubblewrap \
  git curl ca-certificates python3 jq build-essential \
  cmake clang libclang-dev xz-utils zstd unzip procps nftables
# Nothing may listen in a job's container, and nothing logs in to it (review N9).
apt-get purge -y openssh-server > /dev/null 2>&1 || true
apt-get clean
rm -rf /var/lib/apt/lists/*

# --- the unprivileged account every job runs as --------------------------------------
if id runner > /dev/null 2>&1; then userdel -r runner || true; fi
useradd --uid 1001 --create-home --home-dir /home/runner --shell /bin/bash runner
passwd -l runner > /dev/null
# No sudo, no groups beyond its own.
if getent group sudo | grep -qw runner; then gpasswd -d runner sudo; fi
if command -v sudo > /dev/null; then echo "note: sudo is installed; runner is not in sudoers"; fi

# --- Rust, installed by root, read-only to everyone else -------------------------------
export RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo
mkdir -p /opt/rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal \
  --default-toolchain "$RUST_VERSION" -c clippy -c rustfmt
rm -f /tmp/rustup-init.sh
chown -R root:root /opt/rust
chmod -R go-w /opt/rust

# CARGO_HOME is deliberately not set here: each job's registry lands in its own ~/.cargo.
cat > /etc/profile.d/rust.sh <<'EOF'
export RUSTUP_HOME=/opt/rust/rustup
case ":$PATH:" in *:/opt/rust/cargo/bin:*) ;; *) export PATH="/opt/rust/cargo/bin:$PATH" ;; esac
EOF
chmod 644 /etc/profile.d/rust.sh

# --- the GitHub Actions runner ---------------------------------------------------------
rel=$(curl -fsSL https://api.github.com/repos/actions/runner/releases/latest)
ver=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"].lstrip("v"))' <<<"$rel")
want_sha=$(python3 -c '
import json, re, sys
body = json.load(sys.stdin)["body"]
m = re.search(r"<!-- BEGIN SHA linux-x64 -->([0-9a-f]{64})<!-- END SHA linux-x64 -->", body)
print(m.group(1) if m else "")' <<<"$rel")
[ -n "$want_sha" ] || { echo "no sha256 for the runner in the release notes" >&2; exit 1; }
curl -fsSL -o /tmp/runner.tgz \
  "https://github.com/actions/runner/releases/download/v${ver}/actions-runner-linux-x64-${ver}.tar.gz"
echo "$want_sha  /tmp/runner.tgz" | sha256sum -c -
rm -rf /opt/actions-runner
mkdir -p /opt/actions-runner
tar -xzf /tmp/runner.tgz -C /opt/actions-runner
rm -f /tmp/runner.tgz
/opt/actions-runner/bin/installdependencies.sh
chown -R root:root /opt/actions-runner
chmod -R go-w /opt/actions-runner
echo "$ver" > /opt/actions-runner/.yantrik-runner-version

# --- the rest of the job's world -----------------------------------------------------
mkdir -p /var/lib/yantrik/workshops
chmod 755 /var/lib/yantrik /var/lib/yantrik/workshops

PROVISION

say "installing the launcher, the egress filter and the job-started hook"
push() { pct push "$TEMPLATE_ID" "$SRC_DIR/ct/$1" "$2" --perms "$3" --user 0 --group 0; }
pct exec "$TEMPLATE_ID" -- mkdir -p /etc/yantrik-ci /usr/local/libexec
push egress.nft /etc/yantrik-ci/egress.nft 644
push yantrik-ci-egress /usr/local/sbin/yantrik-ci-egress 755
push yantrik-ci-launch /usr/local/sbin/yantrik-ci-launch 755
push yantrik-ci-job-started.sh /usr/local/libexec/yantrik-ci-job-started.sh 755
pct exec "$TEMPLATE_ID" -- chown root:root /etc/yantrik-ci /usr/local/libexec

# The addresses a job must not reach, for the proofs: this node, the build host, the gateway.
NODE_IP=$(ip -4 -o addr show "$BRIDGE" | awk '{sub(/\/.*/, "", $4); print $4; exit}')
CT530_NET=$(pct config "$BUILD_CT" | grep '^net0:')
CT530_IP=$(sed -n 's/.*ip=\([0-9.]*\).*/\1/p' <<< "$CT530_NET")
GW_IP=$(sed -n 's/.*gw=\([0-9.]*\).*/\1/p' <<< "$CT530_NET")
[ -n "$NODE_IP" ] && [ -n "$CT530_IP" ] && [ -n "$GW_IP" ] || die "cannot work out the LAN addresses for the proofs"

say "proofs before it becomes a template"
pct exec "$TEMPLATE_ID" -- env NODE_IP="$NODE_IP" CT530_IP="$CT530_IP" GW_IP="$GW_IP" bash -s <<'PROOF'
set -u
fail=0
check() { # check <label> <expected: ok|refused> <cmd...>
  local label=$1 want=$2; shift 2
  if "$@" > /dev/null 2>&1; then got=ok; else got=refused; fi
  if [ "$got" = "$want" ]; then echo "PASS  $label ($got)"; else echo "FAIL  $label (wanted $want, got $got)"; fail=1; fi
}
as_runner() { runuser -u runner -- "$@"; }

check "bwrap works as runner" ok as_runner bwrap --unshare-all --ro-bind / / --dev /dev --proc /proc true
check "runner cannot write /opt/rust" refused as_runner touch /opt/rust/x
check "runner cannot write /opt/rust/cargo/bin" refused as_runner touch /opt/rust/cargo/bin/x
check "runner cannot write /opt/actions-runner" refused as_runner touch /opt/actions-runner/x
check "runner cannot overwrite run.sh" refused as_runner sh -c ': >> /opt/actions-runner/run.sh'
check "runner is not root" ok sh -c '[ "$(runuser -u runner -- id -u)" = 1001 ]'
check "runner has no sudo" refused as_runner sudo -n true
check "runner cannot write the job-started hook" refused as_runner sh -c ': >> /usr/local/libexec/yantrik-ci-job-started.sh'
check "runner cannot replace the hook" refused as_runner mv /usr/local/libexec/yantrik-ci-job-started.sh /usr/local/libexec/x
check "runner cannot write the egress rules" refused as_runner sh -c ': >> /etc/yantrik-ci/egress.nft'

# The hook: it admits ci.yml's test job and nothing else.
hook=/usr/local/libexec/yantrik-ci-job-started.sh
ev=$(mktemp); chmod 644 "$ev"
printf '{"pull_request":{"head":{"repo":{"full_name":"yantrikos/yantrik-os","fork":false}}}}' > "$ev"
hookrun() { as_runner env -i PATH=/usr/bin:/bin "$@" "$hook"; }
base="GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/ci.yml@refs/heads/main GITHUB_JOB=test GITHUB_EVENT_PATH=$ev"
# shellcheck disable=SC2086
check "hook admits push to main" ok hookrun $base GITHUB_EVENT_NAME=push
# shellcheck disable=SC2086
check "hook admits a same-repo PR" ok hookrun $base GITHUB_EVENT_NAME=pull_request
printf '{"pull_request":{"head":{"repo":{"full_name":"evil/yantrik-os","fork":true}}}}' > "$ev"
# shellcheck disable=SC2086
check "hook refuses a fork PR" refused hookrun $base GITHUB_EVENT_NAME=pull_request
# shellcheck disable=SC2086
check "hook refuses another workflow" refused hookrun GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/x.yml@refs/heads/main GITHUB_JOB=test GITHUB_EVENT_NAME=push
check "hook refuses iso.yml (build, sign, publish stay on GitHub)" refused hookrun GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/iso.yml@refs/heads/main GITHUB_JOB=test GITHUB_EVENT_NAME=push
check "hook refuses iso.yml's own job" refused hookrun GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/iso.yml@refs/heads/main GITHUB_JOB=build GITHUB_EVENT_NAME=workflow_dispatch
check "hook refuses a scheduled run" refused hookrun $base GITHUB_EVENT_NAME=schedule
# shellcheck disable=SC2086
check "hook refuses another job" refused hookrun GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/ci.yml@refs/heads/main GITHUB_JOB=clippy GITHUB_EVENT_NAME=push
# shellcheck disable=SC2086
check "hook refuses another repository" refused hookrun GITHUB_REPOSITORY=other/repo GITHUB_JOB=test GITHUB_EVENT_NAME=push
rm -f "$ev"

# The egress filter: loaded the way the launcher loads it. (The template had a network for
# the provisioning above; from here on it has the job's.)
/usr/local/sbin/yantrik-ci-egress
tcp() { timeout 5 bash -c "exec 3<>/dev/tcp/$1/$2"; }
check "no node API (8006)" refused as_runner curl -m5 -sk "https://$NODE_IP:8006/"
check "no node ssh" refused as_runner timeout 5 bash -c "exec 3<>/dev/tcp/$NODE_IP/22"
check "no build host (CT 530) ssh" refused as_runner timeout 5 bash -c "exec 3<>/dev/tcp/$CT530_IP/22"
check "no LAN http" refused as_runner curl -m5 -s "http://$CT530_IP/"
check "no gateway http" refused as_runner curl -m5 -s "http://$GW_IP/"
check "no gateway https" refused as_runner curl -m5 -sk "https://$GW_IP/"
check "no port 80 to the internet" refused as_runner curl -m5 -s http://example.com/
check "no ipv6" refused as_runner curl -m5 -6 -s https://api.github.com/zen
check "GitHub reachable" ok as_runner curl -m10 -sSfo /dev/null https://api.github.com/zen
check "index.crates.io reachable" ok as_runner curl -m10 -sSfo /dev/null https://index.crates.io/config.json
check "static.crates.io reachable" ok as_runner curl -m10 -sSfo /dev/null -r 0-0 https://static.crates.io/crates/libc/libc-0.2.150.crate
check "runner cannot list the rules" refused as_runner nft list ruleset
check "runner cannot flush the rules" refused as_runner nft flush ruleset
check "runner cannot reach the rules from a new namespace" refused as_runner unshare -Urn sh -c 'nft list ruleset && curl -m5 -s https://api.github.com/zen'
rv=$(runuser -u runner -- bash -lc 'rustc --version' 2>&1 || true)
case "$rv" in "rustc 1.92.0 "*) echo "PASS  rustc is 1.92.0 for a login shell";; *) echo "FAIL  rustc: $rv"; fail=1;; esac
exit $fail
PROOF

say "stopping and converting to a template"
pct shutdown "$TEMPLATE_ID" --timeout 60 || pct stop "$TEMPLATE_ID"
pct template "$TEMPLATE_ID"
pct config "$TEMPLATE_ID" | grep -E '^(template|cores|cpuunits|memory|swap|rootfs|net0|features|unprivileged)'
grep -E '^lxc\.cgroup2\.pids\.max' "/etc/pve/lxc/$TEMPLATE_ID.conf"
say "template $TEMPLATE_ID ready"
