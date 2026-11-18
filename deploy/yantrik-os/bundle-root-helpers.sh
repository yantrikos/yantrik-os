#!/bin/bash
# bundle-root-helpers.sh — put the root helpers into a release bundle, from root-helpers.list (#1269).
#
#   bundle-root-helpers.sh ROOT     ROOT is the bundle's root: share/root-helpers/<name> for every
#                                   `ship` line, and share/root-helpers.list, a copy of the list.
#
# Called by build-release.sh, and by check-root-helpers.sh (CI) into a scratch folder, which holds
# the result against the list: a bundle that stops carrying a helper an image installs is removed
# from the machine's trust, and the cloud image never gets it.
set -eu
ROOT="${1:?usage: bundle-root-helpers.sh ROOT}"
HERE="$(cd "$(dirname "$0")" && pwd)"
LIST="$HERE/root-helpers.list"
[ -f "$LIST" ] || { echo "bundle-root-helpers: no $LIST" >&2; exit 1; }
mkdir -p "$ROOT/share/root-helpers"
while read -r kind name src _; do
  [ "$kind" = ship ] || continue
  [ -f "$HERE/$src" ] || { echo "bundle-root-helpers: root-helpers.list ships $name from $src, which is not there" >&2; exit 1; }
  install -m 0755 "$HERE/$src" "$ROOT/share/root-helpers/$name"
done < <(grep -v '^[[:space:]]*#' "$LIST")
install -m 0644 "$LIST" "$ROOT/share/root-helpers.list"
