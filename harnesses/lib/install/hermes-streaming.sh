# Turn on Hermes's streaming so the desktop gets its answers as they are written (#737).
#
# Hermes's own config (~/.hermes/config.yaml) ships with a top-level `streaming:` block whose
# `enabled:` is `false`, so the gateway hands each platform only the finished answer. The desktop
# plugin wants the progressive edits, so this flips that one key to `true` and leaves every other
# line byte-for-byte. Sourced (never run) by hermes.sh and by yantrik-update, so the one
# implementation is shared and the two callers cannot drift apart.
#
# enable_hermes_streaming HOME_DIR
#
# Edits HOME_DIR/.hermes/config.yaml with python3 (stdlib only, no PyYAML):
#   * a top-level `streaming:` block whose `enabled:` is `false` -> `true`, nothing else touched;
#   * no top-level `streaming:` block -> append `streaming:\n  enabled: true\n`;
#   * already `true` -> change nothing and print nothing;
#   * no config file -> return 0 silently (Hermes is not set up yet);
#   * turned on by this once already (the backup is there) -> change nothing: the person may have
#     turned it back off, and the updater runs this at every update.
# Only the top-level `streaming:` key (column 0) is touched, never `display.streaming` or a
# model-level `streaming`. Writes through a temp file in the same directory, keeps the mode, moves
# it over, and keeps a one-time backup `config.yaml.before-streaming`. Prints one `say` line when
# it changed something.
# The callers source common.sh (hermes.sh) or define say themselves (yantrik-update); a test that
# sources only this file still needs a say to print through.
command -v say >/dev/null 2>&1 || say() { printf '%s\n' "$*"; }

enable_hermes_streaming() {
    home=$1
    cfg="$home/.hermes/config.yaml"
    [ -f "$cfg" ] || return 0
    py="$(command -v python3)" || return 0
    out="$("$py" - "$cfg" <<'PY'
import os, re, stat, sys

path = sys.argv[1]
if not os.path.exists(path):
    sys.exit(0)
# Turned on once, by this, which kept the config as it was beside it. From then on the setting
# is the person's: one they turned back off stays off, at every update and every Install.
if os.path.exists(path + ".before-streaming"):
    sys.exit(0)

with open(path) as f:
    lines = f.readlines()

def flip(line):
    # An indented `enabled: <value>` line inside the streaming block. Only the key at the start of
    # the line (after indentation) counts, never an `enabled:` inside a comment.
    body = line.rstrip("\n")
    nl = line[len(body):]
    stripped = body.lstrip()
    if not stripped.startswith("enabled:"):
        return None
    rest = stripped[len("enabled:"):]
    m = re.match(r"^(\s*)false(\s*(?:#.*)?)$", rest)
    if not m:
        return None
    indent = body[:len(body) - len(stripped)]
    return indent + "enabled:" + m.group(1) + "true" + m.group(2) + nl

block = None
for i, line in enumerate(lines):
    if line.startswith("streaming:"):
        block = i
        break

changed = False
new_lines = list(lines)
if block is None:
    new_lines.append("streaming:\n")
    new_lines.append("  enabled: true\n")
    changed = True
else:
    j = block + 1
    while j < len(lines) and lines[j][:1] in (" ", "\t"):
        flipped = flip(lines[j])
        if flipped is not None:
            new_lines[j] = flipped
            changed = True
            break
        j += 1

if not changed:
    sys.exit(0)

st = os.stat(path)
mode = stat.S_IMODE(st.st_mode)
backup = path + ".before-streaming"
if not os.path.exists(backup):
    with open(path, "rb") as src, open(backup, "wb") as dst:
        while True:
            chunk = src.read(65536)
            if not chunk:
                break
            dst.write(chunk)
    os.chmod(backup, mode)
tmp = path + ".tmp.%d" % os.getpid()
with open(tmp, "w") as f:
    f.writelines(new_lines)
os.chmod(tmp, mode)
os.replace(tmp, path)
print("Hermes now streams its answers to the desktop")
PY
)" || return 1
    [ -n "$out" ] && say "$out"
    return 0
}
