#!/bin/sh
# Install the Yantrik Live publisher on the live instance, from node2 as root, through the guest
# agent. It needs no secret: the relay's live on the gate (../gate-forward). A publish.env left
# by the earlier design, which kept them here, is shredded.
#   sh setup-publisher.sh        (from this directory; VM defaults to 561)
set -eu
cd "$(dirname "$0")"
VM=${VM:-561}
. ../guest.sh

guest 'DEBIAN_FRONTEND=noninteractive apt-get -qq install -y wf-recorder >/dev/null && command -v wf-recorder >/dev/null'
guest 'set -e; install -d -m 755 /usr/local/lib/yantrik-live; cat > /usr/local/lib/yantrik-live/publish.new; chmod 755 /usr/local/lib/yantrik-live/publish.new; mv /usr/local/lib/yantrik-live/publish.new /usr/local/lib/yantrik-live/publish' < publish
guest 'set -e; cat > /etc/systemd/system/yantrik-live-publish.service; chmod 644 /etc/systemd/system/yantrik-live-publish.service; systemctl daemon-reload; systemctl enable yantrik-live-publish.service 2>&1; systemctl restart yantrik-live-publish.service; sleep 5; systemctl is-active yantrik-live-publish.service' < yantrik-live-publish.service
guest 'd=/home/yantrik/.config/yantrik-live; if [ -e $d/publish.env ]; then shred -u $d/publish.env && echo "shredded the old publish.env"; fi; rmdir $d 2>/dev/null; true'
echo "publisher running on VM $VM"
