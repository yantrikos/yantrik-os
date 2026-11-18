# LibreOffice on the Yantrik desktop

An adapter that puts LibreOffice where a mind can find it: `app-libreoffice.sock`, answering
`app.describe` and `app.act` like every app this OS ships. A mind opens an office file, reads a
Writer document's text or a Calc sheet's cells, writes them, saves, and exports a PDF — under the
person's grades, with `yos act libreoffice …`, the MCP bridge, or anything else that speaks the
[surface protocol](../../docs/surface-protocol.md).

It is built only on the public Python SDK ([`sdk/python/yantrik_surface`](../../sdk/python/README.md)),
which is the point of it: it is the second worked example in
[Wrap an app you did not write](../../docs/sdk/wrap-an-app.md), after Blender.

| file | |
| --- | --- |
| `yantrik_libreoffice/office.py` | LibreOffice over UNO — the only file that touches `uno` |
| `yantrik_libreoffice/surface.py` | the actions, their grades and descriptions, and what describe shows |
| `yantrik_libreoffice/adapter.py` | the program: how it starts, what it serves, when it stops |
| `yantrik_libreoffice/agent_office.py` | the agents' own LibreOffice, started in its sandbox, which serves every agent call |
| `yantrik_libreoffice/sandbox.py`, `fence.py`, `canaries.py`, `places.py` | the sandbox (bwrap), the launcher inside it (Landlock and the proof), the canaries it must not reach, and the person's places |
| `yantrik_libreoffice/workspace.py`, `copies.py`, `workdir.py` | an agent's actions on copies of the person's files, the only way bytes get back, and the only way into the sandbox's work folder |
| `yantrik_libreoffice/unopipe.py` | the agents' UNO pipe, pinned where the sandbox cannot reach and checked before every connection |
| `yantrik_libreoffice/store.py`, `guard.py` | `save`, `save_as`, `export_pdf`, and the scan they and the reads share |
| `bin/yantrik-libreoffice-adapter` | the command `X-Yantrik-Adapter` names |
| `bin/yantrik-libreoffice` | the command `Exec` names: LibreOffice, listening on its UNO pipe |
| `yantrik-libreoffice.desktop` | how the shell finds it while it is closed |
| `tests/` | the adapter against a fake UNO (`fake_uno.py`), the program and the launcher, `yos check`, the launcher and the sandbox against the kernel (`test_fence.py`, `test_sandbox.py`), the copies (`test_workspace.py`, `test_agents_sandboxed.py`), and `test_live.py` and `test_live_sandbox.py` against a real LibreOffice |

## What a mind sees

```text
$ yos describe libreoffice
LibreOffice — report.odt (Writer, unsaved changes); budget.ods (Calc); budget.ods in front
```

The state lists every open document by the **name** the actions take as `document` (its file
name, or `Untitled 1` for one never saved), its kind, path, whether it has unsaved changes or is
read-only, and for a spreadsheet its sheets and the range each one uses. Each document also carries
`content`, a fingerprint of its text or cells, so the revision moves when a cell does and an
`expect_revision` guard catches an edit made in between — not only when the unsaved-changes flag
flips. When LibreOffice is not there, the summary says so (`LibreOffice — not running`) and the
state says why.

## The actions, and why each is graded as it is

| action | grade | why |
| --- | --- | --- |
| `read_text` | safe | Reading changes nothing. |
| `read_cells` | safe | Reading changes nothing. |
| `open` | standard | A window opens; no file changes, and the documents already open are left alone. |
| `write_text` | standard | The document in memory changes and nothing on disk does until `save`; LibreOffice's Undo takes it back. |
| `write_cells` | standard | As `write_text`. Every cell is checked before any is written, so a bad one leaves the sheet as it was. |
| `save_as` | standard | Only ever creates a file: a path where something already is is refused. |
| `export_pdf` | standard | Only ever creates a file, and leaves the document as it is. |
| `close` | standard | Refused while the document has unsaved changes, so it never throws edits away. |
| `save` | **sensitive** | It replaces the file the document came from. |

`save` is the decision that matters. With writing graded `standard`, a mind in `ask` mode can open
any document and change it without a card, which is right while the change lives only in
LibreOffice's memory — closing without saving undoes all of it. `save` is the step where a mind's
edits become the file, and the only step that destroys anything (the old contents), so it is the
one step where the person is asked. In `auto` mode it runs unasked, which is what `auto` means.

None of the descriptions says the action "cannot be undone", on purpose. Those words make the
dispatch ask about an action in every mode but full bypass (`bypass_all`) (docs/surface-protocol.md §7), and a
person who chose `auto` has said `sensitive` work may run unasked. The save description says
exactly what happens — the old contents are replaced and LibreOffice keeps no copy — and the grade
does the asking. `tests/test_surface.py` holds both decisions.

`save_as` and `export_pdf` stay `standard` because they refuse to replace a file. A version that
could overwrite would have to be `sensitive` for every call, since a grade is per action and not
per argument; refusing the one argument that would make it dangerous keeps the common case
unasked.

## Agents work on copies, in a sandboxed LibreOffice of their own (#953)

A document can read files and reach sockets: an external reference, `INDIRECT`, `WEBSERVICE`, a
linked section, a macro, an import filter. Whatever LibreOffice reads or reaches, it does with the
rights of its process, and the person's LibreOffice runs with all of theirs. So **no agent call
reaches the person's LibreOffice** (#942). An agent's calls (and the Mind's) go to a second,
headless LibreOffice that the adapter starts on the first agent `open` (`agent_office.py`), in a
sandbox `bwrap` builds (`sandbox.py`):

- **its own mount namespace, on an empty root.** Only `/usr` and a short list of `/etc` are shown,
  read-only, with the few places LibreOffice's own install links to (Debian's
  `/var/spool/libreoffice`), a fresh `/proc` and a minimal `/dev`. Its profile, its temporary
  files and the copies it works on are private folders of the adapter's, and its `/tmp` holds
  nothing but its own UNO pipe. The person's home, `/run` (the session bus, the display, every
  Yantrik socket) and the real `/tmp` (the person's LibreOffice pipe) are not there at all, and
  the root is read-only once built. A protected place inside a folder that is shown is covered
  again with an empty one.
- **its own network namespace**: nothing but loopback, so no TCP, no UDP, and none of the
  session's abstract sockets.
- its own IPC, PID and UTS namespaces, a new session, no capabilities, no user namespaces of its
  own (`--disable-userns`, so bwrap 0.8 or later), an environment with no bus, display or
  runtime folder, and it dies with the adapter.

Inside, `fence.py` runs first. It puts **Landlock** up as a second wall where the kernel offers it,
then **proves the sandbox holds** before any document exists: the home and every protected place
must fail to open, the person's sockets and three canaries the adapter leaves listening outside (a
socket path, an abstract socket, a TCP port on the host) must fail to connect, and no interface but
loopback and no route out may exist. Only the kernel's words for "not there" or "not allowed"
count as failing. Then soffice runs. The paths it probes are not in the sandbox at all, so they
are a net for a later bind that shows one; the abstract and TCP canaries and the network checks
are what test the namespaces. Landlock lets the sandbox make no links anywhere.

**Everything the sandbox can write is hostile** (#962 review H1, H2). A document that takes over
the sandboxed LibreOffice can change its work folder and its `/tmp`, so the adapter, which works
outside with the person's rights, never follows anything there, with or without Landlock:

- the work folder is reached only through `workdir.py`. Each document gets a slot, a folder with
  a random name the adapter made and holds open; a file in it is opened through that handle, never
  followed, and read only when it is a regular file of the adapter's own uid with one name, of a
  size it takes. Renaming the slot or putting a link where a file was leads nowhere outside;
- the UNO pipe is pinned (`unopipe.py`): the socket the sandbox made is hard-linked, as itself and
  never as what a link points to, into a folder the sandbox is not shown, its listener must
  descend from the sandbox's bwrap (SO_PEERCRED), and the name in the real `/tmp` that a UNO
  client connects to is a link to that pinned socket, checked again before every connection. The
  pipe's name is random per sandbox.

**Where the sandbox cannot be built** (no `bwrap`, user namespaces off, LibreOffice installed in
the person's own files) or does not prove, every agent call is refused with "LibreOffice isn't
open to agents yet: its sandbox is still being built (#953). Nothing was run." and the reason.
Nothing is started; there is no unsandboxed way round. `bubblewrap` is in the image's package
list for this.

**Copies, in and out** (`workspace.py`, `copies.py`). The sandboxed LibreOffice never sees a
person's file where it lies. `open` holds the path to the shell's rule (as named and where links
resolve), refuses a file the person has open in their own LibreOffice or with a LibreOffice lock
beside it, and copies it into the sandbox: a regular file with one name, checked to still be at
the path the rule judged. `save` holds the file to the write rule again, refuses when the person
has opened it since or it changed on disk, lets the sandboxed LibreOffice save the copy, and only
then replaces the person's file with it, at once (a temporary file in the same folder, renamed
over it). `save_as` and `export_pdf` write into the sandbox, then create the person's new file,
never replacing one. Answers and describe (`agent_documents`) name the person's paths, never the
sandbox's.

The trade-off: an agent works on files, not on the person's open windows, so it never sees the
person's unsaved changes. A file a document links to beside it (a linked image, say) is not in the
sandbox either.

The scan in `guard.py` stays as a second wall. Before an agent's read, save, `save_as` or export,
the document may be linked to nothing, and a spreadsheet's formulas (array formulas included)
and named expressions must pass the agent formula allowlist. The links are asked again after the
read or write, and a file written while a link appeared is taken back.

Not covered: the sandbox sets no limit on the CPU, memory or disk an agent's LibreOffice uses, and
has no seccomp filter: with no capabilities, no new privileges and no user namespaces of its own,
what is left of the kernel is ordinary system calls, and a filter is its own reviewed change
(#962 review L2). Where the kernel has no Landlock, the namespaces and the two guards above
stand alone, and the launcher says so in its log.

## How it is started, and why that differs from Blender

Blender hosts its surface itself: the desktop starts `blender --python bootstrap.py`, and the
add-on runs inside Blender's own Python, on Blender's main thread, for as long as Blender does.
LibreOffice has no Python of ours running inside it. It could (an extension), but that would mean
installing into each person's LibreOffice profile, and a surface that dies with a LibreOffice
crash. So this is the other shape the protocol provides: a **separate adapter process**,
declared with `X-Yantrik-Adapter`, driving LibreOffice from outside over the UNO remote bridge.

- **The desktop opens it** through `yantrik-libreoffice.desktop`. Its `Exec` is
  `yantrik-libreoffice %U`, which is LibreOffice started listening on a UNO pipe
  (`--accept=pipe,name=yantrik-libreoffice;urp;`) and otherwise untouched. The shell starts
  `yantrik-libreoffice-adapter` beside it with `YANTRIK_SURFACE` and `YANTRIK_APP_PID`, and sends
  it SIGTERM when LibreOffice exits. The adapter also stops on its own once that process has gone
  and nothing answers on the pipe. It is not started while something already answers on
  `app-libreoffice.sock`.
- **By hand**, `yantrik-libreoffice-adapter` serves until stopped, for any LibreOffice started
  with that `--accept`.
- **Headless**, `yantrik-libreoffice-adapter --headless` starts a LibreOffice of its own — no
  window, a private profile, a pipe of its own — and stops it on the way out. That is how
  `tests/test_live.py` runs it, and how a script on a machine with no display would.

Anything running as the person can connect to the pipe and drive LibreOffice, as anything running
as the person can already drive LibreOffice's windows and files: the uid is the boundary, as it is
on the socket bus (docs/surface-protocol.md §3).

Not yet seen on a real desktop: a LibreOffice already running when the shell opens this entry. The
launcher's `--accept` should be handed to the running instance (LibreOffice passes a second launch's
arguments to the first), and that second process exits at once. The adapter keeps serving while the
pipe answers, but whether the shell stops it on that exit depends on the shell's handover detection
(`crates/yantrik-ui/src/wire/dock.rs`).

## Running it

```sh
# the tests: the adapter over a fake UNO, the program, the launcher, yos check
python3 -m unittest discover -s adapters/libreoffice/tests -v

# against a real LibreOffice (needs soffice and python3-uno; skipped without them)
python3 -m unittest discover -s adapters/libreoffice/tests -p test_live.py -v

# the agents' sandbox with a real LibreOffice (needs bwrap and user namespaces as well)
python3 -m unittest discover -s adapters/libreoffice/tests -p test_live_sandbox.py -v

# by hand, headless, then drive it
adapters/libreoffice/bin/yantrik-libreoffice-adapter --headless &
yos act libreoffice open path=~/Documents/budget.ods
yos act libreoffice read_cells range=A1:D10
yos act libreoffice write_cells cells='{"D2": "=B2*C2"}'
yos act libreoffice export_pdf path=~/Documents/budget.pdf
yos check libreoffice
```

The adapter needs a Python that can `import uno` — on Debian, the system `python3` with
`python3-uno` — which is why its script runs `/usr/bin/env python3` and carries the SDK with it
rather than asking for a virtual environment.

## Shipping it

Nothing installs the adapter or LibreOffice today (`packages.list` names neither; it names only `bubblewrap`, the sandbox), so a machine has the adapter only when it is placed by hand, as on 520. Not in `build-release.sh` or the image yet. Shipping it would take:

1. **LibreOffice in the image**: `libreoffice-writer`, `libreoffice-calc` and `python3-uno` (the `-nogui` packages do not pull it in; the 520 test needed it beside them) in the
   package list of `deploy/yantrik-os/build-debian-iso.sh` (a few hundred MB), or leaving it to the
   person to install, in which case the entry must not be listed until `soffice` exists.
2. **The adapter in the release**: `yantrik_libreoffice/`, `bin/` and a copy of
   `sdk/python/yantrik_surface/` under `share/libreoffice/` (the SDK beside the package, as the
   Blender add-on carries it), and `yantrik-libreoffice` and `yantrik-libreoffice-adapter`
   linked into `/opt/yantrik/bin`, where the shell looks for programs.
3. **The entry**: `yantrik-libreoffice.desktop` into `share/applications` with the others
   (`shipped-desktop-files.sh` reads `apps/desktop-files`; the entry would move or be listed there),
   and a check that `Name=LibreOffice` does not collide with Debian's own
   `libreoffice-startcenter.desktop` in the launcher.
4. **A live run on a VM**: `test_live.py` on the image, and the handover case above driven by hand.
