# Who sent a notification

Every notification the notifications service stores carries a `sender` record beside the
`app` name the caller gave (#114). This page says what that record establishes, and what it does
not.

## What is recorded

At the moment a call arrives, the service reads the caller's kernel credentials (`SO_PEERCRED`:
pid and uid) and walks `/proc` from that pid to the first program a person would recognise
(`yantrik_ipc_transport::peer_identity`). It stores:

- `claimed`: the `app` the caller gave, verbatim, or nothing.
- `verified`: the recognisable program's command line and pid, or "could not be identified".
- `exe`: that program's `/proc/<pid>/exe`.
- `desktop`: whether the process on the socket itself was the desktop (see below).

The name on the row and the toast is the caller's own, except that only the desktop may use
`Yantrik`. Any other caller whose name starts with "Yantrik" once case is folded, or that spells
it with Cyrillic or Greek lookalike letters, is filed under its verified program's name instead.

## "The desktop"

A caller is the desktop when all of these hold, judged by the service at call time:

1. its uid is the service's own uid, so a mind under an account of its own is not the desktop;
2. the process on the socket itself, not an ancestor, is `yantrik-ui` or `notifications-service`
   (`yantrik_ipc_transport::owner::DESKTOP_BINARIES`);
3. that binary is in `/opt/yantrik/bin`, or in the directory the service itself runs from. The
   service's own directory counts only in a debug build, or when root owns it and neither group
   nor others can write to it.

Only the desktop gets the name `Yantrik` and the notification card's plain line
"Sent by yantrik-ui · verified". Everyone else is shown as "Sent by <program> (verified)". The
real executable is named when it is not what the command line calls itself, a terminal origin
is shown, and any claim follows, cut short.

## What this does NOT hold against

"Verified desktop" does **not** hold against a process running as the person's own user.
`/proc/<pid>/exe` is read when the request is handled, not when the socket was connected. So a
same-user process can connect, send, and then exec `/opt/yantrik/bin/yantrik-ui`, or start the
real binary with `LD_PRELOAD` pointing at code of its own. It does hold against minds under their
own accounts and against anything that is only *named* like the desktop. This is the same limit
#154 records for the shell's control socket.

**Follow-up (not built):** the real fix is a socket, or an inherited file descriptor, that the
service hands the shell when the shell is spawned. Being the desktop then becomes a capability
the shell holds, not a fact read from `/proc` after the call. The TODO is in
`services/notifications-service/src/main.rs` at `is_the_desktop`.
