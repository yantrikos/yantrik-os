# Run inside the live instance AS the yantrik-mind account (point-mind.sh sends it on stdin).
# Adds the cloud providers to the Mind's settings, naming the gate as their address and the
# instance key as their key. Not run as root: the file is the account's, and root following a
# link the account planted at it or its backup would be root writing wherever the account chose.
set -eu
umask 077
f=/var/lib/yantrik-mind/.config/yantrik-mind.env
key=$(sed -n 's/^YM_LOCAL_OLLAMA_KEY=\([0-9a-f]*\)$/\1/p' "$f" | head -n 1)
url=$(sed -n 's|^YM_LOCAL_OLLAMA_URL=\(http://[0-9.:]*\)/*$|\1|p' "$f" | head -n 1)
[ -n "$key" ] && [ -n "$url" ] || { echo "the Mind has no gate lane to build on" >&2; exit 1; }
# The backup is the settings from before the first run; a second run must not overwrite it.
[ -e "$f.before-cloud" ] || cp "$f" "$f.before-cloud"
# Any earlier copy of these lines goes, then they are written once, into a new file moved over
# the old one.
sed '/^# The cloud providers, through the gate/d; /^YM_LOCAL_ROLE=/d; /^YM_PRIMARY_BRAIN=/d;
     /^YM_PROVIDER_BASE_URL_OLLAMA_CLOUD=/d; /^YM_PROVIDER_BASE_URL_NANOGPT=/d;
     /^YM_PROVIDER_BASE_URL_NIM=/d; /^OLLAMA_CLOUD_KEY=/d; /^NANOGPT_KEY=/d; /^NIM_KEY=/d; /^NVIDIA_API_KEY=/d' "$f" > "$f.new"
# The names below were checked in the Mind's own code (mind-inference): provider id nim (alias
# nvidia), key env NVIDIA_API_KEY, base-URL override YM_PROVIDER_BASE_URL_NIM, default base
# https://integrate.api.nvidia.com/v1. NVIDIA_API_KEY is the name the Mind reads and cannot be
# renamed here; the value is the instance key, good only on the gate.
cat >> "$f.new" <<EOF
# The cloud providers, through the gate (deploy/live/gate-models/point-mind.sh).
YM_LOCAL_ROLE=fallback
YM_PRIMARY_BRAIN=ollama-cloud:kimi-k3
YM_PROVIDER_BASE_URL_OLLAMA_CLOUD=$url/ollama-cloud/v1
YM_PROVIDER_BASE_URL_NANOGPT=$url/nanogpt/api/v1
YM_PROVIDER_BASE_URL_NIM=$url/nim/v1
OLLAMA_CLOUD_KEY=$key
NANOGPT_KEY=$key
NVIDIA_API_KEY=$key
EOF
mv "$f.new" "$f"
echo "the Mind's settings name the gate for both cloud providers"
