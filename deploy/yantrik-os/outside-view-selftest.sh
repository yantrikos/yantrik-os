#!/bin/bash
# What a command a Mind drives sees of the home (#1265): bash deploy/yantrik-os/outside-view-selftest.sh
#
# Run as the person on a test VM after a deploy, with the shell up and the mode set to bypass-all, and
# YANTRIK_AGENT_TOKEN set to a Mind's token. It plants canaries, runs ONE command through a Mind's
# `agent_run` that lists and reads each path and appends a line to ~/.bashrc, and checks:
#   - the four hidden canaries are reported missing, and the visible one is read;
#   - the host's ~/.bashrc is byte-for-byte what it was.
# Then a second call with `reach: ["~/.ssh"]` must raise a card even in bypass-all; that needs the
# always-asked table of #1313 and is checked here only once `reach` is accepted. Skipped, saying so,
# without a token or `yos`. The Terminal-tab half is the same command typed into a Mind's tab.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/selftest-lib.sh"

yos="${YOS:-$(command -v yos || echo /opt/yantrik/bin/yos)}"
if [ ! -x "$yos" ] || [ -z "${YANTRIK_AGENT_TOKEN:-}" ]; then
  echo "SKIP: needs yos and YANTRIK_AGENT_TOKEN (a Mind's token) on a VM with the shell running"
  exit 0
fi

canaries=(.ssh/canary-1265 .config/gh/canary-1265 .newthing/canary-1265 Pictures/Screenshots/canary-1265)
visible=Documents/ok-1265
cleanup() { for c in "${canaries[@]}" "$visible"; do rm -f "$HOME/$c"; done; rmdir "$HOME/.newthing" 2>/dev/null; }
trap cleanup EXIT
for c in "${canaries[@]}" "$visible"; do mkdir -p "$(dirname "$HOME/$c")"; echo canary-1265 > "$HOME/$c"; done
touch "$HOME/.bashrc"
before="$(sha256sum "$HOME/.bashrc")"

cmd='for f in '"${canaries[*]}"' '"$visible"'; do if cat "$HOME/$f" >/dev/null 2>&1; then echo "READ $f"; else echo "MISSING $f"; fi; done; echo "# canary-1265" >> "$HOME/.bashrc"; true'
out="$("$yos" act shell agent_run "command=$cmd" --agent-token "$YANTRIK_AGENT_TOKEN" 2>&1)"

for c in "${canaries[@]}"; do has "$c is missing in the view" "MISSING $c" "$out"; done
has "the visible folder's file is read" "READ $visible" "$out"
is "the host's ~/.bashrc is unchanged" "$before" "$(sha256sum "$HOME/.bashrc")"

[ "$fails" = 0 ] && echo "outside view: all ok" || { echo "outside view: $fails FAILED"; exit 1; }
