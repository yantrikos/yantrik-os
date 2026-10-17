#!/bin/bash
# Sourced, not run: the one check of what a Yantrik Mind bundle can do, shared by
# build-debian-iso.sh (the image) and `yantrik-update install-mind` (a reinstall), so the two
# can never disagree about which Mind this OS may be paired with.
#
# A capability is claimed twice, and both must be there:
#   - the bundle's BUILD says `<name>=1`. yantrik-mind's package.sh writes that line only when
#     the binary's bytes carry the capability, so a new script packaging an old binary cannot
#     claim it;
#   - the binary itself carries one compiled-in marker, exactly `ym-capabilities: <names>;`
#     (yantrik-mind fc5aa7b and later), and <name> is one of its words. This is what refuses a
#     hand-edited BUILD. The binary is never run to ask: install-mind does not start a second
#     Mind beside the service.
# The marker ends at `;` because a Rust string has no terminator in the binary: without it, any
# lowercase bytes after the string would read as more words.

# What this OS relies on, one per line: <name> <what goes wrong with a Mind that lacks it>.
# A new capability is one more line here.
MIND_CAPABILITIES_REQUIRED='
agent_token every act from it would be refused by the MIND rule (#423)
cmd_answer it misreads the first line yos-mcp writes on a command answer, so a failed command reads as done (#566)
'
MIND_CAPABILITIES_FIX='build the bundle with yantrik-mind fc5aa7b or later (its deploy/yantrik-os/package.sh)'

# The words of the binary's first marker, space-separated; fails when it has none.
# `sed -n 1p`, not `head -1`: under pipefail, head closing the pipe early kills grep with SIGPIPE
# and fails a binary that does carry the marker.
mind_capabilities_marked() { # <binary>
  grep -aoE 'ym-capabilities: [a-z_ ]*;' "$1" 2>/dev/null | sed -n 1p \
    | sed -e 's/^ym-capabilities: //' -e 's/;$//'
}

# Silent and 0 when the unpacked bundle at <dir> has every required capability. Otherwise
# prints one line naming the first missing one, why it matters and the fix, and returns 1.
mind_capabilities_check() { # <unpacked bundle dir>
  local dir="$1" marked name why
  [ -f "$dir/BUILD" ] || { printf 'it carries no BUILD manifest; %s\n' "$MIND_CAPABILITIES_FIX"; return 1; }
  [ -f "$dir/bin/yantrik-mind" ] || { printf 'it has no bin/yantrik-mind; %s\n' "$MIND_CAPABILITIES_FIX"; return 1; }
  # The manifest first, so an older Mind is told the capability it lacks rather than that its
  # binary predates the marker.
  while read -r name why; do
    [ -n "$name" ] || continue
    grep -qx "$name=1" "$dir/BUILD" && continue
    printf 'it does not declare %s=1: %s; %s\n' "$name" "$why" "$MIND_CAPABILITIES_FIX"
    return 1
  done <<<"$MIND_CAPABILITIES_REQUIRED"
  marked=" $(mind_capabilities_marked "$dir/bin/yantrik-mind") "
  while read -r name why; do
    [ -n "$name" ] || continue
    case "$marked" in
      *" $name "*) ;;
      *) printf 'its BUILD declares %s=1 but the Mind binary'"'"'s ym-capabilities marker does not name %s: %s; %s\n' \
           "$name" "$name" "$why" "$MIND_CAPABILITIES_FIX"
         return 1 ;;
    esac
  done <<<"$MIND_CAPABILITIES_REQUIRED"
  return 0
}
