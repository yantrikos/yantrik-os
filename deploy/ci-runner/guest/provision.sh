#!/bin/bash
# Provision the CI template VM from inside (#1252, VM design). Run as root by make-template.sh, over a
# throwaway ssh key, on a Debian 13 cloud image that is on the filtered bridge yci0. Everything a job
# inherits is baked in here: the toolchain, the apt packages and the GitHub Actions runner, all
# root-owned, and an empty `runner` account. Every way in (ssh, sudo, the image's default
# user, cloud-init's user module) is removed by seal.sh, which runs after the proofs. Environment:
# RUST_VERSION. The guest files (egress filter, launcher, hook) are installed by make-template.sh.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

# The image's first boot may still hold the apt lock: wait for cloud-init to finish.
cloud-init status --wait > /dev/null 2>&1 || true

# The default Debian mirror speaks http; the egress filter has no port 80. https needs ca-certificates,
# which the image has.
for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.sources /etc/apt/sources.list.d/*.list; do
  [ -f "$f" ] && sed -i 's#http://#https://#g' "$f"
done
apt-get update
# The image's package set is a snapshot: bring it to today's before anything is installed, so the kernel, libc, systemd
# and the virtio drivers of every job's VM are as current as the day of the build (README: rebuild cadence).
apt-get -y full-upgrade
# The native libraries from ci.yml's "System dependencies" step (found by looking for the -sys crates
# in Cargo.lock), then what a hosted runner's image brings that this one must be given: a compiler, git,
# jq, python, bubblewrap. cmake, clang and the archive tools are insurance for build scripts that
# expect the hosted image. qemu-guest-agent is how the controller hands over the job.
apt-get install -y --no-install-recommends \
  pkg-config libasound2-dev libudev-dev libssl-dev libspeechd-dev libwayland-dev \
  libxkbcommon-dev libfontconfig1-dev libfreetype-dev libinput-dev libgbm-dev \
  libegl1-mesa-dev bubblewrap \
  git curl ca-certificates python3 jq build-essential \
  cmake clang libclang-dev xz-utils zstd unzip procps nftables iproute2 \
  qemu-guest-agent

# --- the unprivileged account every job runs as --------------------------------------
if id runner > /dev/null 2>&1; then userdel -r runner || true; fi
useradd --uid 1001 --create-home --home-dir /home/runner --shell /bin/bash runner
passwd -l runner > /dev/null
if getent group sudo | grep -qw runner; then gpasswd -d runner sudo; fi

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
cat > /etc/profile.d/rust.sh << 'EOF'
export RUSTUP_HOME=/opt/rust/rustup
case ":$PATH:" in *:/opt/rust/cargo/bin:*) ;; *) export PATH="/opt/rust/cargo/bin:$PATH" ;; esac
EOF
chmod 644 /etc/profile.d/rust.sh

# --- the GitHub Actions runner ---------------------------------------------------------
rel=$(curl -fsSL https://api.github.com/repos/actions/runner/releases/latest)
ver=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"].lstrip("v"))' <<< "$rel")
want_sha=$(python3 -c '
import json, re, sys
body = json.load(sys.stdin)["body"]
m = re.search(r"<!-- BEGIN SHA linux-x64 -->([0-9a-f]{64})<!-- END SHA linux-x64 -->", body)
print(m.group(1) if m else "")' <<< "$rel")
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
systemctl enable qemu-guest-agent.service > /dev/null 2>&1 || true
