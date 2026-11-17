#!/bin/bash
# Install the CI guest files inside the build VM (#1252), as root: the egress filter, the launcher, the
# start command the controller runs through the guest agent, the job-started hook and the launch unit,
# all root-owned. The unit is never enabled: only yantrik-ci-start starts it. Usage:
# install.sh <own-public-ips file>
set -euo pipefail
g=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
own=${1:?usage: install.sh <own-public-ips file>}
install -d -m 755 -o root -g root /etc/yantrik-ci /usr/local/libexec
install -m 644 -o root -g root "$g/egress.nft" /etc/yantrik-ci/egress.nft
install -m 644 -o root -g root "$own" /etc/yantrik-ci/own-public-ips
install -m 755 -o root -g root "$g/yantrik-ci-egress" "$g/yantrik-ci-launch" "$g/yantrik-ci-start" /usr/local/sbin/
install -m 755 -o root -g root "$g/yantrik-ci-job-started.sh" /usr/local/libexec/
install -m 644 -o root -g root "$g/yantrik-ci-launch.service" /etc/systemd/system/
systemctl daemon-reload
if [ "$(systemctl is-enabled yantrik-ci-launch.service 2> /dev/null)" = enabled ]; then
  echo "install.sh: the launch unit is enabled" >&2
  exit 1
fi
