#!/bin/bash
# check-root-helpers.sh — an image build must not install a root helper the bundle does not carry (#1269).
#
# The first `yantrik-update apply` on an ISO install removed yantrik-broadcom-wifi and yantrik-mac-fans:
# the image installed them and the bundle did not carry them. root-helpers.list is the one list of
# what lives at the top level of /usr/lib/yantrik. This fails when
#   - a script that builds an image names /usr/lib/yantrik/<name> and the list does not (as ship, dir or retire);
#   - a `ship` line's source is missing, a name is listed twice, or a name is both shipped and retired;
#   - build-release.sh does not read the list, or the bundle's hash list does not name it.
# Only a literal name is seen: /usr/lib/yantrik/$x is how the image copies the bundle's own
# share/root-helpers, which is the list by construction.
#
#   check-root-helpers.sh [DIR]     DIR holds the scripts and the list (default: this directory)
set -u
DIR="${1:-$(cd "$(dirname "$0")" && pwd)}"
LIST="$DIR/root-helpers.list"
fails=0
bad() { echo "check-root-helpers: $*" >&2; fails=$((fails + 1)); }

[ -f "$LIST" ] || { echo "check-root-helpers: no $LIST" >&2; exit 1; }
names="$(grep -v '^[[:space:]]*#' "$LIST" | awk 'NF { print $2 }')"
dups="$(printf '%s\n' "$names" | sort | uniq -d)"
[ -z "$dups" ] || bad "named twice in root-helpers.list: $(echo "$dups" | tr "\n" " ")"
while read -r kind name src _; do
  case "$kind" in
    ship) [ -f "$DIR/$src" ] || bad "root-helpers.list ships $name from $src, which is not there" ;;
    retire|dir) ;;
    *) bad "root-helpers.list: unknown kind '$kind' ($name)" ;;
  esac
done < <(grep -v '^[[:space:]]*#' "$LIST" | awk 'NF')

# Everything that builds an image: the two image builds, both installers and what they stage.
scripts=("$DIR"/build-debian-iso.sh "$DIR"/build-image.sh "$DIR"/build-iso.sh "$DIR"/build-vbox-image.sh \
         "$DIR"/yantrik-install.sh "$DIR"/stage-*.sh "$DIR"/cloud-init/user-data.yaml)
for f in "${scripts[@]}"; do
  [ -f "$f" ] || continue
  while read -r name; do
    printf '%s\n' "$names" | grep -qxF -- "$name" \
      || bad "$(basename "$f") installs /usr/lib/yantrik/$name, which neither the bundle ships nor root-helpers.list retires — the first update would remove it (add it to root-helpers.list)"
  done < <(grep -v '^[[:space:]]*#' "$f" | grep -oE 'usr/lib/yantrik/[A-Za-z0-9][A-Za-z0-9._-]*' | sed 's|.*/||' | sort -u)
done

grep -q 'root-helpers\.list' "$DIR/build-release.sh" 2>/dev/null || bad "build-release.sh does not read root-helpers.list"
grep -qE 'sha256sum .*root-helpers\.list' "$DIR/build-release.sh" 2>/dev/null || bad "build-release.sh does not hash-list share/root-helpers.list"

[ "$fails" = 0 ] && echo "check-root-helpers: every helper an image installs is in root-helpers.list"
[ "$fails" = 0 ]
