# Hermes as the desktop's mind

A Hermes *platform* plugin, not a process of ours. Hermes is a gateway with its own platforms —
Telegram, Slack, IRC — and this makes a Yantrik OS desktop one more of them: what you type in the
chat panel arrives as a message, and Hermes streams the answer back.

Hermes keeps its model, endpoint and keys in `~/.hermes`, as it always has. This plugin reads none
of it except the model name, which it passes as the `detail` the picker shows.

Its memory is not in `~/.hermes`. On this machine Hermes remembers in YantrikDB, the memory Yantrik
Mind keeps, so both minds know the same person and Hermes keeps no second record of them; see
[Memory](#memory) below.

## Install

**Settings → Harnesses** lists Hermes whether or not it is installed, says which part is
missing, and has an *Install* button that does all of it with the output on the row:
`harnesses/lib/install/hermes.sh` installs Hermes itself when it is not there (Hermes's own
installer, into `~/.hermes`, no root, browser and computer-use tools left out), copies and enables
the plugin, installs the YantrikDB memory provider and makes it Hermes's memory, and installs and
starts Hermes's gateway as your user service. Hermes then needs a
model, which is Hermes's own setting, so the row opens Hermes's own picker (`hermes model`) in a
terminal as soon as the install finishes, and keeps a *Choose model* button for changing it
later. The gateway is restarted afterwards so the running Hermes uses it. There is no *Start*
button and there should not be: Hermes starts Hermes, and this plugin runs inside its gateway.

On a machine that already has Hermes, the same thing by hand:

```sh
mkdir -p ~/.hermes/plugins/yantrik
cp -r /opt/yantrik/share/harnesses/hermes/. ~/.hermes/plugins/yantrik/
hermes plugins enable yantrik-desktop
systemctl --user restart hermes-gateway   # or however Hermes is started

yos act shell use_harness id=hermes       # once it appears in the picker
```

From a checkout, `cp -r harnesses/hermes ~/.hermes/plugins/yantrik` instead.

## Give the desktop's tools, not Hermes's own

Hermes arrives with a `terminal`, `file`, `code_execution`, `browser` and `web` toolset. On this
desktop they are a second, **ungraded** route to everything the apps already offer, and each
`terminal` call stops on Hermes's own approval — which reaches the person as a paragraph to answer
with `/approve`, five minutes at a time. The first long job given to it spent most of its life
waiting on those. *Install* sets this in `~/.hermes/config.yaml` for you
(`harnesses/lib/install/hermes_config.py`), keeping every other key and a copy of the file as it
was (`config.yaml.before-yantrik-desktop`). The list is an allowlist, and the install fails unless
Hermes's own resolver gives the desktop platform nothing outside it. That check matters because
Hermes adds to a platform's list when it resolves it: a plugin toolset the platform has not seen is
on by default, and every enabled MCP server is added unless the platform names the ones it wants.
So the install also marks every plugin toolset Hermes knows as seen for the platform, adds the
`yantrik_os` MCP server when it is missing (naming it is what makes Hermes keep every other server
off), and turns `skills.inline_shell` off. It does not use Hermes's `no_mcp`, which drops the
servers a platform names as well, `yantrik_os` with them. `hermes tools` does not know plugin
platforms and `hermes config set` cannot write a list, so by hand it is:

```yaml
platform_toolsets:
  yantrik: [skills, todo, memory, session_search, clarify, delegation, yantrik_os]
delegation:
  max_iterations: 25        # a research sub-agent that may take 50 turns will take 50
```

## Memory

`harness.yaml` says `memory: yantrikdb`. Pressing *Install* on the row does three things for it:

- the installer puts the YantrikDB memory provider into Hermes (from a commit, its dependencies
  from a lock of exact versions and hashes), sets `memory.provider` to it, writes
  `YANTRIKDB_MODE=yantrik` into `~/.hermes/.env` (only when it is not set there already), and turns
  off Hermes's own `MEMORY.md` and `USER.md`, which a provider runs beside rather than replaces;
- once the install has worked, and not before, the desktop grants Hermes ordinary recall,
  remember and believe in `~/.config/yantrik/memory-grants.json`. Never health, finance or
  household memory, and credentials are never a grant;
- from Hermes's next turn, each turn carries a credential for the memory server. This plugin
  registers it with the provider inside Hermes's process, for that turn's gateway session alone,
  and never puts it in the environment: the gateway serves every platform from one process, and
  what is in its environment is every platform's, and every command's they start. A turn without
  one clears it, so a grant taken away stops working at the next turn.

The Minds row then reads *Memory: YantrikDB (shared with Yantrik Mind)*. Only the person's own
click grants it: an agent that asks for the install through `install_harness` gets Hermes
installed and grants it nothing.

Taking the memory away reaches Hermes at once, not at its next turn: the desktop answers the
harness's next poll with `memory_revoked`, and the adapter calls `set_desktop_credential(key, None)`
for every session it ever registered one under. The shell stops vouching for the credential in
`memory_validate` in the same moment. A turn the provider cannot take a credential for, or one
whose address is not loopback `http://` or `unix:/abs/path` (checked here as well as in the
provider), ends with no credential held for that session: the adapter fails closed.

There is no button to take the grant back yet. Until there is, set `"hermes": {}` in
`~/.config/yantrik/memory-grants.json`. An entry that grants nothing is a revocation: installing
Hermes again does not give the memory back, and the row says so. (Removing the entry instead
means nobody has decided, and the next Install grants it again.) The desktop keeps a narrowing
made in that file, and ignores a widening it did not make.

**The allowlist is held at every start.** The adapter re-asserts the desktop platform's toolset
allowlist and the `yantrik_os` MCP server (`/opt/yantrik/bin/yos-mcp`, timeout 300) whenever the
gateway starts (`guard.py`, with `hermes_config.py` copied beside it by the installer), and checks
them again at every attach. If `hermes update`, a plugin or an edit had widened them, the file is put
back, that start does not attach, and the Hermes row says why. Restart the gateway to attach.

**What this does not protect against.** The id a harness attaches under is its own word, so any
process running as the person can attach as `hermes` and be handed Hermes's credential with its
turns. That is the known limit of minds that run as the person (#411): per-mind grants hold
against a mind that plays by the desktop's rules, not against another of the person's own
processes, until each mind runs under an account of its own.

## Turning it off

`YANTRIK_HARNESS=off` in the Hermes environment keeps the plugin loaded and off the desktop.
`YANTRIK_HARNESS_SOCKET` points it at a socket that is not in the usual place.

## When something goes wrong

Hermes's own log, wherever your install keeps it. Two failures this plugin exists to avoid, and
which are worth knowing if you write another gateway-shaped harness, are in
[docs/harness.md](../../docs/harness.md): closing every turn exactly once, and answering a message
that arrives while it is already working instead of queueing it behind the turn that is owed.
