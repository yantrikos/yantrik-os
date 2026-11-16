#!/bin/bash
# check-root-helpers.sh — the release bundle carries every root helper an image installs (#1269).
#
# The first `yantrik-update apply` on an ISO install removed yantrik-broadcom-wifi and yantrik-mac-fans:
# the image installed them and the bundle did not carry them. root-helpers.list is the one list of
# what lives at the top level of /usr/lib/yantrik. This fails when
#   - the bundle step (bundle-root-helpers.sh, run here into a scratch folder) does not make
#     share/root-helpers hold exactly the list's `ship` names, each 0755 and identical to its source,
#     with share/root-helpers.list identical to the list;
#   - build-release.sh does not call that step, exactly, outside a comment; has any other line that
#     names share/root-helpers (a later rm, a call into another root); does not hash-list both the list
#     and root-helpers/* on its sha256sum line; or lost the guard that fails the build when a bundled
#     helper has no line in the root-files list (marked `# root-helpers:guard`);
#   - the list is malformed: an unknown kind, a name twice, a `ship` source that is missing;
#   - a script that builds an image puts something at the top level of /usr/lib/yantrik that the
#     list does not ship (a file it installs must be `ship`, not `retire` or `dir`): a literal name, a destination that ends at the directory (cp x dir/,
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

# The bundle step, run: what it makes is held to the list. First, how build-release.sh uses it.
REL="$DIR/build-release.sh"
GUARD='root-helpers:guard'
rel_raw="$(grep -v '^[[:space:]]*#' "$REL" 2>/dev/null)"
grep -qF "$GUARD" <<< "$rel_raw" || bad "build-release.sh lost the guard that fails the build when a bundled helper is not in the root-files list ('# $GUARD')"
rel_code="$(grep -vF "$GUARD" <<< "$rel_raw" | sed 's/[[:space:]]#.*//')"
# shellcheck disable=SC2016  # a pattern, not an expansion
grep -qE '^bash "\$SCRIPT_DIR/bundle-root-helpers\.sh" "\$ROOT" \|\| fail ' <<< "$rel_code" \
  || bad "build-release.sh does not call: bash \"\$SCRIPT_DIR/bundle-root-helpers.sh\" \"\$ROOT\" || fail … (a call in a comment, a different root, or one that does not run does not count)"
sumline="$(grep 'sha256sum' <<< "$rel_code" | grep -F 'root-helpers.list' || true)"
grep -qF 'root-helpers/*' <<< "$sumline" \
  || bad "build-release.sh's sha256sum line does not name both root-helpers.list and root-helpers/*: every helper would be refused and removed by every machine"
others="$(grep -F 'root-helpers/' <<< "$rel_code" | grep -v 'sha256sum' || true)"
[ -z "$others" ] || bad "build-release.sh names share/root-helpers outside the bundle step: $(head -1 <<< "$others")"
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
  while IFS= read -r line; do
    # A line that makes a directory or names one in a variable is not installing a file in it.
    dirline=0; grep -qE 'install -d |install -[a-z]*d[a-z]* |mkdir |^[[:space:]]*[A-Za-z_]+=' <<< "$line" && dirline=1
    while read -r m; do
      [ -n "$m" ] || continue
      name="${m#usr/lib/yantrik/}"; slash=0
      case "$name" in */) slash=1; name="${name%/}" ;; esac
      kind="$(awk -v n="$name" '$2 == n { print $1; exit }' <<< "$entries")"
      if [ -z "$kind" ]; then
        bad "$(basename "$f") installs /usr/lib/yantrik/$name, which root-helpers.list does not name — the first update would not keep it (add it as ship)"
      elif [ "$kind" != ship ] && [ "$slash" = 0 ] && [ "$dirline" = 0 ]; then
        bad "$(basename "$f") installs /usr/lib/yantrik/$name as a file, and root-helpers.list says '$kind': a file an image installs must be 'ship', or the update removes it (or the bundle drops it)"
      fi
    done < <(grep -oE 'usr/lib/yantrik/[A-Za-z0-9][A-Za-z0-9._-]*/?' <<< "$line")
  done < <(grep 'usr/lib/yantrik/' <<< "$code")
  # Forms that put a file at the top level without a literal name.
  hits="$(grep -nE 'usr/lib/yantrik/\$|usr/lib/yantrik/["'\'']?([[:space:]]|$)|-t[ =]*["'\'']?[^ ]*usr/lib/yantrik["'\'']?|usr/lib/yantrik["'\'']?[[:space:]]*\\?$' <<< "$code" || true)"
  # `install -d` and mkdir make the directory, not a helper in it; so does naming it in a variable.
  hits="$(grep -vE 'install -d |install -[a-z]*d[a-z]* |mkdir |^[0-9]+:[[:space:]]*[A-Za-z_]+=' <<< "$hits" | grep . || true)"
  [ -z "$hits" ] || bad "$(basename "$f") puts files in /usr/lib/yantrik without naming them: $(head -1 <<< "$hits") — name them in root-helpers.list, or mark the bundle copy with '# $MARK'"
done

[ "$fails" = 0 ] && echo "check-root-helpers: the bundle carries every helper an image installs (root-helpers.list)"
[ "$fails" = 0 ]
