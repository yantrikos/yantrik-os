#!/usr/bin/env python3
"""Boot a Yantrik OS ISO and make it answer for itself. The gate between "built" and "published".

    boottest.py <iso> <outdir> [--expect-git REV]

An ISO that was assembled is not an ISO that boots, and the first public image was checked by
hand: start QEMU, log in on the serial console, ask. This is that, scripted, with the answers
turned into assertions — so a pipeline can refuse to publish an image that reaches a login
prompt with no desktop behind it, or one that has picked up a private address again.

It talks to two unix sockets on this host — the guest's serial port and QEMU's monitor — and to
nothing else. The guest has no network (`-net none`), which also proves the image needs none
to reach its first screen.

Exit 0: every check passed.  1: a check failed.  2: it never reached a login prompt.
Leaves in <outdir>: serial.log (everything the machine said), screen.ppm, report.json.
"""
import json
import os
import socket
import subprocess
import sys
import time

import argparse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import private_addresses  # noqa: E402

parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument("iso")
parser.add_argument("outdir")
parser.add_argument("--expect-git", metavar="REV",
                    help="fail unless the image's BUILD manifest names this revision")
cli = parser.parse_args()
iso, out, expect_git = cli.iso, cli.outdir, cli.expect_git
if not os.path.isfile(iso):
    sys.exit("boottest: no such ISO: " + iso)

os.makedirs(out, exist_ok=True)
ser, mon = os.path.join(out, "ser.sock"), os.path.join(out, "mon.sock")
for p in (ser, mon):
    if os.path.exists(p):
        os.unlink(p)

# KVM when the host has it (a CI runner usually does); software emulation when it does not
# (WSL does not). Emulated, the same boot takes about four times as long, so the waits below
# are sized for that.
kvm = os.access("/dev/kvm", os.R_OK | os.W_OK)
cmd = ["qemu-system-x86_64", "-m", "4096", "-smp", "4", "-cdrom", iso, "-boot", "d",
       "-display", "none", "-device", "virtio-vga",
       "-chardev", "socket,id=ser0,path=%s,server=on,wait=off" % ser, "-serial", "chardev:ser0",
       "-monitor", "unix:%s,server,nowait" % mon, "-net", "none"]
if kvm:
    cmd += ["-enable-kvm", "-cpu", "host"]
print("qemu: %s" % ("kvm" if kvm else "tcg (no /dev/kvm)"), flush=True)
qemu = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def connect(path, tries=120):
    for _ in range(tries):
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.connect(path)
            return s
        except OSError:
            if qemu.poll() is not None:
                sys.exit("qemu exited: " + qemu.stderr.read().decode(errors="replace")[-600:])
            time.sleep(0.5)
    sys.exit("could not connect to " + path)


s = connect(ser)
s.settimeout(2)
log = open(os.path.join(out, "serial.log"), "wb")
buf = b""


def pump(seconds):
    global buf
    end = time.time() + seconds
    while time.time() < end:
        try:
            chunk = s.recv(65536)
            if chunk:
                buf += chunk
                log.write(chunk)
                log.flush()
        except socket.timeout:
            pass


def wait_for(needle, timeout):
    end = time.time() + timeout
    while time.time() < end:
        if needle in buf:
            return True
        pump(2)
    return needle in buf


def send(line):
    s.sendall(line.encode() + b"\n")


def finish(code):
    try:
        m = connect(mon, tries=10)
        m.settimeout(5)
        time.sleep(1)
        m.sendall(("screendump %s\n" % os.path.join(out, "screen.ppm")).encode())
        time.sleep(5)
        m.sendall(b"quit\n")
        time.sleep(2)
    except SystemExit:
        pass
    try:
        qemu.wait(timeout=20)
    except subprocess.TimeoutExpired:
        qemu.kill()
    sys.exit(code)


t0 = time.time()
pump(25)
send("")  # GRUB waits on its menu; Enter takes the default entry
if not wait_for(b"login:", 1500):
    print("NO LOGIN PROMPT after %d s; the last of what the machine said:" % (time.time() - t0))
    print(buf[-2000:].decode(errors="replace"))
    finish(2)
boot_seconds = round(time.time() - t0)
print("login prompt after %d s" % boot_seconds, flush=True)

send("yantrik")
pump(6)
send("yantrik")
pump(15)
# The session autostarts on tty1; give it time to come up before asking about it.
pump(45 if kvm else 120)

MARK = "YTEST"


def ask(command, timeout=90):
    """Run `command` on the serial console and return what it printed.

    The markers are printed by printf from two halves, so the text `YTEST-BEGIN` / `YTEST-END`
    exists only in what the machine prints, never in the command line the terminal echoes back.
    With `echo YTEST-END` in the line itself, the echo alone satisfied the wait: any command
    slower than a pump or two (release-check takes minutes) came back empty at once, and every
    later command was typed into a terminal still busy with it — the nightly of 23 September
    failed eight checks that way, from one.
    """
    global buf
    buf = b""
    send("printf '%%s-%%s\\n' %s BEGIN; %s; printf '%%s-%%s\\n' %s END" % (MARK, command, MARK))
    wait_for(("%s-END\r" % MARK).encode(), timeout)
    text = buf.decode(errors="replace")
    return text.split(MARK + "-BEGIN")[-1].split(MARK + "-END")[0].strip()


checks = []


def check(name, ok, evidence):
    checks.append({"name": name, "passed": bool(ok), "evidence": evidence})
    print(("ok    " if ok else "FAIL  ") + name + ("" if ok else "\n        " + repr(evidence)[:400]),
          flush=True)


os_release = ask("cat /etc/os-release")
check("the image says what it is", 'NAME="Yantrik OS"' in os_release, os_release)

build = ask("cat /opt/yantrik/BUILD")
check("it carries a BUILD manifest", "git=" in build and "version=" in build, build)
if expect_git:
    check("and it is the revision that was built", "git=" + expect_git[:7] in build, build)

procs = ask("pgrep -af 'labwc|yantrik-ui' | cut -c1-120")
check("the compositor is running the shipped session",
      "labwc" in procs and "/opt/yantrik/bin/yantrik-ui" in procs, procs)

socks = ask("ls /run/user/1000/yantrik/ 2>/dev/null | tr '\\n' ' '")
wanted = ["app-shell.sock", "a11y.sock", "network.sock", "notifications.sock",
          "system-monitor.sock", "weather.sock"]
check("the shell and every autostart service answer on a socket",
      all(w in socks for w in wanted), socks)

# The desktop, asked the way everything else asks it: every app opens and answers, every screen
# shows, the permission gate refuses on every door, the ceiling holds on a service, nothing
# crashes, and the shell is idle when idle. docs/releasing.md: this is the nightly's gate.
rc = ask("XDG_RUNTIME_DIR=/run/user/1000 timeout 900 /opt/yantrik/bin/release-check --tier ci --may-lock "
         "--json /tmp/release-check.json > /tmp/release-check.txt 2>&1; echo exit=$?; "
         "grep -E '^(PASS|FAIL|SKIP)' /tmp/release-check.txt; grep -A1 '^FAIL' /tmp/release-check.txt",
         timeout=960)
check("release-check (tier ci) passes inside the booted image", "exit=0" in rc, rc)

shelved = ask("ls /opt/yantrik/bin | grep -c -E 'music-player|spreadsheet'")
check("no shelved app was packaged", shelved.splitlines()[-1].strip() == "0", shelved)

essentials = ask("for b in yantrik-ui yantrik yos yantrik-update yantrik-session yantrik-notes "
                 "yantrik-email yantrik-calendar network-service email-service; do "
                 "[ -x /opt/yantrik/bin/$b ] || echo MISSING:$b; done; echo checked")
check("the binaries a desktop cannot do without are there",
      "MISSING" not in essentials and "checked" in essentials, essentials)

# What a mind needs to read a web page: the browser's surface (#477), its page reader and the SDK
# vendored beside it, and `websocket` for the system interpreter both yos and the service name.
web = ask("[ -x /opt/yantrik/bin/yantrik-browser ] || echo MISSING:yantrik-browser; "
          "[ -s /opt/yantrik/share/browser/yantrik_browser/page.js ] || echo MISSING:page.js; "
          "[ -d /opt/yantrik/share/browser/yantrik_surface ] || echo MISSING:yantrik_surface; "
          "for f in yos yantrik-browser; do head -1 /opt/yantrik/bin/$f | grep -qx '#!/usr/bin/python3' || echo BAD:shebang:$f; done; "
          "/usr/bin/python3 -c 'import websocket' 2>/dev/null || echo MISSING:python3-websocket; echo checked")
check("the pieces the browser is driven with are all there",
      "MISSING" not in web and "BAD" not in web and "checked" in web, web)

ssh_state = ask("systemctl is-enabled ssh 2>&1 | head -1")
ssh_word = ssh_state.strip().splitlines()[-1].strip() if ssh_state.strip() else ""
check("sshd is not enabled beside a published password",
      ssh_word in ("disabled", "masked", "not-found") or "No such file" in ssh_state, ssh_state)

# The config names no private address at all; the updater names them only as ranges in CIDR or
# inside its marked selftest, and nothing anywhere on 192.168.4.x (private_addresses.py says why).
# grep runs in the guest, where the files are; only its matches come back to be judged here.
private_files = ["/opt/yantrik/config.yaml", "/opt/yantrik/update.conf", "/opt/yantrik/bin/yantrik-update"]
private = ask("grep -HnoE '%s' %s 2>/dev/null; echo scanned" % (private_addresses.GREP_ERE, " ".join(private_files)))
private_found = private_addresses.parse_grep(private.replace("scanned", ""))
private_found.setdefault(private_files[-1], [])
private_bad = private_addresses.problems(private_found)
check("no real network address ships in the config or the updater",
      private.strip().endswith("scanned") and not private_bad, private_bad or private)

update_conf = ask("cat /opt/yantrik/update.conf")
check("the updater is pointed at the public host over https",
      "HOST=releases.yantrikos.com" in update_conf and "SCHEME=https" in update_conf, update_conf)

name = ask("grep -E '^user_name' /opt/yantrik/config.yaml")
check("the config greets nobody in particular", '"User"' in name or "User" in name, name)

report = {"iso": os.path.basename(iso), "kvm": kvm, "boot_seconds": boot_seconds,
          "total_seconds": round(time.time() - t0), "checks": checks,
          "passed": all(c["passed"] for c in checks)}
with open(os.path.join(out, "report.json"), "w") as f:
    json.dump(report, f, indent=2)
failed = [c["name"] for c in checks if not c["passed"]]
print("\n%d checks, %d failed, %d s" % (len(checks), len(failed), report["total_seconds"]))
finish(1 if failed else 0)
