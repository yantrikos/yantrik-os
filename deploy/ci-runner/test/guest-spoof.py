#!/usr/bin/env python3
"""Run inside a CI clone, as root, by check-net.sh (via the guest agent, python3 - <args> on stdin).
Sends hand-made frames from the clone's own network card so the node's table (host/yci.nft) can be
shown to drop what it must and pass what it must. The node counts what arrives on yci0 with tcpdump;
nothing here is received or decided in the guest.

  guest-spoof.py <own-ip> <bridge-mac> <peer-mac|-> <peer-ip|->

The UDP destination port names each frame (40001...), so the node's capture says which got through:
  40001 control: own MAC, own address, to the node's MAC      -> must arrive
  40002 spoofed source address (a made-up one in the subnet)   -> must be dropped
  40003 spoofed source MAC (own address)                       -> must be dropped
  40004 spoofed source address and MAC                         -> must be dropped
  40005 own MAC and address, but to the peer clone's MAC       -> must be dropped (no clone to clone)
  40006 own MAC and address, a DHCP discover from 0.0.0.0      -> must be dropped (port 67)
  an ARP who-has for the node from the clone's MAC and address -> must arrive (the control)
  an ARP reply claiming the node's address at the clone's MAC  -> must be dropped
  an IPv6 frame, a VLAN-tagged frame                            -> must be dropped
  an experimental-ethertype (0x88b5) broadcast, own MAC         -> reaches the node's bridge, no other clone
  the same with a forged source MAC                             -> must be dropped (locked port)
"""
import socket
import struct
import sys
import time

own_ip, bridge_mac, peer_mac, peer_ip = sys.argv[1:5]


def mac(s):
    return bytes.fromhex(s.replace(":", ""))


def iface():
    for line in open("/proc/net/route").read().splitlines()[1:]:
        f = line.split()
        if f[1] == "00000000":
            return f[0]
    raise SystemExit("no default route")


IF = iface()
own_mac = mac(open("/sys/class/net/%s/address" % IF).read().strip())
s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW)
s.bind((IF, 0))


def csum(b):
    if len(b) % 2:
        b += b"\0"
    t = sum(struct.unpack("!%dH" % (len(b) // 2), b))
    while t >> 16:
        t = (t & 0xFFFF) + (t >> 16)
    return ~t & 0xFFFF


def udp_frame(src_mac, dst_mac, src_ip, dst_ip, sport, dport):
    payload = b"yantrik-ci-spoof-%d" % dport
    udp = struct.pack("!HHHH", sport, dport, 8 + len(payload), 0) + payload
    ip = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(udp), dport, 0, 64, 17, 0,
                     socket.inet_aton(src_ip), socket.inet_aton(dst_ip))
    ip = ip[:10] + struct.pack("!H", csum(ip)) + ip[12:]
    return dst_mac + src_mac + b"\x08\x00" + ip + udp


def arp_frame(op, src_mac, src_ip, dst_mac, dst_ip, eth_dst):
    return eth_dst + src_mac + b"\x08\x06" + struct.pack("!HHBBH6s4s6s4s", 1, 0x0800, 6, 4, op, src_mac,
                                                         socket.inet_aton(src_ip), dst_mac, socket.inet_aton(dst_ip))


fake_ip = ".".join(own_ip.split(".")[:3] + ["77"])
fake_mac = mac("02:aa:bb:cc:dd:ee")
bmac = mac(bridge_mac)
bcast = b"\xff" * 6
gw = ".".join(own_ip.split(".")[:3] + ["1"])
frames = [
    udp_frame(own_mac, bmac, own_ip, "9.9.9.9", 40001, 40001),
    udp_frame(own_mac, bmac, fake_ip, "9.9.9.9", 40002, 40002),
    udp_frame(fake_mac, bmac, own_ip, "9.9.9.9", 40003, 40003),
    udp_frame(fake_mac, bmac, fake_ip, "9.9.9.9", 40004, 40004),
    udp_frame(own_mac, bcast, "0.0.0.0", "255.255.255.255", 68, 67),
    arp_frame(1, own_mac, own_ip, b"\0" * 6, gw, bcast),
    arp_frame(2, own_mac, gw, bcast, gw, bcast),
    b"\x33\x33\x00\x00\x00\x02" + own_mac + b"\x86\xdd" + b"\x60\0\0\0\x00\x08\x3a\xff" + b"\xfe\x80" + b"\0" * 14 + b"\xff\x02" + b"\0" * 13 + b"\x02" + b"\x85\0\0\0\0\0\0\0",
    bmac + own_mac + b"\x81\x00\x00\x01" + udp_frame(own_mac, bmac, own_ip, "9.9.9.9", 40007, 40007)[12:],
]
frames.append(bcast + own_mac + b"\x88\xb5" + b"yantrik-ci-own-mac".ljust(46, b"."))
frames.append(bcast + fake_mac + b"\x88\xb5" + b"yantrik-ci-fake-mac".ljust(46, b"."))
if peer_mac != "-":
    frames.append(udp_frame(own_mac, mac(peer_mac), own_ip, peer_ip, 40005, 40005))
for f in frames:
    s.send(f)
    time.sleep(0.2)
print("sent %d frames from %s (%s)" % (len(frames), IF, own_mac.hex(":")))
