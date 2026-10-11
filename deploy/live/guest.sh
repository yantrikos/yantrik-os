# Shared by the live setup scripts, which run on node2 as root and reach the VMs only through the
# Proxmox guest agent (the live segment has no path in from the LAN). Source it, set VM, then:
#   guest 'SCRIPT' [< stdin]
# runs SCRIPT with sh in the VM as root, passes stdin through (the agent takes at most 1 MiB),
# prints what it said, and fails when it exits non-zero.
guest() {
    _out=$(qm guest exec "$VM" --timeout 120 --pass-stdin 1 -- sh -c "$1")
    # Printed with its control characters taken out: the live instance is assumed hostile, and an
    # escape sequence in what it says would be one written to root's terminal on node2.
    printf '%s' "$_out" | python3 -c 'import json,re,sys; d=json.load(sys.stdin); c=lambda s: re.sub(r"[\x00-\x08\x0b-\x1f\x7f-\x9f]", "", s); sys.stdout.write(c(d.get("out-data",""))); sys.stderr.write(c(d.get("err-data","")))'
    [ "$(printf '%s' "$_out" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("exitcode", 1))')" = 0 ]
}
