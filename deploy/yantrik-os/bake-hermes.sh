#!/bin/sh
# Bake Hermes, the default mind, into the image being built: bake-hermes.sh ROOTFS
#
# Without it Hermes installs in the background after first run (4 min 40 s, and it needs the
# internet) while the built-in Companion answers, and the first answer comes from a different mind
# than the one the README describes (#1166). With it Hermes is on the image, pinned and checked,
# and the first run only wires it up.
#
# Where: /var/lib/yantrik-mind/harness/hermes, the home of the mind account `yantrik-mind`, which
# is where hermes.sh installs Hermes when it runs as that account (#411, #1076). The account is made
# here with the flags yantrik-update's reconcile_minds uses, which then finds it and leaves it. What is baked is Hermes alone (harnesses/lib/install/hermes-bake.sh): the person's
# plugin, config and memory are made at first run, as they were.
#
# BAKED is the desktop's cue that Hermes is here (crates/yantrik-ui/src/default_mind_baked.rs): root's,
# outside the account's home, which the person cannot open.
#
# YANTRIK_ISO_WITHOUT_HERMES=1 builds an image without it on purpose. Any other failure, a
# checksum mismatch included, stops the build: an image that quietly lost it would bring the wait
# back.
set -eu
[ $# -eq 1 ] || { echo "usage: bake-hermes.sh ROOTFS" >&2; exit 2; }
rootfs=$1
fail() { echo "bake-hermes: $*" >&2; exit 1; }

home=/var/lib/yantrik-mind/harness/hermes
if [ "${YANTRIK_ISO_WITHOUT_HERMES:-0}" = 1 ]; then
    echo "bake-hermes: building WITHOUT Hermes (YANTRIK_ISO_WITHOUT_HERMES=1); it installs at first run" >&2
    exit 0
fi
[ -f "$rootfs/opt/yantrik/share/harnesses/lib/install/hermes-bake.sh" ] \
    || fail "the harnesses are not staged in $rootfs yet"

chroot "$rootfs" getent group yantrik-minds >/dev/null || chroot "$rootfs" groupadd --system yantrik-minds
chroot "$rootfs" getent passwd yantrik-mind >/dev/null || chroot "$rootfs" useradd --system \
    --gid yantrik-minds --home-dir /var/lib/yantrik-mind --no-create-home \
    --shell /usr/sbin/nologin --comment "Yantrik minds" yantrik-mind
# 0750 and 0700 as yantrik-update and harness-isolation.sh make them; the tree is the account's.
chroot "$rootfs" install -d -o yantrik-mind -g yantrik-minds -m 0750 /var/lib/yantrik-mind
chroot "$rootfs" install -d -o yantrik-mind -g yantrik-minds -m 0700 /var/lib/yantrik-mind/harness "$home"

# The installer needs the network: the build host's resolv.conf is in the chroot while it builds.
# umask 077 as the harness installs run, and a clean environment, as harness-isolation.sh runs them.
# In a pid namespace of its own, with a fresh /proc over the chroot's: what the fetched code runs
# as the image's `yantrik-mind` uid then sees and signals none of the build host's processes, whose
# uid that number may belong to there (review of #1169, L1). Its SysV IPC is its own too, and so
# is its /dev (bake-hermes-sandbox.sh): a tmpfs with the few nodes an installer opens, not the
# host's /dev with the /dev/shm every host process of that uid shares (L-E). The network stays: it
# is the fetch. An egress-only network namespace is left for a follow-up, as it needs pasta or
# slirp4netns on every build host.
unshare --fork --pid --ipc --mount --mount-proc="$rootfs/proc" \
    sh "$(dirname "$0")/bake-hermes-sandbox.sh" "$rootfs" \
    chroot "$rootfs" runuser -u yantrik-mind -- env -i HOME="$home" USER=yantrik-mind LOGNAME=yantrik-mind \
    LANG=C.UTF-8 PATH=/usr/local/bin:/usr/bin:/bin TERM=dumb \
    sh -c 'umask 077; exec sh /opt/yantrik/share/harnesses/lib/install/hermes-bake.sh' \
    || fail "Hermes did not bake; the image build stops (YANTRIK_ISO_WITHOUT_HERMES=1 builds without it on purpose)"

[ "$(stat -c %U "$rootfs$home/.local/bin/hermes")" = yantrik-mind ] || fail "Hermes is not the mind account's"
mkdir -p "$rootfs/opt/yantrik/hermes"
commit=$(. "$(dirname "$0")/../../harnesses/lib/install/hermes-pin.sh" && printf '%s' "$HERMES_COMMIT")
printf '%s\n' "$commit" > "$rootfs/opt/yantrik/hermes/BAKED"
chmod 755 "$rootfs/opt/yantrik/hermes"; chmod 644 "$rootfs/opt/yantrik/hermes/BAKED"
echo "bake-hermes: Hermes $commit baked at $home ($(du -sh "$rootfs$home" | cut -f1))"
