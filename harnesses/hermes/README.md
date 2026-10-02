# Hermes as the desktop's mind

A Hermes *platform* plugin, not a process of ours. Hermes is a gateway with its own platforms —
Telegram, Slack, IRC — and this makes a Yantrik OS desktop one more of them: what you type in the
chat panel arrives as a message, and Hermes streams the answer back.

Hermes keeps its model, endpoint, keys and memory in `~/.hermes`, as it always has. This plugin
reads none of it except the model name, which it passes as the `detail` the picker shows.

## Install

**Settings → Harnesses** lists Hermes whether or not it is installed, says which part is
missing, and has an *Install* button that does all of it with the output on the row:
`harnesses/lib/install/hermes.sh` installs Hermes itself when it is not there (Hermes's own
installer, into `~/.hermes`, no root, browser and computer-use tools left out), copies and enables
the plugin, and installs and starts Hermes's gateway as your user service. Hermes then needs a
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
waiting on those. `hermes tools` does not know plugin platforms, so set it in
`~/.hermes/config.yaml`:

```yaml
platform_toolsets:
  yantrik: [skills, todo, memory, session_search, clarify, delegation, yantrik_os]
delegation:
  max_iterations: 25        # a research sub-agent that may take 50 turns will take 50
```

## Turning it off

`YANTRIK_HARNESS=off` in the Hermes environment keeps the plugin loaded and off the desktop.
`YANTRIK_HARNESS_SOCKET` points it at a socket that is not in the usual place.

## When something goes wrong

Hermes's own log, wherever your install keeps it. Two failures this plugin exists to avoid, and
which are worth knowing if you write another gateway-shaped harness, are in
[docs/harness.md](../../docs/harness.md): closing every turn exactly once, and answering a message
that arrives while it is already working instead of queueing it behind the turn that is owed.
