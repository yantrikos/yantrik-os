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

## Known limits today

These are open and public. A report that one of them is worse than described is in scope.

- **A Mind that runs as you is held by the gate only where it goes through `yos` or `yos-mcp`.**
  Hermes and Pi run as your account, and so do the jobs a Mind starts; there is no sandbox around
  them. Running Hermes under the Mind account is in review (#1076).
- **#1137:** such a Mind can drive your signed-in Browser through its DevTools port
  (`127.0.0.1:9222`), with no card. Fix in review (#1151).
- **#1027:** apps the shell opens in Mind View for a Mind are classified as you.
- **#548:** your own dotfiles start the session, so the boot lock is not a barrier to a process
  running as you.
- **#1022:** the built-in companion's own mail account keeps its password in `config.yaml` in plain
  text. The Email app's passwords are in the credential vault.
- **#542:** some companion tools read credential files, or change state while graded Safe.
  **#885:** recipes and the cognitive kernel give tool output to a model without the injection
  check.
- **The ISO is not signed.** The update manifest is (ed25519); the first image you download is
  checked only against the sha256 the same host publishes.

## Supported versions

Only the latest nightly is supported. Fixes ship in the next nightly.
