# Run-time trust for the CI controller (#1252 L3). Sourced first by the controller, which checks this
# file and its directory itself (it cannot use what it has not yet loaded), then by everything else.
# shellcheck shell=bash

# trusted_path PATH: root-owned and not writable by group or others. Root sources these files, splices
# /etc/yantrik-ci/* into an nft ruleset and includes host-egress.nft, so anything that is not root's
# alone is refused. Needs log().
trusted_path() {
  local m
  m=$(stat -c '%u %a' "$1" 2> /dev/null) || { log "cannot stat $1"; return 1; }
  if [ "${m%% *}" != 0 ] || [ $((8#${m#* } & 8#022)) -ne 0 ]; then
    log "REFUSING to use $1: owner/mode '$m' (need root, not group- or other-writable)"
    return 1
  fi
}
