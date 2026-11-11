#!/bin/sh
# Give the command its own /dev, then run it: bake-hermes-sandbox.sh ROOTFS COMMAND...
#
# Run by bake-hermes.sh inside `unshare --mount`, so what is mounted here is seen by the command
# and by nothing else. The chroot's /dev is the build host's, bind-mounted by build-debian-iso.sh,
# and Hermes's installer (fetched code, hash-pinned but still fetched) runs with it: /dev/shm there
# is shared with every process of that uid on the host (review of #1169, L-E). It gets a tmpfs
# with the few nodes an installer opens, and a /dev/shm of its own.
set -eu
[ $# -ge 2 ] || { echo "usage: bake-hermes-sandbox.sh ROOTFS COMMAND..." >&2; exit 2; }
rootfs=$1; shift
dev=$rootfs/dev

mount -t tmpfs -o mode=0755,nosuid,size=4m tmpfs "$dev"
for node in null zero full random urandom tty; do
    : > "$dev/$node"
    mount --bind "/dev/$node" "$dev/$node"
done
mkdir -m 1777 "$dev/shm"
mount -t tmpfs -o mode=1777,nosuid,nodev,size=256m tmpfs "$dev/shm"
ln -s /proc/self/fd "$dev/fd"
ln -s /proc/self/fd/0 "$dev/stdin"
ln -s /proc/self/fd/1 "$dev/stdout"
ln -s /proc/self/fd/2 "$dev/stderr"
exec "$@"
