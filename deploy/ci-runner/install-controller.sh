#!/usr/bin/env bash
# Install the CI controller on the Proxmox node (#1234). Run as root, from this directory.
#
# Installs the script, its libraries, the host-side nft rules and the unit. It never enables,
# starts or restarts the service: a reboot of the node must not start CI by itself (review R2),
# and a restart would end a job in flight. The operator does that, when ready (README.md).

set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
token_dir=/root/.yantrik-ci
token=$token_dir/token
lib=/usr/local/lib/yantrik-ci

[ "$(id -u)" -eq 0 ] || { echo "install-controller: run as root" >&2; exit 1; }

install -m 755 -o root -g root "$here/yantrik-ci-controller.sh" /usr/local/sbin/yantrik-ci-controller
install -d -m 755 -o root -g root "$lib"
install -m 644 -o root -g root "$here/lib/github.sh" "$here/lib/host.sh" "$here/host/host-egress.nft" "$lib/"
install -m 644 -o root -g root "$here/yantrik-ci-controller.service" /etc/systemd/system/yantrik-ci-controller.service
install -d -m 700 -o root -g root "$token_dir" /var/log/yantrik-ci
install -d -m 755 -o root -g root /etc/yantrik-ci

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

echo "yantrik-ci-controller is installed. It is NOT enabled and NOT started."
if [ ! -s /etc/yantrik-ci/own-public-ips ]; then
  echo "Before the first start, list our own public addresses (the router's WAN address, any other),"
  echo "one per line, in /etc/yantrik-ci/own-public-ips, then rebuild the template."
fi
if [ ! -e "$token" ]; then
  echo "The token is not in place: put it in $token (mode 600, owner root)."
else
  mode=$(stat -c '%u %a' "$token")
  if [ "$mode" != "0 600" ] && [ "$mode" != "0 400" ]; then
    echo "install-controller: $token is '$mode'; fix with: chown root:root $token; chmod 600 $token" >&2
    exit 1
  fi
fi
if systemctl is-active --quiet yantrik-ci-controller; then
  echo "It is RUNNING the previous version; apply the new one when idle with: systemctl restart yantrik-ci-controller"
else
  echo "When ready: systemctl enable --now yantrik-ci-controller"
fi
