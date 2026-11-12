"""Hermes's model, set for the person by the desktop ("Use Yantrik models"): hermes_model.py apply|revert CONFIG

Run by `yantrik-update harness model hermes` inside the gateway's sandbox, as the mind account, with
Hermes's own Python (hermes-model.sh finds it). Once Hermes runs as the mind account (#411) its
config.yaml is in that account's home, which the person's desktop cannot write, so the desktop's
"Use Yantrik models" cannot write the file the way it does for a harness that is still the person's.
It sends the four `model` settings here instead, by root, on stdin: never in an argument list or the
environment, and never printed.

`apply` reads `{"default", "provider", "base_url", "api_key"}` from stdin and sets those four keys under
`model:`, leaving every other key in the file as it was (above all `platform_toolsets`, the allowlist
that keeps Hermes's own terminal, file, code, browser and web tools off). The model section the file
had is kept once, beside it, and `revert` puts it back (or removes `model:` when there was none).
The address must be this machine's own gateway (http, a loopback name, port 7460, no userinfo: the
host of `http://127.0.0.1:@evil/` is `evil`), so a caller cannot use this to send Hermes's calls, or
the token, anywhere else.
"""

import json
import os
import stat
import sys
import time
from urllib.parse import urlsplit

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hermes_config  # noqa: E402  (Hermes's own Python has PyYAML; this file's neighbour reads and writes the config)

KEYS = ("default", "provider", "base_url", "api_key")
KEPT_SUFFIX = ".model-before-yantrik.json"
MAX_BODY = 8192
# The desktop's model gateway listens on this machine only, on this port.
LOCAL_HOSTS = ("127.0.0.1", "localhost", "::1")
GATEWAY_PORT = 7460


def is_gateway(url):
    """True only for http://<loopback>:7460/...: parsed, so userinfo, a lookalike host or another port
    cannot pass a prefix test (`http://127.0.0.1:@evil/` starts with a loopback prefix)."""
    try:
        parts = urlsplit(url)
        return (parts.scheme == "http" and parts.username is None and parts.password is None
                and "@" not in parts.netloc and parts.hostname in LOCAL_HOSTS and parts.port == GATEWAY_PORT)
    except ValueError:
        return False


class ModelError(Exception):
    """What was sent is not something to write, or the file cannot be changed safely."""


def parse(raw):
    """The four settings from `raw` (JSON text), refused unless they are plain strings and the address
    is this machine's."""
    try:
        spec = json.loads(raw)
    except ValueError:
        raise ModelError("the model settings are not JSON")
    if not isinstance(spec, dict) or set(spec) != set(KEYS) or not all(isinstance(spec[k], str) and spec[k].strip() for k in KEYS):
        raise ModelError("the model settings must be exactly: %s" % ", ".join(KEYS))
    if not is_gateway(spec["base_url"]):
        raise ModelError("the model address is not this machine's gateway; not writing it")
    return {k: spec[k].strip() for k in KEYS}


def _kept(path):
    return path + KEPT_SUFFIX


def _refuse_link(path):
    try:
        if stat.S_ISLNK(os.lstat(path).st_mode):
            raise ModelError("%s is a link; not touching it" % path)
    except FileNotFoundError:
        pass


def _keep_live_model(path, kept, data):
    """A kept section that this tool did not write (no `made_by` mark) is adopted, never replaced. If the
    live `model` section differs from it and is not the gateway's own, that section is the person's newer
    work: copy it, with the config's mode and owner, to `<config>.model-replaced-<unix time>.json`
    (exclusive, never over an earlier copy), and name it in the kept file so Revert can say so (#1201)."""
    try:
        with open(kept, encoding="utf-8") as f:
            before = json.load(f)
    except (OSError, ValueError):
        return
    if before.get("made_by") == "apply" or before.get("replaced"):
        return
    live = data.get("model")
    if live == before.get("model") or (isinstance(live, dict) and is_gateway(str(live.get("base_url", "")))):
        return
    st = os.stat(path)
    now = int(time.time())
    for n in range(1000):
        copy = "%s.model-replaced-%d%s.json" % (path, now, "" if n == 0 else "-%d" % n)
        try:
            fd = os.open(copy, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        except FileExistsError:
            continue
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump({"had": "model" in data, "model": live}, f)
        os.chmod(copy, stat.S_IMODE(st.st_mode))
        if hasattr(os, "chown") and (os.stat(copy).st_uid, os.stat(copy).st_gid) != (st.st_uid, st.st_gid):
            os.chown(copy, st.st_uid, st.st_gid)
        before["adopted"] = True
        before["replaced"] = copy
        with open(kept, "w", encoding="utf-8") as f:
            json.dump(before, f)
        return
    raise ModelError("could not find a free name to keep a copy of the model section; nothing was changed")


def apply(path, spec):
    """Set the four `model` keys in the file at `path`, keeping the earlier section once."""
    _refuse_link(path)
    data, text = hermes_config._read(path)
    kept = _kept(path)
    if not os.path.lexists(kept):
        had = "model" in data
        fd = os.open(kept, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump({"had": had, "model": data.get("model"), "made_by": "apply"}, f)
    else:
        _keep_live_model(path, kept, data)
    model = data.get("model")
    # An older config names the model as a bare string; it becomes the mapping Hermes reads now.
    if not isinstance(model, dict):
        model = data["model"] = {}
    model.update(spec)
    hermes_config._write(path, data, text)


def revert(path):
    """Put the model section back as `apply` found it, and drop the kept copy."""
    _refuse_link(path)
    kept = _kept(path)
    try:
        with open(kept, encoding="utf-8") as f:
            before = json.load(f)
    except FileNotFoundError:
        return _revert_from_before_the_move(path)
    data, text = hermes_config._read(path)
    if before.get("had"):
        data["model"] = before.get("model")
    else:
        data.pop("model", None)
    hermes_config._write(path, data, text)
    # A kept section this tool did not make is not this tool's to delete.
    if not before.get("adopted"):
        os.unlink(kept)
    return before.get("replaced")


def _revert_from_before_the_move(path):
    """Revert with no kept section: "Use Yantrik models" was applied before Hermes moved here, so the
    copied config carries the gateway's settings (and its token) with no record of the section they
    replaced, which stayed in the person's `.hermes.yantrik-moved`. The four keys come out, so Hermes is
    no longer pointed at the gateway and the token is gone from the file; the model is then Hermes's
    to ask for again (`hermes model`). A model that is not the gateway's is left alone."""
    data, text = hermes_config._read(path)
    model = data.get("model")
    if not isinstance(model, dict) or not is_gateway(str(model.get("base_url", ""))):
        raise ModelError("Hermes was not given Yantrik models here; nothing to put back")
    for key in KEYS:
        model.pop(key, None)
    if not model:
        del data["model"]
    hermes_config._write(path, data, text)


def main(argv):
    if len(argv) != 3 or argv[1] not in ("apply", "revert"):
        print("usage: hermes_model.py apply|revert CONFIG", file=sys.stderr)
        return 2
    try:
        if argv[1] == "apply":
            apply(argv[2], parse(sys.stdin.read(MAX_BODY + 1)[:MAX_BODY]))
            replaced = None
        else:
            replaced = revert(argv[2])
    except (ModelError, hermes_config.ConfigError, OSError) as exc:
        # Our own words; the settings (which hold a token) are never in one.
        print(exc, file=sys.stderr)
        return 1
    print("Hermes's model settings %s" % ("now point at Yantrik models" if argv[1] == "apply" else "are back as they were"))
    if argv[1] == "revert" and replaced:
        print("Your model settings from before this change are also kept at %s" % replaced)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
