#!/bin/bash
# The image's refusal of a Mind bundle that lacks a capability this OS relies on
# (mind-capabilities.sh), run the way build-debian-iso.sh runs it: its own lines, lifted out of
# it, under its `set -euo pipefail`, against fake unpacked bundles (a BUILD and a bin/yantrik-mind
# whose bytes carry the marker or do not). `yantrik-update selftest` holds install-mind to the
# same check.
#
#   bash deploy/yantrik-os/mind-capabilities-selftest.sh
# shellcheck disable=SC2016  # the patterns below are literal shell text, matched, not expanded
# shellcheck disable=SC2034,SC2317  # SCRIPT_DIR, MIND_UNPACK and fail() are used by the eval'd ISO lines
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
iso="$here/build-debian-iso.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0 passes=0

# The ISO's own check: from where it sources the shared file to its `|| fail` line.
snippet="$(sed -n '/^    \. "\$SCRIPT_DIR\/mind-capabilities.sh"$/,/|| fail "mind bundle /p' "$iso")"
[ "$(printf '%s\n' "$snippet" | wc -l)" -ge 3 ] || { echo "FAIL: the capability check is not in $iso where this test looks"; exit 1; }
grep -q 'mind_capabilities_check "$stage"' "$here/yantrik-update" \
  || { echo "FAIL: yantrik-update install-mind does not run mind_capabilities_check"; exit 1; }

# fake <name> <BUILD body> <binary bytes>: an unpacked bundle, as the ISO has it in $MIND_UNPACK.
fake() {
  mkdir -p "$work/$1/bin"
  printf '%s\n' "$2" > "$work/$1/BUILD"
  printf '%s' "$3" > "$work/$1/bin/yantrik-mind"
}

# iso_check <name>: the ISO's lines against that bundle, in a subshell like the build; its message.
iso_check() {
  ( set -euo pipefail
    SCRIPT_DIR="$here" MIND_UNPACK="$work/$1"
    fail() { printf 'REFUSED %s\n' "$1"; exit 1; }
    eval "$snippet"
    printf 'ACCEPTED\n' ) 2>&1 || true
}

expect() { # expect <name> <description> <expected substring>
  local out; out="$(iso_check "$1")"
  if [[ "$out" == *"$3"* ]]; then
    passes=$((passes + 1)); printf '  ok   %s\n' "$2"
  else
    fails=$((fails + 1)); printf '  FAIL %s\n       want: %s\n       got:  %s\n' "$2" "$3" "$out"
  fi
}

marker='ym-capabilities: agent_token cmd_answer;'

# fc5aa7b as built: BUILD commit=fc5aa7b agent_token=1 cmd_answer=1, the marker in the binary,
# with more lowercase bytes straight after its `;` (a Rust string has no terminator).
fake fc5aa7b $'name=yantrik-mind-fc5aa7b-linux-amd64\ncommit=fc5aa7b\nagent_token=1\ncmd_answer=1' \
  $'\x7fELF\x02\x01fc5aa7b\x00'"${marker}"$'rustc_and_more\x00'
expect fc5aa7b "fc5aa7b (agent_token=1, cmd_answer=1, marker): accepted" "ACCEPTED"

# 6a6006c: agent_token=1, no cmd_answer line, no marker.
fake 6a6006c $'commit=6a6006c\nagent_token=1' $'\x7fELF6a6006c\x00'
expect 6a6006c "6a6006c-style (no cmd_answer): refused, naming it" \
  "REFUSED mind bundle 6a6006c: it does not declare cmd_answer=1"
expect 6a6006c "... and the fix" "yantrik-mind fc5aa7b or later"

fake notoken $'commit=0000001\ncmd_answer=1' "x${marker}"
expect notoken "BUILD without agent_token=1: refused" "does not declare agent_token=1"

fake handedit $'commit=0000002\nagent_token=1\ncmd_answer=1' $'\x7fELF no marker here\x00'
expect handedit "BUILD with cmd_answer=1 but no marker in the binary (hand-edited BUILD): refused" \
  "marker does not name agent_token"

fake nocmd $'commit=0000003\nagent_token=1\ncmd_answer=1' 'ym-capabilities: agent_token;'
expect nocmd "marker present but without cmd_answer: refused" "marker does not name cmd_answer"

fake noterm $'commit=0000004\nagent_token=1\ncmd_answer=1' 'ym-capabilities: agent_token cmd_answer'
expect noterm "marker without its closing ;: refused" "marker does not name agent_token"

fake nobuild '' "$marker"; rm "$work/nobuild/BUILD"
expect nobuild "no BUILD at all: refused" "carries no BUILD manifest"

# A binary with the marker first and then many more matches, more than a pipe holds: the first
# is taken, and grep is not killed by SIGPIPE under pipefail (what `head` would do).
fake big $'commit=0000005\nagent_token=1\ncmd_answer=1' "$marker"
for _ in $(seq 1 20000); do printf 'ym-capabilities: agent_token;\n'; done >> "$work/big/bin/yantrik-mind"
expect big "marker first, then 20000 more matches: accepted on the first" "ACCEPTED"
# shellcheck source=deploy/yantrik-os/mind-capabilities.sh
big_marked="$( set -euo pipefail; . "$here/mind-capabilities.sh"
               m="$(mind_capabilities_marked "$work/big/bin/yantrik-mind")"; printf '%s [exit %s]' "$m" "$?" )" || true
if [ "$big_marked" = "agent_token cmd_answer [exit 0]" ]; then
  passes=$((passes + 1)); printf '  ok   %s
' "... the marker read succeeds under pipefail, with the first marker's words"
else
  fails=$((fails + 1)); printf '  FAIL %s
       got:  %s
' "the marker read under pipefail" "$big_marked"
fi

printf '%d passed, %d failed\n' "$passes" "$fails"
[ "$fails" = 0 ]
