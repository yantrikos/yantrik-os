#!/bin/sh
# Put back every file an install wrote: revert.sh HARNESS
#
# Each install keeps the person's own file once, before its first change (common.sh, backup_once),
# and lists it in ~/.local/state/yantrik/harnesses/<harness>/files. This puts each one back as it
# was, removes a file the install created where there was none, and forgets the install, so the
# row reads "not installed" again. The harness's YantrikDB file is kept: it holds what the mind
# remembered, and deleting a memory is the person's decision, not Revert's.
#
# A harness's running service keeps what it read until it restarts; the last line says which.
set -eu
. "$(dirname "$0")/common.sh"

[ $# -eq 1 ] || fail "usage: revert.sh HARNESS"
state=$(harness_state "$1")
[ -f "$state/files" ] || fail "nothing was written for $1, so there is nothing to put back"
while IFS= read -r file; do
    [ -n "$file" ] || continue
    if [ -e "$file.before-yantrik" ]; then
        mv -f "$file.before-yantrik" "$file" || fail "could not put back $file"
        say "put back $file"
    elif [ -e "$file.before-yantrik.absent" ]; then
        rm -f "$file" "$file.before-yantrik.absent" || fail "could not remove $file"
        say "removed $file, which the install had created"
    fi
done < "$state/files"
rm -rf "$state"
case "$1" in
    hermes) say "restart Hermes's gateway to finish: systemctl --user restart hermes-gateway" ;;
    mind) ;;
    *) say "restart it to finish: systemctl --user restart yantrik-$1.service" ;;
esac
