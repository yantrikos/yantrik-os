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
export. An unsealed job runs as your account, so it can change your files, including what your
session runs at login, and under a session rule it needs no card to do so. Giving jobs an account
of their own (#1137, step 3) comes after launch. Until then, these are known and open:

- **The DevTools route (#1137).** A job shares the host's loopback, so it can drive Mind View's
  browser on 9223. That browser, and the companion's own headless browser and Arcade's verifier, start in a view with no
  workshops, but a job can still drive them.
- **The startup-file route (#1137, step 3).** A job can append to `~/.bashrc`, `~/.bash_profile`,
  `~/.config/labwc/*`, `~/.config/systemd/user`, `~/.local/share/applications`, `mimeapps.list` or
  a browser profile. What those run later runs in no view. The acceptance for the fix is that
  each of these fails with EACCES from a job.
- **The loopback listeners a job reaches:** the egress ports 7450 and 7451, the gateway on 7460
  (with a token), the local model on 11434, Mind View's browser on 9223, and anything else you run
  on the machine.
- **No sound in `workshop_present`.** The player runs with no audio path until a safer one exists.
- **Programs already running when the workshops folder was made** do not have it hidden. No
  workshop is granted until you log out and back in after the folder is made (an update that makes it starts a new session) or first-boot setup
  asks for a restart.

## Supported versions

Only the latest nightly is supported. Fixes ship in the next nightly.
