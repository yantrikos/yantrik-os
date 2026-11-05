# Proposal: a coordinate-free "press +" for test machines (needs security review)

Today the battery presses the Lens's + with the pointer, found by colour, and proves it by the new scope id in `taint-scopes.json`. That works, but it depends on the Lens's look. The OS refuses every programmatic way to begin a taint scope on purpose (`taint_scope::begin_by_person`, `NotAPress`, and a test that `invoke_lens_new_chat` is called in one place), and that must stay true for minds.

**Proposed hook.** On a machine that has `/etc/yantrik/test-machine` (root's, not writable by others, the same file the battery checks), the shell listens on a root-only unix socket `/run/yantrik/test-hook.sock` (mode 0600, owner root, not in the runtime directory the person's account and the minds can reach). One command, `press_plus`, runs the same code path as a pointer press (`is_press()` true, `begin_by_person`).

Why a mind cannot use it:
- minds run as the person or as `yantrik-mind`, never root; the socket is root 0600 and `SO_PEERCRED` uid 0 is checked on every connection;
- the socket is created only when the marker exists at shell start, and the marker is checked again at each command (a root-owned regular file in a root-owned directory, the same rule as `/etc/yantrik/start-open`);
- the hook is compiled out of release builds (`cfg(feature = "test-hook")`), so a person's machine has no code for it at all.

Risk to review: a root process on a test machine can then clear a taint scope. That is the same power as pressing +, held by root, on a machine marked as a test machine. Not implemented in this PR; the runner would try the socket first and fall back to the pointer press.

## Changes after review (#1086)

- **Not `begin_by_person` and not `is_press()`.** The hook calls a separate `begin_by_test_hook`, which records `begun: "test-hook"` in `taint-scopes.json`, so the pointer-press invariant (only a press makes `begun: person`) holds for the whole OS. The leak test would check which kind of scope it ran in.
- **Root is not mind-proof.** A mind can get root through an approved `sudo` card, and its children are root too. `SO_PEERCRED` uid 0 is not enough: also require the peer's cgroup/unit to be the battery's own (a dedicated systemd unit, not a mind's session), and refuse a peer whose parent chain contains a mind's pid.
- **One marker.** The hook uses `/etc/yantrik/no-person-data`, the same file the battery uses; the marker is only a gate, and the real guard is the compiled-out `test-hook` cargo feature, with a CI check that no release image contains the hook's symbol.
- **No time-of-check race.** Open the marker once and `fstat` the descriptor; do not re-check by path at each command.
- **Every use is logged** to the journal with the peer's pid and unit.
