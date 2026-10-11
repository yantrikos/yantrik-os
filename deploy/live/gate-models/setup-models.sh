#!/bin/sh
# Give the live instance's Mind three cloud providers, Ollama Cloud, NanoGPT and NVIDIA NIM, through the gate
# (VM 560), beside the AIG route it already has. Run on node2 as root, from this directory, with
# the keys on stdin as KEY=value lines; they go straight into the gate (root, 600) and are
# never written on node2 or printed:
#   grep -E '^(OLLAMA_CLOUD_KEY|NANOGPT_KEY|NIM_KEY)=' keys.env | ssh root@node2 'cd …/gate-models && sh setup-models.sh'
# VM defaults to 560. Needs gate-setup.sh's model proxy in place, and must be run again after
# gate-setup.sh, which rewrites live-model.conf without this directory's include. Run it again
# to change a key; a run nginx refuses puts back everything it touched.
#
# Then the instance's Mind is pointed at these routes (point-mind.sh; see README.md).
set -eu
cd "$(dirname "$0")"
VM=${VM:-560}
. ../guest.sh

# Carriage returns dropped: a key file or pipe from Windows ends its lines in \r\n.
keys=$(tr -d '\r')
value() { printf '%s\n' "$keys" | sed -n "s/^$1=\([A-Za-z0-9._-]\{20,200\}\)\$/\1/p" | head -n 1; }
ollama=$(value OLLAMA_CLOUD_KEY)
nano=$(value NANOGPT_KEY)
# A NIM key is `nvapi-` and base64url, about 70 characters in all; anything else is refused, not trimmed.
nim=$(printf '%s\n' "$keys" | sed -n 's/^NIM_KEY=\(nvapi-[A-Za-z0-9_-]\{40,90\}\)$/\1/p' | head -n 1)
# A key line that is present but fails its format is an error, never a silent "keep the old key":
# the person would believe the key was rotated. Only the provider's name is printed, never the value.
present() { printf '%s\n' "$keys" | grep -q "^$1="; }
for pair in "OLLAMA_CLOUD_KEY:$ollama" "NANOGPT_KEY:$nano" "NIM_KEY:$nim"; do
  if present "${pair%%:*}" && [ -z "${pair#*:}" ]; then
    echo "${pair%%:*} is on stdin but is not in the format accepted; nothing changed" >&2; exit 1
  fi
done
# The keys are written into nginx's configuration, so anything outside that character set (a
# quote, a semicolon, a newline) is refused here rather than allowed to become a directive.
[ -n "$ollama" ] || [ -n "$nano" ] || [ -n "$nim" ] \
    || { echo "stdin needs at least one of OLLAMA_CLOUD_KEY= or NANOGPT_KEY= (each 20-200 of [A-Za-z0-9._-]) and/or NIM_KEY= (nvapi- then 40-90 of [A-Za-z0-9_-]) lines" >&2; exit 1; }
# A provider left out keeps the key the gate already has (pick-keys.py sends only what it finds),
# and one the gate has never had gets a placeholder key: the route loads, the provider answers 401,
# the gate turns that into its 502, and no provider's key is a precondition for rotating another's.
keep() {
  if guest "test -s /etc/live-gate/$1.auth" < /dev/null; then
    echo "$1: keeping the key the gate has"
  else
    echo "$1: no key on stdin and none on the gate; the route answers 502 until one is given"
    stub="$stub $1"
  fi
}
stub=""
[ -n "$ollama" ] || keep ollama-cloud
[ -n "$nano" ] || keep nanogpt
[ -n "$nim" ] || keep nim

# Everything this run may change, kept first, so a refused configuration goes back to exactly
# what was there, the previous working routes and keys included.
guest 'set -e
test -s /etc/live-gate/instance-key
test -f /etc/nginx/conf.d/live-model.conf
# A gate whose configuration nginx already refuses is not this script to fix, and its refusal
# would otherwise be reported as the new routes being refused. (2026-10-01: the LAN router
# answered NXDOMAIN for aig.mycluster.cyou, so the AIG route itself would not load.)
if ! nginx -t 2>/dev/null; then nginx -t 2>&1 | tail -2; echo "the gate is refused as it stands; nothing changed" >&2; exit 1; fi
DEBIAN_FRONTEND=noninteractive apt-get -qq install -y libnginx-mod-http-js >/dev/null
umask 077
rm -rf /etc/live-gate/before-cloud
install -d -m 700 /etc/live-gate/before-cloud /etc/nginx/live-routes /etc/nginx/njs
for f in /etc/nginx/conf.d/live-model.conf /etc/nginx/conf.d/live-cloud.conf /etc/nginx/live-routes/cloud.conf /etc/nginx/live-routes/models-list.conf \
         /etc/nginx/njs/live_models.js /etc/live-gate/ollama-cloud.auth /etc/live-gate/nanogpt.auth /etc/live-gate/nim.auth; do
  if [ -e "$f" ]; then cp -p "$f" "/etc/live-gate/before-cloud/$(echo "$f" | tr / _)"; fi
done
echo kept' < /dev/null

# Any failure from here on (a guest call timing out, awk failing) puts the kept files back, not only
# nginx's refusal: otherwise a new key could be left beside an old route. Safe to run twice.
restore='for f in /etc/nginx/conf.d/live-model.conf /etc/nginx/conf.d/live-cloud.conf /etc/nginx/live-routes/cloud.conf /etc/nginx/live-routes/models-list.conf /etc/nginx/njs/live_models.js /etc/live-gate/ollama-cloud.auth /etc/live-gate/nanogpt.auth /etc/live-gate/nim.auth; do
  kept="/etc/live-gate/before-cloud/$(echo "$f" | tr / _)"
  if [ -e "$kept" ]; then cp -p "$kept" "$f"; else rm -f "$f"; fi
done
nginx -t 2>/dev/null && systemctl reload nginx || true'
trap 'rc=$?; trap - EXIT; if [ $rc -ne 0 ]; then guest "$restore" < /dev/null || true; echo "run failed; the gate was put back as it was" >&2; fi; exit $rc' EXIT

# One root-only file per provider, holding the one header nginx adds for it. printf is the
# shell's own, so a key is never on a command line.
auth() { printf 'proxy_set_header Authorization "Bearer %s";\n' "$1"; }
[ -z "$ollama" ] || auth "$ollama" | guest 'set -e; umask 077; cat > /etc/live-gate/ollama-cloud.auth.new; mv /etc/live-gate/ollama-cloud.auth.new /etc/live-gate/ollama-cloud.auth'
[ -z "$nano" ] || auth "$nano" | guest 'set -e; umask 077; cat > /etc/live-gate/nanogpt.auth.new; mv /etc/live-gate/nanogpt.auth.new /etc/live-gate/nanogpt.auth'
[ -z "$nim" ] || auth "$nim" | guest 'set -e; umask 077; cat > /etc/live-gate/nim.auth.new; mv /etc/live-gate/nim.auth.new /etc/live-gate/nim.auth'
for p in $stub; do
  auth unconfigured | guest "set -e; umask 077; cat > /etc/live-gate/$p.auth.new; mv /etc/live-gate/$p.auth.new /etc/live-gate/$p.auth"
done
guest 'set -e; umask 077; cat > /etc/nginx/njs/live_models.js.new; mv /etc/nginx/njs/live_models.js.new /etc/nginx/njs/live_models.js' < live_models.js

# The Mind reads GET <base>/models before it uses a provider; the gate answers it from a static file
# generated from live_models.js's allowlist (gen-models-list.js), so the two cannot drift.
node gen-models-list.js --check >&2 || { echo "models-list.conf is out of step with live_models.js; run: node gen-models-list.js" >&2; exit 1; }
# The routes, with the instance key filled in on the gate itself (by awk reading the key file,
# so it is on no command line), and the server block made to include them.
guest 'set -e; umask 077; cat > /etc/nginx/live-routes/models-list.conf.in' < models-list.conf
guest 'set -e
umask 077
case "$(cat /etc/live-gate/instance-key)" in *[!0-9a-f]*|"") echo "the instance key is not hex" >&2; exit 1;; esac
awk "BEGIN { getline k < \"/etc/live-gate/instance-key\" } { gsub(/@INSTANCE_KEY@/, k); print }" \
  > /etc/nginx/live-routes/cloud.conf.new
mv /etc/nginx/live-routes/cloud.conf.new /etc/nginx/live-routes/cloud.conf
awk "BEGIN { getline k < \"/etc/live-gate/instance-key\" } { gsub(/@INSTANCE_KEY@/, k); print }" \
  /etc/nginx/live-routes/models-list.conf.in > /etc/nginx/live-routes/models-list.conf.new
mv /etc/nginx/live-routes/models-list.conf.new /etc/nginx/live-routes/models-list.conf
rm -f /etc/nginx/live-routes/models-list.conf.in
# Keyed on the gate address, not the caller address: the instance could add addresses of its own on
# the segment, and each would get a limit of its own. There is one instance; there is one limit.
cat > /etc/nginx/conf.d/live-cloud.conf <<EOF
# live-gate (deploy/live/gate-models): rates, daily counts and the filter of the cloud routes.
limit_req_zone \$server_addr zone=live_ollama_cloud:1m rate=30r/m;
limit_req_zone \$server_addr zone=live_nanogpt:1m rate=10r/m;
limit_req_zone \$server_addr zone=live_nim:1m rate=10r/m;
js_shared_dict_zone zone=live_budget:64k type=number timeout=3d state=/var/lib/nginx/live_budget.json;
js_import live from /etc/nginx/njs/live_models.js;
EOF
# The zones file of the first version named the same zones; left beside this one, nginx refuses both.
rm -f /etc/nginx/conf.d/live-cloud-zones.conf
grep -q "include /etc/nginx/live-routes/" /etc/nginx/conf.d/live-model.conf \
  || sed -i "s|^    location / { return 404; }|    include /etc/nginx/live-routes/*.conf;\n    location / { return 404; }|" /etc/nginx/conf.d/live-model.conf
chmod 600 /etc/nginx/conf.d/live-model.conf
if ! grep -q "include /etc/nginx/live-routes/" /etc/nginx/conf.d/live-model.conf || ! nginx -t 2>/dev/null; then
  nginx -t 2>&1 | tail -3 || true
  for f in /etc/nginx/conf.d/live-model.conf /etc/nginx/conf.d/live-cloud.conf /etc/nginx/live-routes/cloud.conf /etc/nginx/live-routes/models-list.conf \
           /etc/nginx/njs/live_models.js /etc/live-gate/ollama-cloud.auth /etc/live-gate/nanogpt.auth /etc/live-gate/nim.auth; do
    kept="/etc/live-gate/before-cloud/$(echo "$f" | tr / _)"
    if [ -e "$kept" ]; then cp -p "$kept" "$f"; else rm -f "$f"; fi
  done
  nginx -t 2>/dev/null && systemctl reload nginx
  echo "nginx refused the cloud routes; everything is as it was" >&2
  exit 1
fi
systemctl reload nginx || { echo "nginx would not reload; the files are new but the running gate is as it was" >&2; exit 1; }
# The rotated-out keys leave with the backup once the new configuration is loaded.
rm -rf /etc/live-gate/before-cloud
echo "cloud routes live: /ollama-cloud/v1/chat/completions, /nanogpt/api/v1/chat/completions, /nim/v1/chat/completions (and their /models lists)"' < routes.conf
