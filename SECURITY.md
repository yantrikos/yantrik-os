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
`deploy/yantrik-os/config-default.yaml`). Besides that ceiling, the tool loop applies taint, reach,
policy denials and each tool's own checks. The only thing that can stop a tool is a refusal;
nothing asks you. The "ask" answers of the policy engine are logged, and the tool still runs. Its
mail still asks on a card (`ask_before_sending`), except in full bypass, and `app_action` goes through the shell's gate.

- **Restricting it.** Edit `tools.max_permission` in `/opt/yantrik/config.yaml`, then restart the session. No Settings
  control sets that key today. The Settings control "Tool permission" writes a different key,
  `tool_permission`, which is the ceiling for Minds and harnesses. It reaches the companion only
  through `app_action`; its other tools do not read it.
- **Where the companion's calls are recorded.** Its tool calls become `audit/tools` memories in
  YantrikDB. They carry a seven-day half-life (they fade from recall), keep at most 200 characters of each
  result, and are not written in incognito.
  `~/.local/share/yantrik/mind-audit.jsonl`, and "See what it did without asking" in the mode
  menu, cover Minds and harnesses only. The shell commands and screen reads this section warns
  about are not in that file.
- **Files and commands.** The file tools refuse protected places such as `.ssh`. Some other tools
  read around that: `read_terminal_buffer` and `search_terminal_history` read `~/.bash_history`,
  `save_workspace` does too, and `ssh_check_host` reads `~/.ssh/config`
  ([#542](https://github.com/yantrikos/yantrik-os/issues/542),
  [#1247](https://github.com/yantrikos/yantrik-os/issues/1247)). `run_background` and `run_command` run only
  an allowlist of programs, and `code_execute` and
  `script_run` run only in a sandbox (`sandbox: "none"` is refused for the companion). A screen read is not
  held to any list.

A Mind or a harness (Hermes, Pi, OpenClaw, DeepSeek) is different, but not in every call. Its
shell commands are asked on a card. Its `read_screen` and `screenshot_screen` are asked on a card
every time, except in full bypass. Its `os_screen` (the text of every window, accessibility trees, the browser's links),
`os_perception`, `read_mind_view` and `screenshot_mind_view` run with no card. Later calls can
also run with no card:

- **Allow for this session** answers every later command until the shell restarts. A standing
  rule is stored and outlives restarts.
- The `bypass_all` mode asks for almost nothing. It still asks for the three actions that always
  ask, and for the commands, hand-offs and new web addresses of a conversation that has read your
  private data.

This is a known gap, and it is open. These issues track closing it:

- [#1247](https://github.com/yantrikos/yantrik-os/issues/1247) and
  [#1248](https://github.com/yantrikos/yantrik-os/pull/1248): a credential copied outside its
  folder can be read; #1248 refuses secret file names everywhere. It is not merged.
- [#1265](https://github.com/yantrikos/yantrik-os/issues/1265): a session rule or `bypass_all`
  runs commands with no card for the call; they see only the visible folders and a named
  toolchain list; `agent_run_reach` asks for one more place, every time. A Mind's Terminal tab
  cannot ask for one yet.
- [#1266](https://github.com/yantrikos/yantrik-os/issues/1266): a MCP server you add is called
  with no card per call, and a raised ceiling adds Dangerous tools such as `docker_exec`.
- [#1267](https://github.com/yantrikos/yantrik-os/issues/1267): what a Mind is shown in Mind
  View and in `describe`.
- [#542](https://github.com/yantrikos/yantrik-os/issues/542): some companion tools read
  credential files or change state while graded Safe.
- [#540](https://github.com/yantrikos/yantrik-os/issues/540): what a program running as you can
  reach without an agent token.
- [#1275](https://github.com/yantrikos/yantrik-os/issues/1275): what you consent to when you
  share with a provider says less than what is sent. See "Known limits today" below.

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
- **#1275, what is sent to a provider:** when sharing is allowed for an account, every companion
  call goes to that provider with its private context: your conversation, what your minds remember
  about you (including notification text, file names and apps opened that the shell wrote by
  itself), your name and the local time, on every turn the computer's state (network name,
  running apps, battery, CPU, memory, disk, idle time), and the results of every tool the model
  uses (files read, the clipboard, the text of other apps' windows, browser pages, screenshots
  described by the vision model, command output). It applies to every mind on Yantrik models. The
  words on the consent screens are narrower than this.
- **#1250:** Hermes copies its gateway token into config backups and request dumps.
- **#1244:** once a place is saved, the weather service sends the machine's coordinates to
  `api.open-meteo.com` at every start, with nobody asking.
- **#1228 and #1243:** once Hermes is installed, its gateway contacts hosts of Nous Research and
  OpenRouter. `clawhub.ai` was seen contacted at boot on one test VM, from no Yantrik code; the
  source is not yet known. Nobody asked for either.
- **#1157:** the model runtime and the companion run in the process that draws the approval
  cards.
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
only Arcade holds, so a job on the loopback has no DevTools endpoint to drive. The port-driven headless
browser for `web_search` and `browse` is gone. `launch_browser` still accepts `headless: true` and
starts a headless Chromium, with no DevTools switch. These browsers still start in a view with no workshops.

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

## What the AI can read with nobody asked: protected files and the companion

A protected file is a credential, a key, a browser profile, a harness's home, a shell's history, a
repository's object store, or a copy of one under another name (`tokens.json.bak`, `.hermes
(copy)`, an editor's swap file). Pull request #1248 (issue #1247) makes every tool that reads files,
lists them, names them in a window title or runs a program on the AI's behalf refuse them, in one
rule (`crates/yantrik-ipc-contracts/src/home_paths`). That rule has limits. They are listed here in
plain words, so you know what is and is not covered.

**What the companion may do with nobody asked.** The companion is the assistant that lives in the
desktop. It has no approval card of its own. A tool's grade only decides whether the tool runs at
all: Safe, Standard and Sensitive tools run with nobody asked, up to the ceiling set in
`config-default.yaml` (`default_max_permission`), which ships at **sensitive**. So at the shipped
ceiling the companion's Safe, Standard and Sensitive tools run without a card, on every call. Only
Dangerous tools (such as `docker_exec`) are refused at that ceiling. A Mind or an agent harness
is different: its shell actions raise a card according to the mode you chose. Where this matters for
protected files, the companion's tools that could reach one are closed one by one.

What the fix for #1247 (pull request #1248) promises, in three sentences:

1. **Yantrik's own tools** (the file tools, `run_command` and `run_background`, the disk and git tools,
   the terminal tools, the window and process lists, `describe shell` for a Mind, perception,
   `save_workspace`, `list_fixes` and the screen tools) do not return a protected file's contents or
   name, for the places and name shapes the rule lists.
2. **Other programs' output is not filtered.** A server you added through MCP is called with its path
   arguments checked (a protected path in an argument is refused), and what it answers is passed
   through as it is: **a listing or a file returned by an MCP server is not filtered.** That is a known
   limit, closed by #1266.
3. **The list of name shapes is closed.** A copy under a name outside the listed shapes is not
   protected. For example, `~/.kube-copy-of-work/config` (a backup word the list does not have) and
   `~/.kube-keep/config` (another word) are readable.

Closed: `run_background` runs only what `run_command` may run, and its output is filtered before it is
cut to the last lines, as `run_command`'s is; scripts run only inside the sandbox; `claude_code` is
refused for the companion (it would run Claude Code as you, with Bash and Read, and nothing would ask
you first); `claude_think` starts Claude Code with no tools, no MCP servers and `Read` denied, so an
`@file` in its prompt sends nothing; a read of the whole screen, or a picture of it, asks you every
time, except in bypass-all, where both `read_screen` and `screenshot_screen` run unasked (no picture or
text goes to a model). Anything not listed as closed is open.

Two more name edges are known and left: `.claude.json-old` is readable while `.claude.json.bak` is
not, and a `.claude` folder directly under another account's home that is not `/home/<user>`, `/root`
or `$HOME` (such as `/var/lib/hermes/.claude/history.jsonl`) is read as a project's and is readable;
reading it needs your account to read the other account's home.

**Known and open, each with an issue:**

- **Agent shells under a session rule (#1265).** `agent_run` (a Mind's, and every harness's
  `run_command`) and a Mind's Terminal tab can run any command. The first command shows a card. If
  you answer "Allow for this session", or set a standing rule, later commands run with no card, and
  in bypass-all nothing asks at all. What bounds them is the view, not the command's words: a
  session rule for commands reaches your visible folders (not `~/snap`, not a link) and a named
  read-only toolchain list (`.local/bin`, `.cargo/bin`, `.cargo/registry`, `.cargo/git`, `.rustup`,
  git's `ignore` and `attributes`), with any file or folder in those named like a secret masked.
  That masking is a deny list over names: **a token inside a plain-named script (in `.local/bin` or
  `.cargo/bin`, say) is not caught**. A toolchain entry with a link at it or on the way to it, or
  one too big (50,000 things) or too slow (200 ms) to look through, is left out whole, so a build
  that needs it fails with "No such file" until the Mind asks for it with `reach`; a registry's
  unpacked sources, git checkouts and a toolchain's docs are looked at only at their top.
  Git's own settings are never bound: the view is given a short list of the keys that may be shown
  (name, email, default branch, editor, colours), and no `alias`, `credential`, `http`, `url` or
  `include` setting (an alias can carry a credential, and every value shown is a word of the
  command line that any local account can read). The environment is scrubbed of `GIT_CONFIG_*`, the
  other `GIT_*` variables and `XDG_CONFIG_HOME` before they are set, so a variable the shell was
  started with does not point git at a file on the host. Still read by git in the view:
  `/etc/gitconfig` (the host's, with the whole `/`), and a repository's own `.git/config`, which is
  in your visible folder by design. Your settings, keys, app data and startup files are not there,
  and a command's `/tmp` is private to the job: the host's is never bound, and a working directory
  is the home, or a folder inside a visible folder of it, or one inside that private `/tmp`. With no
  `bubblewrap` a command is refused, and the home to protect is the account's: `outside_command`
  compares the home it is given with the account database's and refuses on a mismatch, so no door
  can start a view over `$HOME` by mistake. `Pictures` and `Videos` are put in the view child by
  child, without `Screenshots` and `Recordings`, so nothing is made in your folders and a new file
  made directly in them by a command is kept in the view only. A link at one of those places stops
  the command, with a sentence naming it.
  **A command can change files in the visible folders, including a project's own settings and
  hooks (`.git/hooks`, `.envrc`, editor tasks), which run later as you.** Hiding those nested
  places is tracked in its own issue. A command that needs a hidden place (`git push` over SSH,
  `gh`, `npm publish`) fails with "No such file" unless the Mind runs it with `agent_run_reach`,
  which names the places and is asked on a card every time, in every mode including full bypass,
  and no rule answers it. The card shows each place as it resolves (a link is refused, with the
  place it leads to), and says that the command can copy or send what it reads; Allow makes those
  places visible read-only to that one command. A place inside a visible folder cannot be named: it
  is already visible, or protected and never shown. Each place is opened without following a link
  and bound from that open descriptor (`--ro-bind-fd`), so it is the place that was checked
  whatever happens to its path afterwards; what remains is that the descriptors are held by the
  shell until the command starts and by `bwrap` until it has bound them, and that the folders above
  the place are the person's, not a Mind's (no view can write above a hidden place). The visible
  folders and the toolchain entries are looked at again as the last thing before the command is
  built. Still shared with the host in the view: `/dev` (a camera and microphone among them),
  `/dev/shm`, `/var/tmp`, the system `/run` and the network namespace, tracked in #1351. A Mind's
  Terminal tab has no `reach` door yet. The Mind's `read_screen` is not a file read and is not
  covered here.
- **Docker, MCP servers and the companion's ceiling (#1266).** If you raise the companion's ceiling
  to `dangerous`, it runs every Dangerous tool, `docker_exec` included, with no card. The companion's
  MCP proxy tools are Standard and raise no card either: a path argument is checked, but **what a server
  answers is not filtered**: a listing or a file it returns passes through as it is, and a server you
  added can return a protected file under an argument that is not a path. A card for each Dangerous or MCP call by the companion is not built yet. Bringing `claude_code` back
  needs a card on every call, which is this issue's work and the owner's decision. A person's own
  plugin that takes a parameter lets a newline through to the command it runs; this issue covers that too.
- **Apps and free text in Mind View (#1267).** What an app shows a Mind in its own description (a
  spreadsheet's sheet, an editor's title and path, an image viewer's sibling files, download and
  music lists, notes, snippets, presentations, email store paths, a terminal's recent output, a
  recipe's steps) can name a protected file. The shell's own window, dock, launcher and notification
  fields are covered; these app fields are not yet.

## Supported versions

Only the latest nightly is supported. Fixes ship in the next nightly.
