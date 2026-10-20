#!/bin/bash
# Build Yantrik's labwc: the labwc Debian has now plus debian-patches/, versioned as Debian's
# with `yantrik<REVISION>` appended (0.8.3-1 → 0.8.3-1yantrik2). That sorts above the Debian build it was
# made from and below any later one, so a Debian security fix replaces it on a machine (the
# updater never holds it or puts it back), and the next build patches that fix in turn.
# Run as root in a Debian 13 (trixie) system with deb-src enabled (the labwc-build container on
# node2, or a debian:trixie CI job). Prints the path of the .deb it built; fails, saying so, when
# the patch no longer applies to Debian's labwc.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="${1:-/tmp/labwc-yantrik}"
export DEBIAN_FRONTEND=noninteractive LC_ALL=C.UTF-8
apt-get update -qq
apt-get build-dep -y -qq labwc >/dev/null
# The binary version apt would install (trixie-security's, when it has one), and the source
# behind it. A binNMU (+b1) has the same source, so it is the binary version that ours extends.
BINARY="$(apt-cache policy labwc | sed -n 's/^ *Candidate: //p')"
[ -n "$BINARY" ] && [ "$BINARY" != "(none)" ] || { echo "no labwc in this system's apt sources" >&2; exit 1; }
# Raised by one whenever debian-patches/ changes, so a machine with the last build installs the
# new one (the updater installs a version it does not have). It still sorts below any later
# Debian build: 0.8.3-1yantrik2 < 0.8.3-1+deb13u1.
REVISION=2
OURS="${BINARY}yantrik${REVISION}"
rm -rf "$WORK" && mkdir -p "$WORK" && cd "$WORK"
apt-get source -qq labwc >/dev/null 2>&1
cd labwc-*/
SOURCE="$(dpkg-parsechangelog -S Version)"
case "$BINARY" in
  "$SOURCE"|"$SOURCE"+b[0-9]*) ;;
  *) echo "labwc's binary $BINARY is not built from the source apt fetched ($SOURCE)" >&2; exit 1 ;;
esac
# Debian ships labwc with no patches of its own, so the directory may not exist yet.
mkdir -p debian/patches
for p in "$HERE"/debian-patches/*.patch; do
  cp "$p" debian/patches/
  basename "$p" >> debian/patches/series
done
# Apply our patches in the unpacked tree (dpkg-source applied Debian's own on unpack). --batch:
# a patch that does not fit is a failed build, never a question.
while read -r p; do
  case "$p" in
    yantrik-*) patch -p1 --forward --batch --quiet < "debian/patches/$p" \
      || { echo "$p no longer applies to Debian's labwc $SOURCE: the patch needs porting" >&2; exit 1; } ;;
  esac
done < debian/patches/series
# The entry takes Debian's own last date, not dch's "now": dpkg takes SOURCE_DATE_EPOCH from the
# changelog, so two builds of the same patch on the same Debian labwc and toolchain are the same
# bytes. Anyone can rebuild and compare; what a machine trusts is the bundle's root-files list,
# like every other binary it ships.
STAMP="$(dpkg-parsechangelog -S Date)"
{
  printf 'labwc (%s) trixie; urgency=medium\n\n' "$OURS"
  printf '  * Add an external window switcher renderer (zlabwc_window_switcher_renderer_v1)\n'
  printf '    for Yantrik OS Alt+Tab (yantrikos/yantrik-os#872).\n\n'
  printf ' -- Yantrik OS <dev@yantrikos.com>  %s\n\n' "$STAMP"
  cat debian/changelog
} > debian/changelog.new
mv debian/changelog.new debian/changelog
SOURCE_DATE_EPOCH="$(date -d "$STAMP" +%s)"
export SOURCE_DATE_EPOCH
DEB_BUILD_OPTIONS="nocheck parallel=$(nproc)" dpkg-buildpackage -us -uc -b >"$WORK/build.log" 2>&1 \
  || { tail -30 "$WORK/build.log"; exit 1; }
ls "$WORK/labwc_${OURS}_amd64.deb"
