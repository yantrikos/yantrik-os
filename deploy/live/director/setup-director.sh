#!/bin/sh
# Install the Director on the live instance (VM 561), from node2 as root, from this directory.
# VM defaults to 561. Re-run to update the script or the missions; finished missions are kept
# in the account's ~/director/done.json and are not started again.
set -eu
cd "$(dirname "$0")"
VM=${VM:-561}
# ENABLE=0 installs without starting the timer, e.g. while the Mind is still in Ask mode, where
# its first command would only raise a card that nobody is there to answer.
ENABLE=${ENABLE:-1}
. ../guest.sh

guest 'set -e; install -d -m 755 /opt/yantrik-live/director; install -d -o yantrik -g yantrik -m 700 /home/yantrik/director' < /dev/null
for f in director.py missions.json check-page; do
  guest "set -e; cat > /opt/yantrik-live/director/$f.new; chmod 644 /opt/yantrik-live/director/$f.new; [ $f = check-page ] && chmod 755 /opt/yantrik-live/director/$f.new; mv /opt/yantrik-live/director/$f.new /opt/yantrik-live/director/$f" < "$f"
done
for u in yantrik-live-director.service yantrik-live-director.timer; do
  guest "set -e; cat > /etc/systemd/system/$u; chmod 644 /etc/systemd/system/$u" < "$u"
done
guest 'set -e; python3 -c "import json; json.load(open(\"/opt/yantrik-live/director/missions.json\"))"; systemctl daemon-reload' < /dev/null
if [ "$ENABLE" = 1 ]; then
  guest 'systemctl enable --now yantrik-live-director.timer >/dev/null 2>&1; systemctl is-active yantrik-live-director.timer' < /dev/null
else
  echo "timer left off (ENABLE=0)"
fi
echo "director installed on VM $VM; start a mission now with: systemctl start --no-block yantrik-live-director"
