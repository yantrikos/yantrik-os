# Security

Yantrik OS is a desktop that AI agents operate beside a person, so a hole in it can let an agent,
a web page or another program act as that person. Reports are taken seriously and answered.

## Reporting a vulnerability

Please do not open a public issue. Report it privately through GitHub: the repository's
**Security** tab, then **Report a vulnerability**. Say what you found, how to reproduce it, and what
it lets someone do. A proof of concept against a VM is welcome; please do not test against
machines you do not own.

You will hear back within a few days. Fixes to security problems go ahead of other work, and the
report is credited in the fix unless you would rather it was not.

## What is in scope

Anything in this repository, and especially the boundaries it draws:

- the permission gate and approval cards: an agent acting above its grade, or a grant it did not
  get from a person (`crates/yantrik-ui/src/control*.rs`, `crates/yantrik-ipc-transport`);
- the taint rules that stop a session that read something private from sending it out
  (`crates/yantrik-companion-core`, `deploy/yantrik-os/yos-mcp`);
- the vault and the keys it holds;
- the updater and the release path (`deploy/yantrik-os/yantrik-update`, `deploy/yantrik-os/server`);
- the lock screen and the session.

Known limits that are documented, not hidden, are listed in the README's "Status and known limits"
section and in the open issues labelled security; a report that one of them is worse than it says
is in scope too.

## What the AI can do without asking

Read this before you give the built-in companion a model.

At the setting the image ships with, the built-in companion runs its **Safe, Standard and
Sensitive** tools with **no approval card**. Today that includes running shell commands and
reading the screen. The shipped ceiling is `sensitive` (`max_permission` in
`deploy/yantrik-os/config-default.yaml`), and the companion's tool loop checks that ceiling and
nothing else: the "ask" answers of the policy engine are logged, and the tool still runs. The
file tools refuse protected places such as `.ssh`. A command is held only to a short list of
forbidden words, and a screen read is not held to that list at all. What ran is written to `~/.local/share/yantrik/mind-audit.jsonl`.

A Mind or a harness (Hermes, Pi, OpenClaw, DeepSeek) is different: its shell commands and its
screen reads are asked on a card. Two things still answer later calls with no card. **Allow for
this session** (or a standing rule) answers every later command until the shell restarts, and the
`bypass_all` mode asks nothing.

This is a known gap, and it is open. These issues track closing it:

- [#1247](https://github.com/yantrikos/yantrik-os/issues/1247) and
  [#1248](https://github.com/yantrikos/yantrik-os/pull/1248): a credential copied outside its
  folder can be read; #1248 refuses secret file names everywhere. It is not merged.
- [#1265](https://github.com/yantrikos/yantrik-os/issues/1265): a session rule or `bypass_all`
  runs commands that read protected files with no card for the call.
- [#1266](https://github.com/yantrikos/yantrik-os/issues/1266): a raised ceiling, and a MCP
  server you add, reach more than the shipped setting does, with no card per call.
- [#1267](https://github.com/yantrikos/yantrik-os/issues/1267): what a Mind is shown in Mind
  View and in `describe`.
- [#542](https://github.com/yantrikos/yantrik-os/issues/542): some companion tools read
  credential files or change state while graded Safe.
- [#540](https://github.com/yantrikos/yantrik-os/issues/540): what a program running as you can
  reach without an agent token.

No date is promised for any of them. Until they land, treat the built-in companion as able to run
a command as you and look at your screen, and do not give it a model you would not trust with that.

## Known limits today

These are open and public. A report that one of them is worse than described is in scope. The
full list is the issues labelled
[`security`](https://github.com/yantrikos/yantrik-os/labels/security).

- **A program that runs as you is held by the gate only where it goes through `yos` or
  `yos-mcp`.** Hermes runs under its own account (`yantrik-mind`, #1076). Pi, OpenClaw and
  DeepSeek still run as your account, and so do the jobs a Mind starts; there is no sandbox around
  them. Giving jobs an account of their own is #1137, step 3.
- **#1137:** no browser has a DevTools port any more (#1151, #1200), but a job that runs as you
  can still read your browser's profile files, or start a browser of its own. Giving jobs an
  account of their own is step 3 of this issue.
- **#1027:** apps the shell opens in Mind View for a Mind are classified as you.
- **#548:** your own dotfiles start the session, so the boot lock is not a barrier to a process
  running as you.
- **#542:** some companion tools read credential files, or change state while graded Safe.
- **#541:** the shell can be read by a program running as you, where the kernel allows it (its memory holds the open vault's
  key and your conversation). It is not yet made non-dumpable, because the services identify it
  by a path that this would hide.
- **#540:** a program running as you that shows no agent token is taken for you. It can write in
  your chat and read the conversation back.
- **#1022:** the built-in companion's own mail account keeps its password in `config.yaml` in plain
  text, and it has its own mail client. The Email app's passwords are in the credential vault.
- **#885:** recipes and the cognitive kernel give tool output to a model without the injection
  check.
- **#1207:** `~/.config/yantrik/free-ai.json`, which records the free tiers you switched off, is
  read once, and a program running as you can rewrite it. Removing an AI account in Settings does
  not take it off a harness that was wired to it.
- **#1262:** the audit log can stop writing when the memory queue fills, and it did not recover
  until the shell restarted. Tool calls in that time have no row.
- **The ISO is not signed.** The update manifest is (ed25519); the first image you download is
  checked only against the sha256 the same host publishes.

## Known limits of the workshop seal

A Mind's workshop (`/var/lib/yantrik/workshops/<uid>/<mind>/<name>`, granted on a card) is sealed
on the program. A sealed job sees only its workshop and has no network. Unsealed jobs, Mind tabs,
browsers and Mind View apps see no workshop. Bytes leave only by `workshop_present` and the carded
export. A program a Mind chooses starts only through three doors (the outside view, the browser
view, or the shell's `open_app`), and `xtask check-architecture` fails the build on any other
spawn in the companion's tools. **The DevTools route (#1137, #1200) is closed.** No browser has a DevTools port any more: the person's
Browser and Mind View's run on a pipe held by the browser service, and Arcade's verifier on a pipe
only Arcade holds, so a job on the loopback has no DevTools endpoint to drive. The companion's own
headless browser is gone with the port. These browsers still start in a view with no workshops.

An unsealed job runs as your account, so it can change your files, including what your
session runs at login, and under a session rule it needs no card to do so. Giving jobs an account
of their own (#1137, step 3) comes after launch. Until then, these are known and open:

- **Browsers you start yourself can start unwrapped (fixed after launch).** `xdg-open` from Lens
  and For You, the onboarding and free-AI browser (`open_apart`) and a `.desktop` launch from Lens
  or chat can start a Chromium with no view when no Browser is running. A Mind cannot trigger
  these. They carry no DevTools port, but a Chromium started this way is the one your next dock
  launch is handed to. The Browser you open from the dock is the wrapped one; open it first.
- **The startup-file route (#1137, step 3).** A job can append to `~/.bashrc`, `~/.bash_profile`,
  `~/.config/labwc/*`, `~/.config/systemd/user`, `~/.local/share/applications`, `mimeapps.list` or
  a browser profile. What those run later runs in no view. The acceptance for the fix is that
  each of these fails with EACCES from a job. The same route has variants that run seconds later,
  on a Mind's own call and not at your next login: a git `filter` or `textconv` in a repository's
  `.git/config` or in `~/.gitconfig` (the companion's `git_status` and `git_diff` read them; a
  `filter.<x>.clean` runs only with a matching `.gitattributes` or `core.attributesFile`, which
  is named beside it), an ssh `ProxyCommand` in `~/.ssh/config` (`ssh_check_host`), and a
  harness's config that switches on its own unwalled shell at its next start
  (`~/.config/yantrik/pi.json`, `~/.hermes/config.yaml`, `~/.config/yantrik/openclaw.json`).
  They close with the same fix. A wheel that `install_deps` fetches is installed with `--target`
  into the scripts folder's own `.deps`, which only the script sees through `PYTHONPATH` and no
  host Python reads as a site directory, so a `.pth` it ships never runs.
- **`llm.backend: claude-cli`** (not the default; no Settings control offers it) runs `claude -p`
  in the host namespace, with Claude's own permission gate in front. It is fixed after launch.
- **The loopback listeners a job reaches:** the egress ports 7450 and 7451, the gateway on 7460
  (with a token), the local model on 11434, and anything else you run
  on the machine.
- **No sound in `workshop_present`.** The player runs with no audio path until a safer one exists.
- **Programs already running when the workshops folder was made** do not have it hidden. No
  workshop is granted until you log out and back in after the folder is made (an update that makes it starts a new session) or first-boot setup
  asks for a restart.

## Supported versions

Only the latest nightly is supported. Fixes ship in the next nightly.
