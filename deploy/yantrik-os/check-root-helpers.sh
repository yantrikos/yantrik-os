#!/bin/bash
# check-root-helpers.sh — the release bundle carries every root helper an image installs (#1269).
#
# The first `yantrik-update apply` on an ISO install removed yantrik-broadcom-wifi and yantrik-mac-fans:
# the image installed them and the bundle did not carry them. root-helpers.list is the one list of
# what lives at the top level of /usr/lib/yantrik. This fails when
#   - the bundle step (bundle-root-helpers.sh, run here into a scratch folder) does not make
#     share/root-helpers hold exactly the list's `ship` names, each 0755 and identical to its source,
#     with share/root-helpers.list identical to the list;
#   - build-release.sh does not call that step (outside a comment), or does not hash-list the list;
#   - the list is malformed: an unknown kind, a name twice, a `ship` source that is missing;
#   - a script that builds an image puts something at the top level of /usr/lib/yantrik that the
#     list does not name: a literal name, a destination that ends at the directory (cp x dir/,
#     install -t dir), a variable standing for it, or /usr/lib/yantrik/$name. The one legitimate
#     case, the image copying the bundle's own share/root-helpers, carries the marker comment
#     `# root-helpers:bundle-copy` on its line.
#
#   check-root-helpers.sh [DIR]     DIR holds the scripts and the list (default: this directory)
set -u
DIR="${1:-$(cd "$(dirname "$0")" && pwd)}"
LIST="$DIR/root-helpers.list"
MARK='root-helpers:bundle-copy'
fails=0
bad() { echo "check-root-helpers: $*" >&2; fails=$((fails + 1)); }

[ -f "$LIST" ] || { echo "check-root-helpers: no $LIST" >&2; exit 1; }
entries="$(grep -v '^[[:space:]]*#' "$LIST" | awk 'NF')"
names="$(printf '%s\n' "$entries" | awk '{ print $2 }')"
dups="$(printf '%s\n' "$names" | sort | uniq -d)"
[ -z "$dups" ] || bad "named twice in root-helpers.list: $(echo "$dups" | tr '\n' ' ')"
while read -r kind name src _; do
  case "$kind" in
    ship) [ -f "$DIR/$src" ] || bad "root-helpers.list ships $name from $src, which is not there" ;;
    retire|dir) ;;
    *) bad "root-helpers.list: unknown kind '$kind' ($name)" ;;
  esac
done <<< "$entries"

# The bundle step, run: what it makes is held to the list.
REL="$DIR/build-release.sh"
rel_code="$(grep -v '^[[:space:]]*#' "$REL" 2>/dev/null | sed 's/[[:space:]]#.*//')"
if ! printf '%s\n' "$rel_code" | grep -q 'bundle-root-helpers\.sh'; then
  bad "build-release.sh does not call bundle-root-helpers.sh (a call in a comment does not count)"
fi
printf '%s\n' "$rel_code" | grep -qE 'sha256sum .*root-helpers\.list' || bad "build-release.sh does not hash-list share/root-helpers.list"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
if [ -f "$DIR/bundle-root-helpers.sh" ] && bash "$DIR/bundle-root-helpers.sh" "$scratch" 2>"$scratch/err"; then
  want="$(printf '%s\n' "$entries" | awk '$1 == "ship" { print $2 }' | sort)"
  have="$(find "$scratch/share/root-helpers" -mindepth 1 -maxdepth 1 -printf "%f\n" 2>/dev/null | sort)"
  [ "$want" = "$have" ] || bad "the bundle's share/root-helpers holds [$(echo "$have" | tr '\n' ' ')], the list ships [$(echo "$want" | tr '\n' ' ')]"
  while read -r kind name src _; do
    [ "$kind" = ship ] || continue
    f="$scratch/share/root-helpers/$name"
    [ -f "$f" ] || continue
    [ "$(stat -c %a "$f")" = 755 ] || bad "bundle: $name is mode $(stat -c %a "$f"), not 755"
    cmp -s "$f" "$DIR/$src" || bad "bundle: $name is not identical to $src"
  done <<< "$entries"
  cmp -s "$scratch/share/root-helpers.list" "$LIST" || bad "bundle: share/root-helpers.list is not the list"
else
  bad "bundle-root-helpers.sh failed or is missing: $(cat "$scratch/err" 2>/dev/null)"
fi

# Everything that builds an image: the two image builds, both installers and what they stage.
scripts=("$DIR"/build-debian-iso.sh "$DIR"/build-image.sh "$DIR"/build-iso.sh "$DIR"/build-vbox-image.sh \
         "$DIR"/yantrik-install.sh "$DIR"/stage-*.sh "$DIR"/cloud-init/user-data.yaml)
for f in "${scripts[@]}"; do
  [ -f "$f" ] || continue
  code="$(grep -v '^[[:space:]]*#' "$f" | grep -vF "$MARK")"
  while read -r name; do
    [ -n "$name" ] || continue
    printf '%s\n' "$names" | grep -qxF -- "$name" \
      || bad "$(basename "$f") installs /usr/lib/yantrik/$name, which neither the bundle ships nor root-helpers.list retires — the first update would remove it (add it to root-helpers.list)"
  done < <(printf '%s\n' "$code" | grep -oE 'usr/lib/yantrik/[A-Za-z0-9][A-Za-z0-9._-]*' | sed 's|.*/||' | sort -u)
  # Forms that put a file at the top level without a literal name.
  hits="$(printf '%s\n' "$code" | grep -nE 'usr/lib/yantrik/\$|usr/lib/yantrik/["'\'']?([[:space:]]|$)|-t[ =]*["'\'']?[^ ]*usr/lib/yantrik["'\'']?|^[[:space:]]*[A-Za-z_]+=["'\'']?[^ ]*usr/lib/yantrik["'\'']?[[:space:]]*$' || true)"
  # `install -d` and mkdir make the directory, not a helper in it.
  hits="$(printf '%s\n' "$hits" | grep -vE 'install -d |install -[a-z]*d[a-z]* |mkdir ' | grep . || true)"
  [ -z "$hits" ] || bad "$(basename "$f") puts files in /usr/lib/yantrik without naming them: $(printf '%s' "$hits" | head -1) — name them in root-helpers.list, or mark the bundle copy with '# $MARK'"
done

[ "$fails" = 0 ] && echo "check-root-helpers: the bundle carries every helper an image installs (root-helpers.list)"
[ "$fails" = 0 ]
