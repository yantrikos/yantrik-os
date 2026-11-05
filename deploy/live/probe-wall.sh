#!/bin/sh
# Prove live-gate's wall from inside vmbr9. Run on node2 as root: it makes a throwaway container
# on vmbr9 only, tries every way out, and destroys it. KEY is the instance's model key, passed in
# the environment (never printed). See design/live-instance-2026-09-29.md.
set -u
CT=599
TPL=local:vztmpl/debian-13-standard_13.1-2_amd64.tar.zst
ok() { echo "ok   $*"; }; bad() { echo "FAIL $*"; }
pct status $CT >/dev/null 2>&1 && pct destroy $CT --force >/dev/null 2>&1
pct create $CT $TPL --hostname wall-probe --memory 256 --cores 1 --unprivileged 1 \
  --net0 name=eth0,bridge=vmbr9,ip=dhcp --rootfs local-lvm:2 >/dev/null || { bad "could not create the probe"; exit 1; }
pct start $CT; sleep 8
x() { pct exec $CT -- sh -c "$1"; }
ip=$(x "ip -4 -br addr show eth0" | awk '{print $3}')
case "$ip" in 10.99.0.*) ok "DHCP from the gate: $ip" ;; *) bad "no address from the gate ($ip)" ;; esac
x "command -v curl >/dev/null || (apt-get -qq update && apt-get -qq install -y curl >/dev/null 2>&1)"
code=$(x "curl -s -o /dev/null -w '%{http_code}' --max-time 15 https://example.com/")
[ "$code" = 200 ] && ok "reaches the internet ($code)" || bad "no internet ($code)"
for t in 192.168.4.1:80 192.168.4.152:8006 192.168.4.152:22 192.168.4.203:443 192.168.4.14:22 10.0.0.1:80 169.254.169.254:80; do
  if x "timeout 4 bash -c 'echo > /dev/tcp/${t%:*}/${t#*:}'" 2>/dev/null; then bad "reached $t"; else ok "cannot reach $t"; fi
done
a=$(x "getent hosts aig.mycluster.cyou" 2>/dev/null)
[ -z "$a" ] && ok "LAN names do not resolve (aig.mycluster.cyou)" || bad "aig resolved: $a"
code=$(x "curl -s -o /dev/null -w '%{http_code}' --max-time 10 http://10.99.0.1:8443/api/chat -d '{}'")
[ "$code" = 401 ] && ok "the model proxy refuses without the key ($code)" || bad "no key gave $code"
code=$(x "curl -s -o /dev/null -w '%{http_code}' --max-time 10 -H 'Authorization: Bearer $KEY' http://10.99.0.1:8443/api/tags")
[ "$code" = 404 ] && ok "only the chat paths are served (/api/tags: $code)" || bad "/api/tags gave $code"
body=$(x "curl -s --max-time 120 -H 'Authorization: Bearer $KEY' http://10.99.0.1:8443/api/chat -d '{\"model\":\"qwen3.8:27b\",\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":\"Reply with the one word: ready\"}]}'")
echo "$body" | grep -qi '"model"' && ok "the model answers through the proxy: $(echo "$body" | grep -o '"model":"[^"]*"' | head -1)" || bad "no model answer: $(echo "$body" | head -c 200)"
pct stop $CT >/dev/null 2>&1; pct destroy $CT --force >/dev/null 2>&1 && ok "probe destroyed"
