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
spawn in the companion's tools. An unsealed job runs as your account, so it can change your files, including what your
session runs at login, and under a session rule it needs no card to do so. Giving jobs an account
of their own (#1137, step 3) comes after launch. Until then, these are known and open:

- **The DevTools route (#1137).** A job shares the host's loopback, so it can drive Mind View's
  browser on 9223. That browser, and the companion's own headless browser and Arcade's verifier, start in a view with no
  workshops, but a job can still drive them.
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
  (with a token), the local model on 11434, Mind View's browser on 9223, and anything else you run
  on the machine.
- **No sound in `workshop_present`.** The player runs with no audio path until a safer one exists.
- **Programs already running when the workshops folder was made** do not have it hidden. No
  workshop is granted until you log out and back in after the folder is made (an update that makes it starts a new session) or first-boot setup
  asks for a restart.

## Supported versions

Only the latest nightly is supported. Fixes ship in the next nightly.
