#!/usr/bin/env python3
"""Import free-tier API keys from an env file into the free pool's key file, without showing them.

    python import-keys.py C:\\Users\\sync\\env\\llm.txt --dry-run      # which providers it found
    python import-keys.py llm.txt --vm 520                            # the desktop on VM 520
    python import-keys.py llm.txt --gate                              # the live gate (VM 560)

The file holds `NAME=value` lines (an `export ` prefix, quotes and comments are fine). Only keys
for providers in the free pool (docs/free-tiers.md) are taken. Every other key in the file, paid
ones included, is left where it is and only named. Nothing here ever prints a key or puts one on a
command line: keys travel on stdin into a file only the destination's owner can read, merged with
what is already there.

Destinations (pool key file, one `provider=key` line each):
  --vm 520  ->  /home/yantrik/.config/yantrik/free-pool.env   (yantrik, 600), via node1
  --gate    ->  /etc/yantrik-pool/keys.env                     (root, 600), via node2, VM 560
"""
import argparse
import base64
import json
import os
import re
import shlex
import subprocess
import sys

# Env names -> pool provider id. The first name found for a provider wins.
POOL_NAMES = {
    "groq": ["GROQ_API_KEY", "GROQ_KEY"],
    "openrouter": ["OPENROUTER_API_KEY", "OPENROUTER_KEY", "OR_API_KEY"],
    "gemini": ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_AI_STUDIO_KEY", "GOOGLE_GENERATIVE_AI_API_KEY"],
    "cloudflare": ["CLOUDFLARE_API_TOKEN", "CF_API_TOKEN", "CLOUDFLARE_AI_TOKEN"],
    "cloudflare_account": ["CLOUDFLARE_ACCOUNT_ID", "CF_ACCOUNT_ID"],
    "zai": ["ZAI_API_KEY", "Z_AI_API_KEY", "ZHIPU_API_KEY", "ZHIPUAI_API_KEY", "GLM_API_KEY", "BIGMODEL_API_KEY"],
    "mistral": ["MISTRAL_API_KEY", "MISTRAL_KEY"],
}

# Known names that are deliberately not imported, and why (docs/free-tiers.md).
NOT_POOLED = {
    "QWEN_API_KEY": "Alibaba's free quota is a one-time grant, not in the pool yet",
    "DASHSCOPE_API_KEY": "Alibaba's free quota is a one-time grant, not in the pool yet",
    "NVIDIA_API_KEY": "NVIDIA's free terms are evaluation only",
    "NIM_API_KEY": "NVIDIA's free terms are evaluation only",
    "CEREBRAS_API_KEY": "Cerebras has no renewing free tier",
    "SAMBANOVA_API_KEY": "SambaNova's free use is unclear",
    "COHERE_API_KEY": "Cohere's trial terms forbid this use",
    "TOGETHER_API_KEY": "Together has no free tier",
    "DEEPINFRA_API_KEY": "DeepInfra has no free tier",
    "FIREWORKS_API_KEY": "Fireworks' free credit is $1",
    "OLLAMA_API_KEY": "Ollama Cloud is the paid fallback, set up separately",
    "OPENAI_API_KEY": "paid",
    "ANTHROPIC_API_KEY": "paid",
    "MOONSHOT_API_KEY": "Kimi is paid",
    "KIMI_API_KEY": "Kimi is paid",
}

LINE = re.compile(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$")


def parse(path):
    found = {}
    with open(path, encoding="utf-8-sig") as f:
        for raw in f:
            if raw.lstrip().startswith("#"):
                continue
            m = LINE.match(raw)
            if not m:
                continue
            name, value = m.group(1), m.group(2)
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
                value = value[1:-1]
            if value:
                found[name.upper()] = value
    return found


def pick(found):
    keys, report = {}, []
    for pid, names in POOL_NAMES.items():
        name = next((n for n in names if n in found), None)
        if name:
            keys[pid] = found[name]
            report.append((pid, f"found as {name}"))
        elif pid != "cloudflare_account":
            report.append((pid, "not in the file"))
    if "cloudflare" in keys and "cloudflare_account" not in keys:
        report.append(("cloudflare", "token found but no CLOUDFLARE_ACCOUNT_ID: it cannot be used without the account id"))
    used = {n for names in POOL_NAMES.values() for n in names}
    others = [f"{name}: left alone ({NOT_POOLED.get(name, 'not a free-pool provider')})" for name in sorted(found) if name not in used]
    return keys, report, others


# Runs inside the VM: merge the incoming `provider=key` lines (stdin) into the key file, written
# beside it and renamed over it, 600 and owned by `owner`. Prints only the provider names.
#
# It is started as root but does its work as `owner` (security review of #535, 1 Oct 2026): the
# key file lives in a directory the owner can write, and root following a link the owner planted
# there would have read or handed over any file on the machine (/etc/shadow included). As the
# owner, a planted link reaches nothing the owner could not already reach. The key file itself is
# never followed if it is a link, and its mode is set on the open file, not by name.
MERGE = '''
import os, pwd, stat, sys, tempfile
path, owner = sys.argv[1], sys.argv[2]
pw = pwd.getpwnam(owner)
if os.getuid() == 0 and pw.pw_uid != 0:
    os.setgroups([])
    os.setgid(pw.pw_gid)
    os.setuid(pw.pw_uid)
keep = {}
try:
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
except FileNotFoundError:
    fd = None
except OSError as e:
    sys.exit("refusing: the key file is a link or cannot be opened as a file (" + e.strerror + ")")
if fd is not None:
    with os.fdopen(fd, encoding="utf-8") as f:
        if not stat.S_ISREG(os.fstat(f.fileno()).st_mode):
            sys.exit("refusing: the key file is not a regular file")
        for line in f.read(1 << 20).splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                keep[k] = v
for line in sys.stdin.read(1 << 20).splitlines():
    if "=" in line:
        k, v = line.split("=", 1)
        keep[k] = v
d = os.path.dirname(path)
os.makedirs(d, mode=0o700, exist_ok=True)
fd, tmp = tempfile.mkstemp(dir=d, prefix=".free-pool.")
with os.fdopen(fd, "w", encoding="utf-8") as f:
    os.fchmod(f.fileno(), 0o600)
    f.write("".join(k + "=" + v + chr(10) for k, v in sorted(keep.items())))
    f.flush()
    os.fsync(f.fileno())
os.rename(tmp, path)
print(" ".join(sorted(keep)))
'''


def push(keys, node, vm, path, owner):
    code = "import base64;exec(base64.b64decode(" + repr(base64.b64encode(MERGE.encode()).decode()) + "))"
    remote = " ".join(["qm", "guest", "exec", vm, "--timeout", "60", "--pass-stdin", "1", "--",
                       "python3", "-I", "-c", shlex.quote(code), shlex.quote(path), shlex.quote(owner)])
    ssh = ["ssh", "-o", "BatchMode=yes"]
    deploy_key = os.path.expanduser("~/.ssh/id_deploy")
    if os.path.exists(deploy_key):
        ssh += ["-i", deploy_key]
    # The keys go on stdin only: never on a command line, never in a file on the way.
    payload = "".join(k + "=" + v + "\n" for k, v in keys.items())
    r = subprocess.run(ssh + [f"root@{node}", remote], input=payload, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"could not reach VM {vm}: {r.stderr.strip()[-300:]}")
    out = json.loads(r.stdout or "{}")
    if out.get("exitcode") != 0:
        sys.exit(f"VM {vm} could not store the keys: {out.get('err-data', '')[-300:]}")
    return out.get("out-data", "").strip()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("file")
    where = ap.add_mutually_exclusive_group(required=True)
    where.add_argument("--dry-run", action="store_true", help="only say which providers were found")
    where.add_argument("--vm", choices=["520"], help="the desktop on this VM")
    where.add_argument("--gate", action="store_true", help="the live gate, VM 560")
    a = ap.parse_args()

    keys, report, others = pick(parse(a.file))
    print("Free-pool providers:")
    for pid, what in report:
        print(f"  {pid}: {what}")
    if others:
        print("Other keys in the file:")
        for line in others:
            print(f"  {line}")
    if a.dry_run or not keys:
        if not keys:
            print("No free-pool keys found; nothing stored.")
        return
    if a.gate:
        stored = push(keys, "192.168.4.152", "560", "/etc/yantrik-pool/keys.env", "root")
    else:
        stored = push(keys, "192.168.4.151", "520", "/home/yantrik/.config/yantrik/free-pool.env", "yantrik")
    print(f"Stored. The key file now holds: {stored}")


if __name__ == "__main__":
    main()
