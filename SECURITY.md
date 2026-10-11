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

## Supported versions

Only the latest nightly is supported. Fixes ship in the next nightly.
