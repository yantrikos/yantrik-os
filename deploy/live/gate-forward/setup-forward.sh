#!/bin/sh
# Install the Yantrik Live forwarder on the gate VM, from node2 as root, with the relay's
# secrets.env on stdin; they go straight into the gate (root, 600) and are never written on node2
# or printed:
#   ssh ubuntu@yantrikos.com sudo cat /etc/mediamtx/secrets.env | ssh root@node2 sh setup-forward.sh
# Run from this directory. VM defaults to 560. The gate's firewall rule for UDP 9000 is
# gate-setup.sh's.
set -eu
cd "$(dirname "$0")"
VM=${VM:-560}
. ../guest.sh

secrets=$(cat)
printf '%s\n' "$secrets" | grep -qE '^MTX_AUTHINTERNALUSERS_0_PASS=[A-Za-z0-9]+$' \
    && printf '%s\n' "$secrets" | grep -qE '^MTX_PATHS_LIVE_SRTPUBLISHPASSPHRASE=[A-Za-z0-9]{10,79}$' \
    || { echo "stdin is not the relay's secrets.env" >&2; exit 1; }

guest 'DEBIAN_FRONTEND=noninteractive apt-get -qq install -y srt-tools >/dev/null && command -v srt-live-transmit >/dev/null'
printf '%s\n' "$secrets" | guest 'set -e; umask 077; install -d -m 700 /etc/yantrik-live; cat > /etc/yantrik-live/relay.env.new; mv /etc/yantrik-live/relay.env.new /etc/yantrik-live/relay.env; stat -c "%U %a %n" /etc/yantrik-live/relay.env'
guest 'set -e; install -d -m 755 /usr/local/lib/yantrik-live; cat > /usr/local/lib/yantrik-live/forward.new; chmod 755 /usr/local/lib/yantrik-live/forward.new; mv /usr/local/lib/yantrik-live/forward.new /usr/local/lib/yantrik-live/forward' < forward
guest 'set -e; cat > /etc/systemd/system/yantrik-live-forward.service; chmod 644 /etc/systemd/system/yantrik-live-forward.service; systemctl daemon-reload; systemctl enable yantrik-live-forward.service 2>&1; systemctl restart yantrik-live-forward.service; sleep 3; systemctl is-active yantrik-live-forward.service' < yantrik-live-forward.service
echo "forwarder running on VM $VM"
