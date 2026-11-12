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
A live section that is not the one this tool wrote (its digest is kept, so a key added to it counts) is copied, never written over (the shared
table harnesses/tests/fixtures/keep_live_cases.json is the rule); the desktop is told where, on
`NOTE: ` lines, since this runs as another account and its files are not readable from the desktop.
The address must be this machine's own gateway (http, a loopback name, port 7460, no userinfo: the
host of `http://127.0.0.1:@evil/` is `evil`), so a caller cannot use this to send Hermes's calls, or
the token, anywhere else.
"""

import copy
import hashlib
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


def _load_kept(kept):
    """The kept section's record, or None when there is none. One that cannot be read is the person's
    only copy of their section: it is reported in words and left as it is, never replaced."""
    try:
        with open(kept, encoding="utf-8") as f:
            before = json.load(f)
    except FileNotFoundError:
        return None
    except (OSError, ValueError):
        raise ModelError("the kept copy of the model section (%s) cannot be read; leaving it as it is" % kept)
    if not isinstance(before, dict):
        raise ModelError("the kept copy of the model section (%s) is not what this tool writes; leaving it as it is" % kept)
    return before


def _publish(path, record, mode=0o600, owner=None, exclusive=False):
    """Write `record` as JSON to a temp file beside `path` (mode first, so a key is never readable by
    others), then put it in place: `os.replace` over `path`, or, `exclusive`, a hard link that fails
    rather than replace an existing file. Nothing is ever left half-written under `path`."""
    tmp = "%s.tmp-%d" % (path, os.getpid())
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(record, f)
        os.chmod(tmp, mode)
        if owner and hasattr(os, "chown") and (os.stat(tmp).st_uid, os.stat(tmp).st_gid) != owner:
            os.chown(tmp, *owner)
        if exclusive:
            os.link(tmp, path)
        else:
            os.replace(tmp, path)
    finally:
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass


def _keep_copy(path, data):
    """Copy the live `model` section to `<config>.model-replaced-<unix time>.json`, with the config's mode
    and owner (it can hold an API key), exclusive, never over an earlier copy. Returns its path."""
    st = os.stat(path)
    now = int(time.time())
    record = {"had": "model" in data, "model": data.get("model")}
    for n in range(1000):
        copy = "%s.model-replaced-%d%s.json" % (path, now, "" if n == 0 else "-%d" % n)
        try:
            _publish(copy, record, stat.S_IMODE(st.st_mode), (st.st_uid, st.st_gid), exclusive=True)
        except FileExistsError:
            continue
        return copy
    raise ModelError("could not find a free name to keep a copy of the model section; nothing was changed")


def _digest(model):
    """sha256 of a `model` section's canonical JSON (never printed)."""
    return hashlib.sha256(json.dumps(model, sort_keys=True, default=str).encode("utf-8")).hexdigest()


def _is_own_section(model, before):
    """The live section is what this tool wrote: its digest is among the ones the kept record holds
    (every section this tool wrote or was about to write). Pointing at the gateway is not enough: the
    person can add a key (`context_length`) and keep the address. A record from before digests were kept
    has none, and then the address is all there is to go by."""
    if not isinstance(model, dict):
        return False
    if "written" not in before:
        return is_gateway(str(model.get("base_url", "")))
    return _digest(model) in before["written"]


def _differs_from_kept(data, before):
    """The live section is something to keep: there is one, and it is not the kept one."""
    return "model" in data and (data["model"] != before.get("model") or not before.get("had"))


def _copies_of(before):
    """The copies the kept record names (a list; an earlier version of this file kept one string)."""
    replaced = before.get("replaced") or []
    return list(replaced if isinstance(replaced, list) else [replaced])


def _keep_live_model(path, kept, before, data):
    """The shared rule (harnesses/tests/fixtures/keep_live_cases.json is the rule; provider_handoff in
    the desktop runs the same table): nothing of the person's is overwritten without a copy. When no
    apply of this tool is in force (the live `model` section is not one this tool wrote), a live section
    that differs from the kept one is the person's newer work, and is copied before anything writes over it.
    Compared on every Apply, however many cycles came before; the copies are a list, and the kept
    record names each (#1201, #1205)."""
    replaced = _copies_of(before)
    if _is_own_section(data.get("model"), before) or not _differs_from_kept(data, before):
        return replaced
    replaced.append(_keep_copy(path, data))
    before["replaced"] = replaced
    _publish(kept, before)
    return replaced


def apply(path, spec):
    """Set the four `model` keys in the file at `path`, keeping the earlier section once. Returns the
    copies made of the person's own section by this call."""
    _refuse_link(path)
    data, text = hermes_config._read(path)
    kept = _kept(path)
    before = _load_kept(kept)
    copies = []
    if before is None:
        # A copy: `model.update` below changes the live section in place, and the record keeps what was there.
        before = {"had": "model" in data, "model": copy.deepcopy(data.get("model")), "made_by": "apply", "written": []}
        _publish(kept, before, exclusive=True)
    else:
        copies = _keep_live_model(path, kept, before, data)
    model = data.get("model")
    # An older config names the model as a bare string; it becomes the mapping Hermes reads now.
    if not isinstance(model, dict):
        model = data["model"] = {}
    model.update(spec)
    # Recorded before the config is written, and added to what is there: a write that fails leaves the
    # section of the apply before it still known as this tool's.
    if "written" in before:
        digest = _digest(model)
        if digest not in before["written"]:
            before["written"].append(digest)
            _publish(kept, before)
    hermes_config._write(path, data, text)
    return copies


def revert(path):
    """Put the model section back as `apply` found it. A live section edited since (not the gateway's,
    not the kept one) is copied first. Returns (older kept section set aside or None, copies of the
    person's own section: those Apply kept and any made now)."""
    _refuse_link(path)
    kept = _kept(path)
    before = _load_kept(kept)
    if before is None:
        _revert_from_before_the_move(path)
        return None, []
    data, text = hermes_config._read(path)
    copies = _copies_of(before)
    if not _is_own_section(data.get("model"), before) and _differs_from_kept(data, before):
        copies.append(_keep_copy(path, data))
    if before.get("had"):
        data["model"] = before.get("model")
    else:
        data.pop("model", None)
    hermes_config._write(path, data, text)
    if before.get("made_by") == "apply":
        os.unlink(kept)
        return None, copies
    # A kept section this tool did not make is not this tool's to delete, and is not to be put back by
    # every later Revert either: it is set aside under a new name.
    return _set_aside(kept), copies


def _set_aside(kept):
    now = int(time.time())
    for n in range(1000):
        to = "%s.found-%d%s" % (kept, now, "" if n == 0 else "-%d" % n)
        try:
            os.link(kept, to)
        except FileExistsError:
            continue
        os.unlink(kept)
        return to
    raise ModelError("could not find a free name to set the kept model section aside")


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


def _notes(found, copies):
    """The lines the desktop shows the person (`NOTE: `); paths only, never a key. The copies are in the
    Hermes account's own folder, which the desktop cannot open, so the way to read one is said."""
    out = []
    if found:
        out.append("NOTE: Put back an older model section that was already kept beside it (set aside, not deleted, at %s)." % found)
    if copies:
        out.append("NOTE: Your own model settings that this replaced are kept at %s. They are in the Hermes account's folder; "
                   "an administrator can read one with: sudo cat <that path>" % ", ".join(copies))
    return out


def main(argv):
    if len(argv) != 3 or argv[1] not in ("apply", "revert"):
        print("usage: hermes_model.py apply|revert CONFIG", file=sys.stderr)
        return 2
    try:
        if argv[1] == "apply":
            found, copies = None, apply(argv[2], parse(sys.stdin.read(MAX_BODY + 1)[:MAX_BODY]))
        else:
            found, copies = revert(argv[2])
    except (ModelError, hermes_config.ConfigError, OSError, ValueError) as exc:
        # Our own words; the settings (which hold a token) are never in one.
        print(exc if isinstance(exc, (ModelError, hermes_config.ConfigError)) else "could not change the model settings: %s"
              % type(exc).__name__, file=sys.stderr)
        return 1
    if argv[1] == "apply":
        print("Hermes's model settings now point at Yantrik models")
    else:
        print("Hermes's model settings are back %s" % ("as an older kept section had them" if found else "as they were"))
    for line in _notes(found, copies):
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
