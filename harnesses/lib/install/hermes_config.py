"""Give Hermes's `yantrik` platform the desktop's tools and nothing of Hermes's own: hermes_config.py apply|check CONFIG

Run by hermes.sh with Hermes's own Python, which has PyYAML and Hermes's own modules, against the
file `hermes config path` names. `hermes config set` cannot write this: it turns every value into a
string, a number or a boolean, never a list. `hermes tools` does not know plugin platforms at all.

Why it matters. Hermes arrives with `terminal`, `file`, `code_execution`, `browser` and `web`
toolsets of its own. On this desktop they are a second, ungraded route to everything the apps
offer: a live machine was found running Hermes's own `terminal` as the person, past every shell
approval, drawing windows on the person's desktop. So the desktop platform is held to an
allowlist, and the desktop's own tools come through the `yantrik_os` MCP server, graded like any
agent's.

A list in the file is not the whole answer, because Hermes adds to it when it resolves a
platform: a plugin toolset the platform has not "seen" is on by default, a composite toolset
expands, and every enabled MCP server is added unless the platform names the ones it wants. So:

- the platform names `yantrik_os`, which makes Hermes treat MCP servers as an allowlist of that
  one. Not Hermes's `no_mcp`: with it, Hermes drops every MCP server the platform names, the
  desktop's own included, and the platform would have no desktop tools at all;
- every plugin toolset Hermes knows now is marked seen for the platform, so none is on by default;
- `skills.inline_shell` is off: with it, a skill's text can run shell commands as it loads;
- `check` asks Hermes's own resolver which toolsets the platform really gets, and fails on
  anything outside the allowlist.

`apply` keeps every other key in the file as it was. The file is backed up once, beside itself,
before the first change this script makes to it, and written whole through a temporary file, so a
failure leaves the old file in place. Comments in it are not kept: Hermes's own `config set` drops
them the same way.
"""

import os
import sys

PLATFORM = "yantrik"
# What harnesses/hermes/README.md asks for: all the desktop platform may have, and what it is given.
ALLOWED = ["skills", "todo", "memory", "session_search", "clarify", "delegation", "yantrik_os"]
YANTRIK_OS = "yantrik_os"
# The desktop's tools, as an MCP server, when the person has not configured it already. The bridge
# may hold a call for 270 seconds while a person answers a card (README.md, "an MCP client must
# allow os_act up to 270 seconds").
YANTRIK_OS_SERVER = {"command": "/opt/yantrik/bin/yos-mcp", "timeout": 300}
# A research sub-agent that may take 50 turns will take 50.
DELEGATION_MAX_ITERATIONS = 25
BACKUP_SUFFIX = ".before-yantrik-desktop"


class ConfigError(Exception):
    """The file is not something this can safely change, or Hermes could not be asked."""


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


def hermes_plugin_toolsets():
    """Every plugin toolset this Hermes knows, from Hermes itself."""
    try:
        from hermes_cli.tools_config import _get_plugin_toolset_keys
    except Exception as exc:
        raise ConfigError("cannot ask Hermes which plugin toolsets it has: %s" % exc)
    return sorted(str(k) for k in _get_plugin_toolset_keys())


def hermes_resolver():
    """Hermes's own answer to which toolsets a platform gets, as the gateway asks it."""
    try:
        from hermes_cli.tools_config import _get_platform_tools
    except Exception as exc:
        raise ConfigError("cannot ask Hermes which toolsets the desktop platform gets: %s" % exc)
    return _get_platform_tools


def apply(path, plugin_toolsets=()):
    """Hold the desktop platform to the allowlist in the file at `path`."""
    import yaml

    data, text = _read(path)
    _section(data, "platform_toolsets", path)[PLATFORM] = list(ALLOWED)
    # Seen, so Hermes's resolver treats each as chosen-off rather than new-and-on.
    known = _section(data, "known_plugin_toolsets", path)
    seen = known.get(PLATFORM)
    if seen is not None and not isinstance(seen, list):
        raise ConfigError("`known_plugin_toolsets.%s` in %s is not a list; not touching it" % (PLATFORM, path))
    known[PLATFORM] = sorted({str(t) for t in (seen or [])} | {str(t) for t in plugin_toolsets})
    servers = _section(data, "mcp_servers", path)
    # Always this entry, whatever is there: a `yantrik_os` that runs another command is a "desktop
    # tools" server the platform's allowlist trusts by name. Only the person's other servers stay.
    servers[YANTRIK_OS] = dict(YANTRIK_OS_SERVER)
    _section(data, "skills", path)["inline_shell"] = False
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
    """The desktop platform's list in the file at `path`, or None when it has none."""
    data, _ = _read(path)
    platforms = data.get("platform_toolsets")
    found = platforms.get(PLATFORM) if isinstance(platforms, dict) else None
    return [str(t) for t in found] if isinstance(found, list) else None


def problems(path, resolve):
    """Everything that gives the desktop platform more than the allowlist, as sentences.

    `resolve(config, platform)` is Hermes's `_get_platform_tools`; tests hand in their own.
    """
    data, _ = _read(path)
    wrong = []
    listed = toolsets(path)
    if listed is None:
        wrong.append("platform_toolsets.%s is not set, so the desktop platform gets every toolset Hermes has" % PLATFORM)
    else:
        extra = [t for t in listed if t not in ALLOWED]
        if extra:
            wrong.append("platform_toolsets.%s lists %s, which the desktop platform may not have" % (PLATFORM, ", ".join(extra)))
    servers = data.get("mcp_servers")
    ours = servers.get(YANTRIK_OS) if isinstance(servers, dict) else None
    if ours != YANTRIK_OS_SERVER:
        wrong.append(
            "mcp_servers.%s is not the desktop's own (command %s, timeout %d), so the desktop platform "
            "has no desktop tools or tools from somewhere else" % (YANTRIK_OS, YANTRIK_OS_SERVER["command"], YANTRIK_OS_SERVER["timeout"])
        )
    skills = data.get("skills")
    if isinstance(skills, dict) and skills.get("inline_shell"):
        wrong.append("skills.inline_shell is on, so a skill can run shell commands as it loads")
    # What the gateway will really hand the platform, from the same raw file it reads.
    try:
        effective = {str(t) for t in resolve(data, PLATFORM)}
    except ConfigError:
        raise
    except Exception as exc:
        raise ConfigError("Hermes could not resolve the desktop platform's toolsets: %s" % exc)
    extra = sorted(effective - set(ALLOWED))
    if extra:
        wrong.append("Hermes resolves the desktop platform to %s as well, which it may not have" % ", ".join(extra))
    return wrong


def reassert(path, plugin_toolsets=hermes_plugin_toolsets, resolver=hermes_resolver):
    """What a gateway start does: say what was wrong with the file as it found it, then put it right.

    Returns `(found, left)`: the problems before and after the re-assertion. `found` is not empty
    when the allowlist had been widened since the last start (`hermes update`, a plugin installed
    by hand, an edit), in which case the gateway that is starting may already have read the
    widened file and the caller must not attach. `left` is not empty when the file could not be
    put right. A file that cannot be read or asked about is a problem, never a pass.
    """
    try:
        found = problems(path, resolver())
    except (ConfigError, OSError) as exc:
        found = [str(exc)]
    try:
        apply(path, plugin_toolsets())
        left = problems(path, resolver())
    except (ConfigError, OSError) as exc:
        left = [str(exc)]
    return found, left


def main(argv, plugin_toolsets=hermes_plugin_toolsets, resolver=hermes_resolver):
    if len(argv) != 3 or argv[1] not in ("apply", "check"):
        print("usage: hermes_config.py apply|check CONFIG", file=sys.stderr)
        return 2
    path = argv[2]
    try:
        if argv[1] == "apply":
            apply(path, plugin_toolsets())
        wrong = problems(path, resolver())
    except (ConfigError, OSError) as exc:
        print(exc, file=sys.stderr)
        return 1
    for line in wrong:
        print(line, file=sys.stderr)
    if wrong:
        return 1
    print("Hermes's desktop platform has only: %s" % ", ".join(toolsets(path)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
