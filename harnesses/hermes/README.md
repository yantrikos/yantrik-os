# Hermes as the desktop's mind

A Hermes *platform* plugin, not a process of ours. Hermes is a gateway with its own platforms —
Telegram, Slack, IRC — and this makes a Yantrik OS desktop one more of them: what you type in the
chat panel arrives as a message, and Hermes streams the answer back.

Hermes keeps its model, endpoint, keys and memory in `~/.hermes`, as it always has. This plugin
reads none of it except the model name, which it passes as the `detail` the picker shows.

## Install

**Settings → Minds** lists Hermes whether or not it is installed, says which part is
missing, and has an *Install* button that does all of it with the output on the row:
`harnesses/lib/install/hermes.sh` installs Hermes itself when it is not there (Hermes's own
installer, into `~/.hermes`, no root, browser and computer-use tools left out), copies and enables
the plugin, and installs and starts Hermes's gateway as your user service. It also (#677):

- installs the YantrikDB memory provider **from the image** (`/opt/yantrik/memory/wheels`, no
  network, no terminal) and sets `memory.provider: yantrikdb`, with Hermes's own MEMORY.md and
  USER.md off. Hermes keeps a YantrikDB of its own (`~/.local/share/yantrik/memory/hermes.db`)
  until the shared `yantrik` mode can be turned on safely (#586);
- allowlists the desktop platform to the desktop's owner only (`YANTRIK_ALLOWED_USERS=owner` in
  Hermes's `.env`, never `YANTRIK_ALLOW_ALL_USERS`), and holds it to the desktop's tools
  (`hermes_config.py`, below);
- ends with the three proofs (`harnesses/lib/proofs.py hermes`), and is not installed until
  they pass. `harnesses/lib/install/revert.sh hermes` puts every file it wrote back.

When the image carries the model step (#675), Hermes is given your model with no question.
Otherwise Hermes needs a model, which is Hermes's own setting, so the row opens Hermes's own
picker (`hermes model`) in a terminal as soon as the install finishes, and keeps a *Choose model*
button for changing it later. The gateway is restarted afterwards so the running Hermes uses it. There is no *Start*
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

Hermes runs that copy, not the one in `/opt/yantrik/share`, so after every OS update
`yantrik-update` brings it up to the build's (`harnesses/lib/install/refresh-plugins.sh`, run as
you): the desktop plugin and the YantrikDB plugin are replaced only where they differ, with what an
install writes. Nothing else in `~/.hermes` is touched. The update does not restart Hermes's
gateway, since that would cut off a turn in flight; Hermes loads the new plugins when its gateway
next starts, at your next login or reboot, or between turns with
`systemctl --user restart hermes-gateway`. `sudo yantrik-update reconcile` does the same on demand,
and `sh /opt/yantrik/share/harnesses/lib/install/refresh-plugins.sh --restart` from your own
terminal refreshes and restarts the gateway in one go.

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

**On an older Hermes, kanban is turned off everywhere.** A Hermes from before commit `3d7f773`
(10 Sep 2026; releases before `v2026.9.14`, 0.14.0 among them) adds its `kanban` toolset to the desktop platform whatever
the list above says. Kanban's `kanban_create` queues worker agents with toolsets of their own, a
route around the desktop's grading, so the install turns it off in `agent.disabled_toolsets`, the
only switch such a Hermes honours, and says so. That setting is **global**: your other Hermes
platforms (Telegram, Discord, the CLI) lose kanban too. Revert puts your own `config.yaml` back,
kanban included. A current Hermes (the commit the installer pins) honours the list, and nothing
global is written.

## New conversation is not asked twice

Hermes 0.21 asks before `/new`, `/reset` and `/undo` (`approvals.destructive_slash_confirm`). On
the desktop, `/new` and `/reset` only ever come from the person — the Lens's new-conversation
button, or typed in the Lens — so the adapter confirms those once on their behalf and the turn gets
Hermes's reply. `/undo` is still asked, in the chat. Nothing global is written: your other Hermes
platforms keep asking. An older Hermes, which does not ask, is unchanged.

## A question Hermes asks is answered in the chat

When Hermes 0.21 asks you something before it goes on (its `clarify` tool), the question is a card
in the reply it is in: its choices are buttons, the one Hermes recommends is marked, and you can
also type your own answer, in the card or in the chat box, which says it is answering. Hermes's
work then goes on in the same reply, and the card folds to one line saying what you answered.
`/stop` or a new conversation drops the question along with the work.

On a desktop from before cards (one that does not take `harness.event`, or will not take a
question), the question ends the reply it is in instead, with any choices numbered, and your next
message is the answer: a number, the choice's words or your own; Hermes's work goes on in the reply
to it. Hermes's "⏳ Working — N min" line is shown as the reply's status, not in its text. An older
Hermes, which has no `clarify` gateway, is unchanged.

## Yantrik models

Installing Hermes (the ISO, the installer, or **Settings → Minds → Install**) does this with no
question; **Use its own models** on its row undoes it, and **Use Yantrik models** does it again. It
sets Hermes's model in its own
`~/.hermes/config.yaml` to the desktop's model gateway: `model.provider: custom`,
`model.base_url: http://127.0.0.1:7460/v1`, `model.default: picked` and a gateway token of Hermes's
own as `model.api_key` — never a provider's key. `picked` is whichever model you pick for Hermes in
the ask bar. The file's comments are not kept in the new file; Revert puts your own back, comments
and all, and withdraws the token. What Hermes remembers about you goes where you chose per account
in Settings → AI accounts: shared, kept on this computer (the private lane), or not sent.

## Turning it off

`YANTRIK_HARNESS=off` in the Hermes environment keeps the plugin loaded and off the desktop.
`YANTRIK_HARNESS_SOCKET` points it at a socket that is not in the usual place.

## When something goes wrong

Hermes's own log, wherever your install keeps it. Two failures this plugin exists to avoid, and
which are worth knowing if you write another gateway-shaped harness, are in
[docs/harness.md](../../docs/harness.md): closing every turn exactly once, and answering a message
that arrives while it is already working instead of queueing it behind the turn that is owed.
