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
| `bin/yantrik-libreoffice-adapter` | the command `X-Yantrik-Adapter` names |
| `bin/yantrik-libreoffice` | the command `Exec` names: LibreOffice, listening on its UNO pipe |
| `yantrik-libreoffice.desktop` | how the shell finds it while it is closed |
| `tests/` | the adapter against a fake UNO (`fake_uno.py`), the program and the launcher, `yos check`, and `test_live.py` against a real LibreOffice |

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

Not in `build-release.sh` or the image yet. Shipping it would take:

1. **LibreOffice in the image**: `libreoffice-writer`, `libreoffice-calc` and `python3-uno` in the
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
