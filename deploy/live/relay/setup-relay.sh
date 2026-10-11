#!/bin/sh
# Set up the Yantrik Live relay on the public VPS (yantrikos.com), as root, from this directory:
#   sudo sh setup-relay.sh
# Idempotent. It installs MediaMTX (pinned, checksum-checked) as its own account, creates the
# publisher's secrets once (root, 600, never printed), and adds /live/ to the yantrikos.com site,
# checking nginx and restoring the site file if the check fails. See design/live-instance-2026-09-29.md.
set -eu
cd "$(dirname "$0")"

VERSION=v1.21.1
SHA256=653abc672a3e693f8d3b2717752492fdcfb8072291ec108d03d3dd857411b0ee
SITE=/etc/nginx/sites-available/yantrikos.com
SNIPPET=/etc/nginx/snippets/yantrikos-live.conf
PAGE=/var/www/yantrikos-live
INCLUDE="    include $SNIPPET;"

[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
# Restarting MediaMTX drops the live stream for a few seconds: only when something it runs changed.
changed=0

# MediaMTX, the exact release this was written against.
if ! /opt/mediamtx/mediamtx --version 2>/dev/null | grep -qx "$VERSION"; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    curl -fsSL -o "$tmp/m.tgz" "https://github.com/bluenviron/mediamtx/releases/download/$VERSION/mediamtx_${VERSION}_linux_amd64.tar.gz"
    echo "$SHA256  $tmp/m.tgz" | sha256sum -c --quiet
    tar -xzf "$tmp/m.tgz" -C "$tmp" mediamtx
    install -d -m 755 /opt/mediamtx
    install -m 755 "$tmp/mediamtx" /opt/mediamtx/mediamtx.new
    mv /opt/mediamtx/mediamtx.new /opt/mediamtx/mediamtx
    changed=1
fi

id mediamtx >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin mediamtx

install -d -m 755 /etc/mediamtx
cmp -s mediamtx.yml /etc/mediamtx/mediamtx.yml || { install -m 644 mediamtx.yml /etc/mediamtx/mediamtx.yml; changed=1; }

# The publisher's secrets: made once, then kept, so the gate's copy (deploy/live/gate-forward)
# keeps working. To rotate: shred this file, run this again, then run setup-forward.sh.
if [ ! -s /etc/mediamtx/secrets.env ]; then
    ( umask 077
      pass=$(head -c 48 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)
      phrase=$(head -c 64 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 40)
      printf 'MTX_AUTHINTERNALUSERS_0_PASS=%s\nMTX_PATHS_LIVE_SRTPUBLISHPASSPHRASE=%s\n' "$pass" "$phrase" > /etc/mediamtx/secrets.env.new
      mv /etc/mediamtx/secrets.env.new /etc/mediamtx/secrets.env )
    changed=1
fi
chown root:root /etc/mediamtx/secrets.env
chmod 600 /etc/mediamtx/secrets.env

cmp -s mediamtx.service /etc/systemd/system/mediamtx.service || { install -m 644 mediamtx.service /etc/systemd/system/mediamtx.service; changed=1; }
systemctl daemon-reload
systemctl enable mediamtx.service >/dev/null 2>&1
if [ "$changed" = 1 ]; then systemctl restart mediamtx.service; else systemctl start mediamtx.service; fi

# The page.
install -d -m 755 "$PAGE"
install -m 644 www/* "$PAGE"/

# The site: the snippet, and one include line in its TLS server block (the one Certbot marked).
# nginx is shared by every site on this host: a change it rejects is put back at once, snippet
# and site file both, so the next restart (a reboot, a Certbot renewal) never meets it.
[ -e "$SNIPPET" ] && cp -p "$SNIPPET" "$SNIPPET.previous"
install -m 644 nginx-live.conf "$SNIPPET"
added=0
if ! grep -qF "$INCLUDE" "$SITE"; then
    cp -p "$SITE" "$SITE.before-live"
    awk -v inc="$INCLUDE" '!done && /listen 443 ssl; # managed by Certbot/ { print inc; done=1 } { print }' "$SITE.before-live" > "$SITE.new"
    grep -qF "$INCLUDE" "$SITE.new" || { rm -f "$SITE.new"; echo "no TLS server block found in $SITE" >&2; exit 1; }
    mv "$SITE.new" "$SITE"
    added=1
fi
if ! nginx -t 2>/dev/null; then
    if [ -e "$SNIPPET.previous" ]; then cp -p "$SNIPPET.previous" "$SNIPPET"; else rm -f "$SNIPPET"; fi
    [ "$added" = 1 ] && cp -p "$SITE.before-live" "$SITE"
    nginx -t
    echo "nginx rejected the change; the snippet and $SITE are as they were" >&2
    exit 1
fi
rm -f "$SNIPPET.previous"
systemctl reload nginx

sleep 2
systemctl is-active --quiet mediamtx.service || { journalctl -u mediamtx -n 20 --no-pager >&2; exit 1; }
# Exactly two sockets, whatever a new release turns on by default: SRT in, and HLS on loopback.
listening=$(ss -Hlntup | awk '/"mediamtx"/ { print $1, $5 }' | sort | tr '\n' ' ')
if [ "$listening" != "tcp 127.0.0.1:8888 udp *:8890 " ]; then
    systemctl stop mediamtx.service
    echo "MediaMTX listened on more than it should ($listening); stopped it" >&2
    exit 1
fi
echo "relay up: MediaMTX $VERSION, SRT :8890/udp in, https://yantrikos.com/live/ out"
