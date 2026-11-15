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
# It touches CT 9530 only.

set -euo pipefail

TEMPLATE_ID=9530
CLONE_MIN=9600
CLONE_MAX=9699
HOSTNAME_CT=yantrik-ci-template
STORAGE=local-lvm
ROOTFS_GB=60
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

# Host-wide hardening the review asks for (M2). These sysctls are not namespaced: an
# unprivileged CT reads the host's value, so they are set here, on the node.
say "host sysctls (fs.protected_*)"
cat > /etc/sysctl.d/90-yantrik-ci.conf <<'EOF'
# Yantrik CI (#1234, review M2): a job in a throwaway CT cannot plant a symlink, file or
# fifo in a sticky world-writable directory for another user's process to follow.
fs.protected_symlinks = 1
fs.protected_hardlinks = 1
fs.protected_regular = 2
fs.protected_fifos = 2
EOF
sysctl -q -p /etc/sysctl.d/90-yantrik-ci.conf

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
  --description "Template for throwaway CI runners (#1234). Built by deploy/ci-runner/make-template.sh. Do not edit by hand."
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
  cmake clang libclang-dev xz-utils zstd unzip procps
apt-get clean
rm -rf /var/lib/apt/lists/*

# --- the unprivileged account every job runs as --------------------------------------
if id runner > /dev/null 2>&1; then userdel -r runner || true; fi
useradd --uid 1001 --create-home --home-dir /home/runner --shell /bin/bash runner
passwd -l runner > /dev/null
# No sudo, no groups beyond its own.
if getent group sudo | grep -qw runner; then gpasswd -d runner sudo; fi
command -v sudo > /dev/null && echo "note: sudo is installed; runner is not in sudoers" || true

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

# --- the launcher: reads the one-job config once, deletes it, runs the runner as `runner` ----
cat > /usr/local/sbin/yantrik-ci-launch <<'EOF'
#!/bin/bash
# Run by the controller (as root, via `pct exec`). The JIT config was pushed to a root-only
# file a moment ago; it is read once and deleted before anything else happens, so the job
# never finds it on disk.
set -euo pipefail
f=/run/yantrik-ci/jit
[ -f "$f" ] || { echo "yantrik-ci-launch: no config" >&2; exit 70; }
[ "$(stat -c '%u %a' "$f")" = "0 600" ] || { echo "yantrik-ci-launch: bad config perms" >&2; exit 71; }
cfg=$(< "$f")
rm -f "$f"
rmdir /run/yantrik-ci 2> /dev/null || true

# The listener keeps its working files (its credentials, _work, _diag) in its own directory,
# and finds that directory from where its binary really is, so the job gets a copy of the
# runner. /opt/actions-runner stays root-owned and untouched; what the job does to its copy
# dies with the container.
rm -rf /home/runner/actions-runner
cp -a /opt/actions-runner /home/runner/actions-runner
chown -R runner:runner /home/runner/actions-runner
cd /home/runner/actions-runner
exec runuser -u runner -- env -i \
  HOME=/home/runner USER=runner LOGNAME=runner SHELL=/bin/bash LANG=C.UTF-8 \
  RUSTUP_HOME=/opt/rust/rustup \
  PATH=/opt/rust/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  /home/runner/actions-runner/run.sh --jitconfig "$cfg"
EOF
chmod 755 /usr/local/sbin/yantrik-ci-launch

# --- the rest of the job's world -----------------------------------------------------
mkdir -p /var/lib/yantrik/workshops
chmod 755 /var/lib/yantrik /var/lib/yantrik/workshops

# Documented here, enforced on the node (they are host-wide; see make-template.sh).
cat > /etc/sysctl.d/90-yantrik-ci.conf <<'EOF'
fs.protected_symlinks = 1
fs.protected_hardlinks = 1
fs.protected_regular = 2
fs.protected_fifos = 2
EOF
PROVISION

say "proofs before it becomes a template"
pct exec "$TEMPLATE_ID" -- bash -s <<'PROOF'
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
rv=$(runuser -u runner -- bash -lc 'rustc --version' 2>&1 || true)
case "$rv" in "rustc 1.92.0 "*) echo "PASS  rustc is 1.92.0 for a login shell";; *) echo "FAIL  rustc: $rv"; fail=1;; esac
exit $fail
PROOF

say "stopping and converting to a template"
pct shutdown "$TEMPLATE_ID" --timeout 60 || pct stop "$TEMPLATE_ID"
pct template "$TEMPLATE_ID"
pct config "$TEMPLATE_ID" | grep -E '^(template|cores|memory|swap|rootfs|net0|features|unprivileged)'
say "template $TEMPLATE_ID ready"
