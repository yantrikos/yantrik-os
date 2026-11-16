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

Known limits that are documented, not hidden, are listed in the README's "Privacy and security"
section and in the open issues labelled security; a report that one of them is worse than it says
is in scope too.

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
protected files, the companion's tools that could reach one are closed one by one. Closed: `run_background`
runs only what `run_command` may run and shows its output as `run_command` does; scripts run only inside
the sandbox; `claude_code` is refused for the companion (it would run Claude Code as you, with Bash and
Read, and nothing would ask you first) and `claude_think` starts Claude Code with no tools; a read of the
whole screen, or a picture of it, asks you every time, except in bypass-all, where `read_screen` runs
unasked. Anything not listed as closed is open.

**Known and open, each with an issue:**

- **Agent shells under a session rule (#1265).** `agent_run` (a Mind's, and every harness's
  `run_command`) and the Terminal app's `run` can run any command. The first command shows a card.
  If you answer "Allow for this session", or set a standing rule, later commands run with no card,
  and in bypass-all nothing asks at all. Such a command can `cat ~/.ssh/id_rsa` or read a kept
  harness home. A session rule for commands reaches every file you can read. This follows the
  decision that open-ended actions may take a session rule. Whether a command that names a protected
  path must still get its own card is open.
- **Docker, MCP servers and the companion's ceiling (#1266).** If you raise the companion's ceiling
  to `dangerous`, it runs every Dangerous tool, `docker_exec` included, with no card. The companion's
  MCP proxy tools are Standard and raise no card either: a path argument is checked and a listing, a sized listing or a tree is
  filtered by the rule for names in free text, but a server you added can still return a protected file under an argument that is not a
  path. A card for each Dangerous or MCP call by the companion is not built yet. Bringing `claude_code` back
  needs a card on every call, which is this issue's work and the owner's decision. A person's own
  plugin that takes a parameter lets a newline through to the command it runs; this issue covers that too.
- **Apps and free text in Mind View (#1267).** What an app shows a Mind in its own description (a
  spreadsheet's sheet, an editor's title and path, an image viewer's sibling files, download and
  music lists, notes, snippets, presentations, email store paths, a terminal's recent output, a
  recipe's steps) can name a protected file. The shell's own window, dock, launcher and notification
  fields are covered; these app fields are not yet.

## Supported versions

Only the latest nightly is supported. Fixes ship in the next nightly.
