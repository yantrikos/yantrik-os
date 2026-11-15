#!/usr/bin/env bash
# Install the CI controller on the Proxmox node (#1234). Run as root, from this directory.
#
# Installs the script and the unit and enables the unit. It never starts or restarts it.
# The token is Pranab's, put there by hand.

set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
token_dir=/root/.yantrik-ci
token=$token_dir/token

[ "$(id -u)" -eq 0 ] || { echo "install-controller: run as root" >&2; exit 1; }

install -m 755 -o root -g root "$here/yantrik-ci-controller.sh" /usr/local/sbin/yantrik-ci-controller
install -m 644 -o root -g root "$here/yantrik-ci-controller.service" /etc/systemd/system/yantrik-ci-controller.service
install -d -m 700 -o root -g root "$token_dir" /var/log/yantrik-ci

cat > /etc/logrotate.d/yantrik-ci <<'ROTATE'
/var/log/yantrik-ci/controller.log {
    weekly
    rotate 8
    compress
    missingok
    notifempty
    copytruncate
}
ROTATE

systemctl daemon-reload
systemctl enable yantrik-ci-controller.service

# Installing never starts or restarts the service: a restart would end a job in flight. Whoever
# installs decides when (README.md).
if [ ! -e "$token" ]; then
  echo "yantrik-ci-controller is installed and enabled, but NOT started: waiting for the token."
  echo "Put the GitHub token in $token (mode 600, owner root), then:"
else
  mode=$(stat -c '%u %a' "$token")
  if [ "$mode" != "0 600" ] && [ "$mode" != "0 400" ]; then
    echo "install-controller: $token is '$mode'; fix with: chown root:root $token; chmod 600 $token" >&2
    exit 1
  fi
  if systemctl is-active --quiet yantrik-ci-controller; then
    echo "yantrik-ci-controller is installed and RUNNING the previous version; apply it when idle with:"
    echo "  systemctl restart yantrik-ci-controller"
    exit 0
  fi
  echo "yantrik-ci-controller is installed and enabled, NOT started. The token is in place; to start:"
fi
echo "  systemctl start yantrik-ci-controller"
