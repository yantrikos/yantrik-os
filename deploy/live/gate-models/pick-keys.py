#!/usr/bin/env python3
"""Read the cloud providers' keys from a NAME=value file and write them for setup-models.sh.

    python3 pick-keys.py ~/env/llm.txt | ssh root@node2 'cd /root/live-setup/gate-models && sh setup-models.sh'

Run by the person who owns the file. The keys go to stdout and only to a pipe: if stdout is a
terminal, nothing is written, so a key cannot land on a screen or in a scrollback. What was found
is said on stderr by name, never by value. The file is read with import-keys.py's own parser, so
both scripts read the same file the same way.

A provider whose key is not in the file is left out. setup-models.sh then keeps the key the gate
already has for it.
"""
import importlib.util
import os
import sys

# NGC_API_KEY is deliberately not an alias: an NGC personal key can carry scopes beyond the NIM API
# (registry, org), and a key installed on the gate should be one made for build.nvidia.com only.
NAMES = {
    "OLLAMA_CLOUD_KEY": ["OLLAMA_CLOUD_KEY", "OLLAMA_API_KEY", "OLLAMA_CLOUD_API_KEY"],
    "NANOGPT_KEY": ["NANOGPT_KEY", "NANOGPT_API_KEY", "NANO_GPT_API_KEY", "NANO_GPT_KEY"],
    "NIM_KEY": ["NIM_KEY", "NVIDIA_API_KEY", "NVIDIA_NIM_API_KEY"],
}


def parser():
    here = os.path.dirname(os.path.abspath(__file__))
    path = os.path.join(here, "..", "..", "free-pool", "import-keys.py")
    spec = importlib.util.spec_from_file_location("import_keys", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.parse


def main(argv):
    if len(argv) != 2:
        sys.exit("usage: pick-keys.py <NAME=value file> | ssh root@node2 'cd …/gate-models && sh setup-models.sh'")
    if sys.stdout.isatty():
        sys.exit("stdout is a terminal; pipe this into setup-models.sh so the keys are never shown")
    found = parser()(argv[1])
    lines = []
    for out, names in NAMES.items():
        name = next((n for n in names if n in found), None)
        if name:
            lines.append(f"{out}={found[name]}")
            print(f"{out}: from {name}", file=sys.stderr)
        else:
            print(f"{out}: not in the file; the gate keeps the one it has", file=sys.stderr)
    if not lines:
        sys.exit("none of the providers' keys is in the file")
    # Bytes, not text: on Windows a text stdout turns each "\n" into "\r\n", and a key ending in
    # "\r" is refused by setup-models.sh's format check.
    sys.stdout.buffer.write(("\n".join(lines) + "\n").encode())


if __name__ == "__main__":
    main(sys.argv)
