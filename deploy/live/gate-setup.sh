#!/bin/sh
# live-gate: the only way in or out of vmbr9, the Yantrik live instance's isolated segment.
# See design/live-instance-2026-09-29.md, "The network". Run as root on the gate VM (Debian 13).
# Idempotent: running it again rewrites the same config.
#
#   lan  — the NIC on vmbr0 (the home LAN): the gate's own address, and the way to the internet.
#   live — the NIC on vmbr9: 10.99.0.1/24, where the instance lives.
#
# What it enforces, outside the instance, where root inside the instance cannot reach:
#   - The instance may reach public addresses (NAT), and nothing private: not the LAN, the
#     router, link-local, CGNAT, multicast. IPv6 is not forwarded at all.
#   - It may reach the gate only for DNS (public resolvers, private answers refused), the model
#     proxy, which forwards exactly the chat calls, with the instance's key, to AIG, and UDP 9000,
#     where the video forwarder takes its desktop to the relay.
#   - The LAN may reach the gate only on SSH, for administration.
set -eu

AIG_HOST=aig.mycluster.cyou
LIVE_ADDR=10.99.0.1
LIVE_NET=10.99.0.0/24
KEY_FILE=/etc/live-gate/instance-key

# The two NICs, found by address rather than by name.
LIVE_IF=$(ip -o -4 addr show | awk -v a="$LIVE_ADDR" '$4 ~ "^"a"/" {print $2; exit}')
LAN_IF=$(ip -o -4 route show default | awk '{print $5; exit}')
[ -n "$LIVE_IF" ] && [ -n "$LAN_IF" ] && [ "$LIVE_IF" != "$LAN_IF" ] || { echo "cannot tell the NICs apart (live=$LIVE_IF lan=$LAN_IF)" >&2; exit 1; }
echo "lan=$LAN_IF live=$LIVE_IF"

export DEBIAN_FRONTEND=noninteractive
apt-get -qq update
apt-get -qq install -y nftables dnsmasq nginx qemu-guest-agent >/dev/null
systemctl enable --now qemu-guest-agent >/dev/null 2>&1 || true

# ── Forwarding, IPv4 only ──
cat > /etc/sysctl.d/60-live-gate.conf <<EOF
net.ipv4.ip_forward = 1
net.ipv6.conf.all.forwarding = 0
net.ipv6.conf.$LIVE_IF.disable_ipv6 = 1
net.ipv4.conf.all.rp_filter = 1
net.ipv4.conf.all.send_redirects = 0
net.ipv4.conf.all.accept_redirects = 0
EOF
sysctl -q --system

# ── The firewall ──
cat > /etc/nftables.conf <<EOF
#!/usr/sbin/nft -f
# live-gate (deploy/live/gate-setup.sh). Edits are put back when the script runs again.
flush ruleset

define LAN_IF  = "$LAN_IF"
define LIVE_IF = "$LIVE_IF"
define PRIVATE = { 0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8, 169.254.0.0/16,
                   172.16.0.0/12, 192.0.0.0/24, 192.168.0.0/16, 198.18.0.0/15, 224.0.0.0/4, 240.0.0.0/4 }

table inet gate {
    chain input {
        type filter hook input priority filter; policy drop;
        iif lo accept
        ct state established,related accept
        ct state invalid drop
        # From the instance: DNS and DHCP from this gate, the model proxy, and its desktop video for
        # the forwarder (deploy/live/gate-forward, which holds the relay's secrets). Nothing else.
        iifname \$LIVE_IF udp dport { 53, 67 } accept
        iifname \$LIVE_IF tcp dport { 53, 8443 } accept
        iifname \$LIVE_IF udp dport 9000 accept
        iifname \$LIVE_IF icmp type echo-request limit rate 5/second accept
        # From the LAN: administration only.
        iifname \$LAN_IF tcp dport 22 accept
        iifname \$LAN_IF icmp type echo-request accept
    }
    chain forward {
        type filter hook forward priority filter; policy drop;
        ct state established,related accept
        ct state invalid drop
        meta nfproto ipv6 drop
        # The instance reaches the public internet, and never a private address.
        iifname \$LIVE_IF oifname \$LAN_IF ip daddr \$PRIVATE counter drop
        iifname \$LIVE_IF oifname \$LAN_IF ip saddr $LIVE_NET accept
    }
    chain output {
        type filter hook output priority filter; policy accept;
    }
}

table ip nat {
    chain postrouting {
        type nat hook postrouting priority srcnat;
        oifname \$LAN_IF ip saddr $LIVE_NET masquerade
    }
}
EOF
nft -c -f /etc/nftables.conf
systemctl enable --now nftables >/dev/null
nft -f /etc/nftables.conf

# ── DNS and DHCP for the instance: public resolvers only, private answers refused ──
cat > /etc/dnsmasq.d/live.conf <<EOF
# live-gate (deploy/live/gate-setup.sh)
interface=$LIVE_IF
bind-interfaces
except-interface=lo
no-resolv
server=1.1.1.1
server=9.9.9.9
# A public name that answers with a private address is refused, so no name can lead into the LAN.
stop-dns-rebind
domain-needed
bogus-priv
dhcp-range=10.99.0.10,10.99.0.20,255.255.255.0,12h
dhcp-option=option:router,$LIVE_ADDR
dhcp-option=option:dns-server,$LIVE_ADDR
EOF
# dnsmasq must not also listen on the LAN for everybody.
sed -i 's/^#\?\s*IGNORE_RESOLVCONF=.*/IGNORE_RESOLVCONF=yes/' /etc/default/dnsmasq 2>/dev/null || true
systemctl restart dnsmasq

# ── The model proxy: the one way to AIG ──
mkdir -p /etc/live-gate
chmod 700 /etc/live-gate
if [ ! -s "$KEY_FILE" ]; then
  head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$KEY_FILE"
fi
chmod 600 "$KEY_FILE"
KEY=$(cat "$KEY_FILE")
rm -f /etc/nginx/sites-enabled/default
cat > /etc/nginx/conf.d/live-model.conf <<EOF
# live-gate (deploy/live/gate-setup.sh): exactly the chat calls, with the instance's key, to AIG.
limit_req_zone \$binary_remote_addr zone=live_model:1m rate=2r/s;
server {
    listen $LIVE_ADDR:8443;
    server_name _;
    client_max_body_size 512k;
    proxy_read_timeout 300s;
    proxy_send_timeout 60s;
    access_log /var/log/nginx/live-model.log;

    location ~ ^/(api/chat|api/generate|v1/chat/completions|v1/models)\$ {
        if (\$http_authorization != "Bearer $KEY") { return 401; }
        limit_req zone=live_model burst=10 nodelay;
        proxy_set_header Authorization "";
        proxy_set_header Cookie "";
        proxy_set_header Host $AIG_HOST;
        proxy_ssl_server_name on;
        proxy_ssl_name $AIG_HOST;
        proxy_http_version 1.1;
        proxy_buffering off;
        proxy_pass https://$AIG_HOST;
    }
    location / { return 404; }
}
EOF
nginx -t 2>&1 | tail -1
systemctl enable --now nginx >/dev/null
systemctl reload nginx
echo "ok — the instance's model key is in $KEY_FILE on this gate"
