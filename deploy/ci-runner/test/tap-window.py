#!/usr/bin/env python3
"""Run on the node, as root, by check-net.sh: shows that the layer-2 table (host/yci-l2.nft) is on a clone's
tap port from the moment the port exists, before anything can send a frame through it (#1303).

  tap-window.py <tap-name> <bridge> <gateway-ip>

It plays qemu: it creates the tap (a throwaway device, gone when this script exits), prints the netdev hooks
the kernel has on it while it is still DOWN, then puts it on the bridge, brings it up and writes frames into
it at once, as a guest's first frames would be: an ARP who-has for the gateway and a frame of an unused
ethertype, both from a MAC that was never pinned. The port is not pinned, so the table must drop both; the
caller reads the drop counter and a capture on the bridge around this run.
"""
import fcntl
import socket
import struct
import subprocess
import sys
import time

name, bridge, gw = sys.argv[1:4]
TUNSETIFF = 0x400454CA
IFF_TAP, IFF_NO_PI = 0x0002, 0x1000
fd = open("/dev/net/tun", "r+b", buffering=0)
fcntl.ioctl(fd, TUNSETIFF, struct.pack("16sH", name.encode(), IFF_TAP | IFF_NO_PI))
hooks = subprocess.run(["nft", "list", "hooks", "netdev", "device", name], capture_output=True, text=True).stdout
print("hooks on %s while it is still down:" % name)
print("\n".join("  " + line.strip() for line in hooks.splitlines() if "yantrik" in line) or "  NONE")
subprocess.run(["ip", "link", "set", name, "master", bridge], check=True)
subprocess.run(["ip", "link", "set", name, "up"], check=True)
mac = bytes.fromhex("021122334455")
bcast = b"\xff" * 6
arp = bcast + mac + b"\x08\x06" + struct.pack("!HHBBH6s4s6s4s", 1, 0x0800, 6, 4, 1, mac, socket.inet_aton("10.95.30.47"),
                                                b"\0" * 6, socket.inet_aton(gw))
for frame in (arp, bcast + mac + b"\x88\xb5" + b"yantrik-ci-first-frame".ljust(46, b".")):
    fd.write(frame)
    time.sleep(0.2)
time.sleep(1)
print("wrote 2 frames into %s right after it came up" % name)
