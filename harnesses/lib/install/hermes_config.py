"""Give Hermes's `yantrik` platform the desktop's tools and not Hermes's own: hermes_config.py apply|check CONFIG

Run by hermes.sh with Hermes's own Python, which has PyYAML, against the file `hermes config path`
names. `hermes config set` cannot write this: it turns every value into a string, a number or a
boolean, never a list. `hermes tools` does not know plugin platforms at all.

Why it matters. Hermes arrives with `terminal`, `file`, `code_execution`, `browser` and `web`
toolsets of its own. On this desktop they are a second, ungraded route to everything the apps
offer: a live machine was found running Hermes's own `terminal` as the person, past every shell
approval, drawing windows on the person's desktop. With `platform_toolsets.yantrik` set, the
desktop platform gets only what is listed here, and the desktop's own tools come through
`yantrik_os`, graded like any agent's.

`apply` keeps every other key in the file as it was. The file is backed up once, beside itself,
before the first change this script makes to it, and written whole through a temporary file, so a
failure leaves the old file in place. Comments in it are not kept: Hermes's own `config set` drops
them the same way.

`check` reads the file back and exits non-zero, saying why, if the platform is missing its list or
has any of Hermes's own toolsets in it.
"""

import os
import sys

# What harnesses/hermes/README.md asks for, and what the desktop platform is given.
YANTRIK_TOOLSETS = ["skills", "todo", "memory", "session_search", "clarify", "delegation", "yantrik_os"]
# Hermes's own routes around the desktop. None of them may be on the desktop platform.
FORBIDDEN = ("terminal", "file", "code_execution", "browser", "web")
# A research sub-agent that may take 50 turns will take 50.
DELEGATION_MAX_ITERATIONS = 25
BACKUP_SUFFIX = ".before-yantrik-desktop"


class ConfigError(Exception):
    """The file is not something this can safely change."""


def _read(path):
    import yaml

    if not os.path.exists(path):
        return {}, None
    with open(path, encoding="utf-8") as f:
        text = f.read()
    try:
        data = yaml.safe_load(text)
    except yaml.YAMLError as exc:
        raise ConfigError("%s is not YAML Hermes could read either: %s" % (path, exc))
    if data is None:
        return {}, text
    if not isinstance(data, dict):
        raise ConfigError("%s is not a mapping of settings" % path)
    return data, text


def _section(data, key, path):
    """data[key] as a mapping, made when missing; refused when it is something else."""
    value = data.setdefault(key, {})
    if value is None:
        value = data[key] = {}
    if not isinstance(value, dict):
        raise ConfigError("`%s` in %s is not a mapping; not touching it" % (key, path))
    return value


def apply(path):
    """Set the desktop platform's toolsets and the delegation limit in the file at `path`."""
    import yaml

    data, text = _read(path)
    _section(data, "platform_toolsets", path)["yantrik"] = list(YANTRIK_TOOLSETS)
    _section(data, "delegation", path)["max_iterations"] = DELEGATION_MAX_ITERATIONS

    directory = os.path.dirname(os.path.abspath(path))
    os.makedirs(directory, exist_ok=True)
    # Once: a second install must not replace the person's own file with the first one's output.
    backup = path + BACKUP_SUFFIX
    if text is not None and not os.path.exists(backup):
        fd = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
    mode = os.stat(path).st_mode & 0o777 if text is not None else 0o600
    temp = os.path.join(directory, ".config.yaml.yantrik-%d.tmp" % os.getpid())
    try:
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            yaml.safe_dump(data, f, sort_keys=False, default_flow_style=False, allow_unicode=True)
            f.flush()
            os.fsync(f.fileno())
        os.replace(temp, path)
    except BaseException:
        if os.path.exists(temp):
            os.unlink(temp)
        raise


def toolsets(path):
    """The desktop platform's toolsets in the file at `path`, or None when it has no list."""
    data, _ = _read(path)
    platforms = data.get("platform_toolsets")
    found = platforms.get("yantrik") if isinstance(platforms, dict) else None
    return [str(t) for t in found] if isinstance(found, list) else None


def problems(path):
    """What is wrong with the desktop platform's toolsets in the file at `path`, if anything."""
    found = toolsets(path)
    if found is None:
        return ["platform_toolsets.yantrik is not set, so the desktop platform gets every toolset Hermes has"]
    own = [t for t in found if t in FORBIDDEN]
    if own:
        return ["platform_toolsets.yantrik still has Hermes's own %s" % ", ".join(own)]
    return []


def main(argv):
    if len(argv) != 3 or argv[1] not in ("apply", "check"):
        print("usage: hermes_config.py apply|check CONFIG", file=sys.stderr)
        return 2
    try:
        if argv[1] == "apply":
            apply(argv[2])
        wrong = problems(argv[2])
    except (ConfigError, OSError) as exc:
        print(exc, file=sys.stderr)
        return 1
    for line in wrong:
        print(line, file=sys.stderr)
    if wrong:
        return 1
    print("Hermes's desktop platform has only: %s" % ", ".join(toolsets(argv[2])))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
