#!/bin/bash
# Seal the CI template VM (#1252, VM design). Run as root by make-template.sh, detached, as the last
# thing in the build VM, after the proofs: it removes every way in (sshd, sudo, the image's default
# account, the build key), keeps cloud-init from making a user or taking keys, makes the image ready
# to be cloned (no machine id, no host keys, no cloud-init state, no logs) and powers the VM off.
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive
sleep 2 # let the ssh session that started this one end

# Nothing listens in a job's VM and nothing logs in to it (review N9): no sshd, no sudo, and no account
# but root (locked) and runner (locked).
cat > /etc/cloud/cloud.cfg.d/99-yantrik-ci.cfg << 'EOF'
users: []
disable_root: true
ssh_pwauth: false
ssh_deletekeys: true
EOF
chmod 644 /etc/cloud/cloud.cfg.d/99-yantrik-ci.cfg
systemctl stop ssh.service ssh.socket > /dev/null 2>&1
apt-get purge -y openssh-server openssh-sftp-server > /dev/null 2>&1
SUDO_FORCE_REMOVE=yes apt-get purge -y sudo > /dev/null 2>&1
rm -f /etc/sudoers.d/*
userdel -rf debian > /dev/null 2>&1
apt-get autoremove -y --purge > /dev/null 2>&1

# A throwaway runner does not upgrade itself (gate 5, 18 Nov: unattended-upgrades was running in a job's VM
# beside a 15 GB compiler and was the process that triggered the kernel's OOM killer). The package is removed
# and every unit that would start apt on a timer is masked, so nothing runs apt in a clone.
apt-get purge -y unattended-upgrades > /dev/null 2>&1
systemctl mask apt-daily.timer apt-daily-upgrade.timer apt-daily.service apt-daily-upgrade.service   unattended-upgrades.service > /dev/null 2>&1
apt-get clean
rm -rf /var/lib/apt/lists/* /root/.ssh /etc/ssh/ssh_host_* /home/debian /tmp/* /var/tmp/* 2> /dev/null
passwd -l root > /dev/null 2>&1

cloud-init clean --logs --seed --configs all > /dev/null 2>&1
rm -rf /var/lib/cloud/instances /var/lib/cloud/instance /var/lib/cloud/data/* 2> /dev/null
: > /etc/machine-id
rm -f /var/lib/dbus/machine-id
journalctl --rotate > /dev/null 2>&1
journalctl --vacuum-time=1s > /dev/null 2>&1
rm -rf /var/log/journal/* /var/log/*.log /var/log/apt/* 2> /dev/null
sync
systemctl poweroff
