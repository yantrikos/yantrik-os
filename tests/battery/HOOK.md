# Proposal: a coordinate-free "press +" for test machines (needs security review)

Today the battery presses the Lens's + with the pointer, found by colour, and proves it by the new scope id in `taint-scopes.json`. That works, but it depends on the Lens's look. The OS refuses every programmatic way to begin a taint scope on purpose (`taint_scope::begin_by_person`, `NotAPress`, and a test that `invoke_lens_new_chat` is called in one place), and that must stay true for minds.

**Proposed hook.** On a machine that has `/etc/yantrik/test-machine` (root's, not writable by others, the same file the battery checks), the shell listens on a root-only unix socket `/run/yantrik/test-hook.sock` (mode 0600, owner root, not in the runtime directory the person's account and the minds can reach). One command, `press_plus`, runs the same code path as a pointer press (`is_press()` true, `begin_by_person`).

Why a mind cannot use it:
- minds run as the person or as `yantrik-mind`, never root; the socket is root 0600 and `SO_PEERCRED` uid 0 is checked on every connection;
- the socket is created only when the marker exists at shell start, and the marker is checked again at each command (a root-owned regular file in a root-owned directory, the same rule as `/etc/yantrik/start-open`);
- the hook is compiled out of release builds (`cfg(feature = "test-hook")`), so a person's machine has no code for it at all.

Risk to review: a root process on a test machine can then clear a taint scope. That is the same power as pressing +, held by root, on a machine marked as a test machine. Not implemented in this PR; the runner would try the socket first and fall back to the pointer press.
