#!/usr/bin/env python3
"""Self-test for yantrik-publish: runs the real script against a scratch directory.

    python3 deploy/yantrik-os/server/publish_selftest.py

The script is run as a subprocess with SSH_ORIGINAL_COMMAND set, exactly as sshd runs it, with
its two roots pointed at a temporary directory by rewriting the ROOTS table in a copy. Nothing
here touches a network or a real web root.
"""
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = (HERE / "yantrik-publish").read_text(encoding="utf-8")


def make_script(tmp):
    iso_root, rel_root = tmp / "iso", tmp / "releases"
    text = SOURCE.replace('"/var/www/iso.yantrikos.com"', repr(str(iso_root)))
    text = text.replace('"/var/www/releases.yantrikos.com"', repr(str(rel_root)))
    assert str(iso_root) in text and str(rel_root) in text, "ROOTS table not found to rewrite"
    script = tmp / "yantrik-publish"
    script.write_text(text, encoding="utf-8")
    return script, iso_root, rel_root


def run(script, command, data=b""):
    env = dict(os.environ, SSH_ORIGINAL_COMMAND=command)
    return subprocess.run([sys.executable, str(script)], input=data, env=env,
                          capture_output=True)


def sha(data):
    return hashlib.sha256(data).hexdigest()


failures = []


def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else "  -- " + str(detail)))
    if not ok:
        failures.append(name)


with tempfile.TemporaryDirectory() as d:
    tmp = pathlib.Path(d)
    script, iso_root, rel_root = make_script(tmp)
    nightly = iso_root / "nightly"

    # 1. four ISOs in a row: the newest three remain, latest points at the newest
    names = []
    for i in range(1, 5):
        body = b"iso-%d" % i * 1000
        name = "yantrik-os-v0.1.0-%d-gabc%04d.iso" % (i, i)
        names.append(name)
        r = run(script, "iso nightly %s %s" % (name, sha(body)), body)
        check("iso %d is accepted" % i, r.returncode == 0, r.stderr.decode())
        time.sleep(0.05)
        os.utime(nightly / name, (time.time() + i, time.time() + i))
    real = sorted(p.name for p in nightly.iterdir()
                  if p.name.endswith(".iso") and not p.is_symlink())
    check("only the newest three ISOs remain", real == sorted(names[1:]), real)
    check("the pruned ISO's checksum file went with it",
          not (nightly / (names[0] + ".sha256")).exists())
    told = json.loads((nightly / "latest.json").read_text())
    check("latest.json names the newest image, its checksum and its version",
          (told["file"], told["sha256"], told["version"], told["latest"], told["bytes"])
          == (names[-1], sha(b"iso-4" * 1000), "v0.1.0-4", "yantrik-os-latest.iso", len(b"iso-4" * 1000)),
          str(told))
    check("latest points at the newest",
          os.readlink(nightly / "yantrik-os-latest.iso") == names[-1])
    check("the website's old name points at the newest too",
          os.readlink(nightly / "yantrik-os-nightly-latest.iso") == names[-1])
    check("a checksum file is written in sha256sum's format",
          (nightly / (names[-1] + ".sha256")).read_text().split() == [sha(b"iso-4" * 1000), names[-1]])

    # 2. a wrong hash changes nothing
    before = sorted(p.name for p in nightly.iterdir())
    r = run(script, "iso nightly yantrik-os-bad.iso %s" % ("0" * 64), b"not what was promised")
    check("a sha256 mismatch is refused", r.returncode != 0 and b"mismatch" in r.stderr, r.stderr)
    check("and leaves the directory exactly as it was",
          sorted(p.name for p in nightly.iterdir()) == before)

    # 3. nothing but a build name reaches the filesystem
    for bad in ("../../etc/cron.d/x.iso", ".hidden.iso", "yantrik-os-a/b.iso", "index.html",
                "yantrik-os-x.iso;rm", "yantrik-os-latest.iso.sha256"):
        r = run(script, "iso nightly %s %s" % (bad, sha(b"x")), b"x")
        check("refuses the filename %r" % bad, r.returncode != 0)
    check("nothing was written outside the two roots",
          sorted(p.name for p in tmp.iterdir()) == ["iso", "yantrik-publish"]
          or sorted(p.name for p in tmp.iterdir()) == ["iso", "releases", "yantrik-publish"])

    # 4. it is not a shell
    for cmd in ("", "bash -i", "cat /etc/passwd", "iso", "iso staging a b", "scp -t /tmp",
                "release nightly x y"):
        r = run(script, cmd, b"x")
        check("refuses %r" % cmd, r.returncode != 0)

    # 5. a release updates its channel in the manifest and leaves the others alone
    (rel_root).mkdir(parents=True, exist_ok=True)
    (rel_root / "manifest.json").write_text(json.dumps(
        {"channels": {"stable": {"version": "old-stable", "git": "1111111"}}}))
    body = b"tarball" * 500
    name = "yantrik-os-v0.1.0-9-gdeadbee-20260921-deadbee-linux-amd64.tar.zst"
    r = run(script, "release nightly %s %s v0.1.0-9-gdeadbee deadbee 27" % (name, sha(body)), body)
    check("a release bundle is accepted", r.returncode == 0, r.stderr.decode())
    m = json.loads((rel_root / "manifest.json").read_text())
    n = m["channels"].get("nightly", {})
    check("the manifest names the new build",
          (n.get("artifact"), n.get("sha256"), n.get("git"), n.get("binaries"))
          == (name, sha(body), "deadbee", 27), n)
    check("the other channel's entry is untouched",
          m["channels"]["stable"] == {"version": "old-stable", "git": "1111111"})
    check("the updater's latest name points at it",
          os.readlink(rel_root / "nightly" / "yantrik-os-latest-linux-amd64.tar.zst") == name)

    # 6. a changelog sits beside its image, under the image's stem, and latest.json names it
    text = b"# What changed in v0.1.0-4\n\n- one thing\n- another\n"
    stem = names[-1][: -len(".iso")]
    r = run(script, "changelog nightly %s.changelog.md %s" % (stem, sha(text)), text)
    check("a changelog is accepted", r.returncode == 0, r.stderr.decode())
    check("it lands beside the image", (nightly / (stem + ".changelog.md")).read_bytes() == text)
    check("latest.changelog.md follows it",
          os.readlink(nightly / "yantrik-os-latest.changelog.md") == stem + ".changelog.md")
    lj = json.loads((nightly / "latest.json").read_text())
    check("latest.json names the changelog of the image it names",
          lj.get("changelog") == stem + ".changelog.md" and lj.get("file") == names[-1], lj)
    check("the images were not pruned by the changelog",
          sorted(p.name for p in nightly.iterdir() if p.name.endswith(".iso") and not p.is_symlink())
          == sorted(names[-3:]))
    other = b"# not this image\n"
    r = run(script, "changelog nightly yantrik-os-v9.9.9-gffffff0.changelog.md %s" % sha(other), other)
    check("a changelog for another image is stored", r.returncode == 0, r.stderr.decode())
    lj = json.loads((nightly / "latest.json").read_text())
    check("but latest.json still names the one that matches its image",
          lj.get("changelog") == stem + ".changelog.md", lj)
    big = b"x" * (1024 * 1024 + 1)
    r = run(script, "changelog nightly yantrik-os-big.changelog.md %s" % sha(big), big)
    check("a changelog over a megabyte is refused", r.returncode != 0)
    r = run(script, "changelog nightly yantrik-os-not-a-changelog.iso %s" % sha(text), text)
    check("an .iso cannot be published as a changelog", r.returncode != 0)

    # 7. an empty upload is a failure, not an empty file on a download page
    r = run(script, "iso beta yantrik-os-empty.iso %s" % sha(b""), b"")
    check("an empty upload is refused", r.returncode != 0)
    check("and leaves no file", not (iso_root / "beta" / "yantrik-os-empty.iso").exists())

    # 8. a signed manifest (#537): stored under its channel, never backwards, never another shape.
    #    The server cannot check the signature and says so; machines do. Here the signature is
    #    any well-formed 64 bytes.
    def envelope(channel, serial):
        sig = "A" * 86 + "=="
        body = json.dumps({"schema": 1, "channel": channel, "serial": serial}).encode()
        return ("yantrik-manifest v1 ed25519 %s\n" % sig).encode() + body + b"\n"

    stored = rel_root / "nightly" / "release.signed"
    first = envelope("nightly", 100)
    r = run(script, "manifest nightly release.signed %s" % sha(first), first)
    check("a signed manifest is accepted", r.returncode == 0, r.stderr.decode())
    check("and stored byte for byte under its channel", stored.read_bytes() == first)
    newer = envelope("nightly", 101)
    r = run(script, "manifest nightly release.signed %s" % sha(newer), newer)
    check("a higher serial replaces it", r.returncode == 0 and stored.read_bytes() == newer)
    older = envelope("nightly", 50)
    r = run(script, "manifest nightly release.signed %s" % sha(older), older)
    check("a lower serial is refused", r.returncode != 0 and b"lower" in r.stderr, r.stderr)
    check("and the published one is untouched", stored.read_bytes() == newer)
    wrong = envelope("stable", 1000)
    r = run(script, "manifest nightly release.signed %s" % sha(wrong), wrong)
    check("a manifest for another channel is refused", r.returncode != 0)
    plain = json.dumps({"channels": {}}).encode()
    r = run(script, "manifest nightly release.signed %s" % sha(plain), plain)
    check("an unsigned file is refused", r.returncode != 0)
    r = run(script, "manifest nightly release.signed %s" % ("0" * 64), newer)
    check("a hash mismatch is refused", r.returncode != 0 and stored.read_bytes() == newer)
    r = run(script, "manifest nightly other.json %s" % sha(newer), newer)
    check("only release.signed can be written", r.returncode != 0)
    check("the legacy manifest.json was not touched by it",
          "nightly" in json.loads((rel_root / "manifest.json").read_text())["channels"])

print()
print("%d failed" % len(failures) if failures else "all passed")
sys.exit(1 if failures else 0)
