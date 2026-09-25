# Wrap an app you did not write

Most programs a person uses were not written for this desktop, and most of them never will be. You
can still put one where a mind can find it, if it can be driven from code at all — a scripting API,
a plugin host, an automation protocol. You write the surface; the program does the work.

There are two shapes, and the program decides which:

- **Inside it.** The program runs your code in its own process — a Python add-on, a plugin. The
  surface lives there, reads the program's state directly, and lives and dies with it. Blender.
- **Beside it.** The program can be driven from another process — over a bridge, a socket, a
  command line. The surface is a separate **adapter** process that the desktop starts next to the
  program. LibreOffice, over UNO.

Either way the surface is built with the same SDK as any other, and a caller cannot tell it from a
program that hosts its own.

## Blender, from inside

Blender runs Python, and anything it runs can import the SDK. The desktop starts Blender with one
extra argument, a script that loads the add-on:

<!-- from: apps/desktop-files/yantrik-blender.desktop -->
```ini
[Desktop Entry]
Type=Application
Name=Blender
Comment=3D modelling, animation and rendering
# The addon is the point of this exact command: it is what makes the Blender this desktop
# opens answer to `yos describe blender`. The dock route runs the same two pieces, and
# availability() checks both before offering the tile.
Exec=blender --python /opt/yantrik/share/blender/bootstrap.py
# TryExec names the program this entry is for — the standard "list it only if installed" key.
# A machine without Blender shows no tile, and `describe shell` leaves `blender` out of its
# apps instead of offering a name `open_app` then refuses (#214).
TryExec=blender
```

The script finds the add-on beside it — in the source tree or in a release — and starts it:

<!-- from: apps/blender/bootstrap.py -->
```python
_HERE = os.path.dirname(os.path.abspath(__file__))
for _candidate in (
    os.path.join(_HERE, "addon"),  # the source tree: apps/blender/addon/yantrik_blender
    _HERE,                         # a release: /opt/yantrik/share/blender/yantrik_blender
):
    if os.path.isdir(os.path.join(_candidate, "yantrik_blender")):
        if _candidate not in sys.path:
            sys.path.insert(0, _candidate)
        break

from yantrik_blender import start  # noqa: E402

start()
```

**Carry the SDK with you.** A release copies `yantrik_surface/` next to the add-on, because a
blender.org build brings its own Python that sees no system packages. The SDK is one directory of
standard-library Python for exactly this reason.

**Get onto the program's thread.** Blender's scene may only be touched from Blender's main thread,
and the socket answers on threads of its own. So the add-on subclasses the SDK's `Surface` and
overrides the one method that says where a dispatch runs, handing each turn to a queue that
Blender's main thread drains between its own event-loop turns:

<!-- from: apps/blender/addon/yantrik_blender/surface.py -->
```python
    def run_on_app_thread(self, fn, timeout):
        """Blender's main thread, reached through the bridge; a thread that does not turn up
        in time is the app not answering."""
        try:
            return self.bridge.submit(fn, timeout=timeout)
        except BridgeTimeout:
            raise NotAnswered() from None
```

<!-- from: apps/blender/addon/yantrik_blender/__init__.py -->
```python
        def pump_timer():
            if self.stopped:
                return None
            self.bridge.pump()
            return 0.05

        self.bpy.app.timers.register(pump_timer, first_interval=0.05, persistent=True)
```

The revision check, the handler and the view read after it cross as one job, so nothing can move
the scene between "is this the revision you read" and "here is what your action did".

**Say what only the program knows.** The dispatch, the gate and the wire are the SDK's; what the
add-on adds is Blender's vocabulary — each action, its grade, and how long Blender's main thread may
take over it (a render is allowed half an hour):

<!-- from: apps/blender/addon/yantrik_blender/surface.py -->
```python
    _action("render",
            "Render the scene to a PNG and report the path, the seconds it took and the "
            "size of the file. With Cycles on a big scene this can take minutes.",
            "sensitive", [
                Param("output", description="where to write the PNG"),
            ], timeout=1800.0),
```

**Test against a fake of the program.** `tests/blender-core` drives the add-on against a fake `bpy`
that models what the add-on observes, so the whole surface is tested with no Blender and no
display; a headless run against a real Blender is recorded separately.

## LibreOffice, from beside it

LibreOffice can host Python too, but only as an extension installed into each person's profile,
and a surface inside it would die with it. It can also be driven from another process over UNO,
its remote API. So [`adapters/libreoffice`](../../adapters/libreoffice/README.md) is an adapter: a
separate program, built only on the public Python SDK, that drives LibreOffice over a UNO pipe.

**Declare the adapter.** The `.desktop` entry's `Exec` starts LibreOffice listening on the pipe;
`X-Yantrik-Adapter` names the program the shell starts beside it; `TryExec` names the program the
entry is *for* — LibreOffice itself, not the wrapper — so a machine without LibreOffice lists it
nowhere:

<!-- from: adapters/libreoffice/yantrik-libreoffice.desktop -->
```ini
[Desktop Entry]
Type=Application
Name=LibreOffice
GenericName=Office Suite
Comment=Documents, spreadsheets and presentations in LibreOffice
# LibreOffice, listening on its UNO pipe so the adapter below can reach it (bin/yantrik-libreoffice).
# Found where the shell looks for programs: /opt/yantrik/bin, or anywhere on PATH.
Exec=yantrik-libreoffice %U
# TryExec names the program this entry is for — LibreOffice itself, not the wrapper the Exec
# runs, which ships with this adapter and is therefore always installed. A machine without
# `soffice` lists this entry nowhere: no tile, and no `libreoffice` row for a mind to try
# `open_app` on (#214).
TryExec=soffice
# The office document types LibreOffice opens, as the freedesktop MimeType key states them.
# The shell's own table has no app for these, so this line is what puts LibreOffice in Files'
# "Open with" list for an .odt and what makes it the answer at double-click time — and it is
# what an "Always use this app" choice for one of these types points back at (#233).
MimeType=application/vnd.oasis.opendocument.text;application/vnd.oasis.opendocument.spreadsheet;application/vnd.oasis.opendocument.presentation;application/msword;application/vnd.ms-excel;application/vnd.ms-powerpoint;application/vnd.openxmlformats-officedocument.wordprocessingml.document;application/vnd.openxmlformats-officedocument.spreadsheetml.sheet;application/vnd.openxmlformats-officedocument.presentationml.presentation;application/rtf;
Icon=libreoffice-startcenter
Terminal=false
Categories=Office;
Keywords=document;spreadsheet;writer;calc;odt;docx;ods;xlsx;pdf;
# The surface, and the separate process that provides it: LibreOffice cannot host one itself, so
# the shell starts the adapter beside it with YANTRIK_SURFACE and YANTRIK_APP_PID, and stops it
# when LibreOffice exits (docs/sdk/found-while-closed.md). The purpose is written against the
# desktop's own yDoc, which keeps Markdown documents: this is for office files.
X-Yantrik-Surface=libreoffice
X-Yantrik-Purpose=office files — .odt, .docx, .ods, .xlsx, .csv — opened, read, written and saved in LibreOffice, and exported to PDF
X-Yantrik-Aliases=writer;calc
X-Yantrik-Adapter=yantrik-libreoffice-adapter
```

The shell starts the adapter with `YANTRIK_SURFACE` (the id to bind) and `YANTRIK_APP_PID` (the
process it serves), and sends it SIGTERM when that process exits. An adapter should also notice on
its own that the program has gone — a program can be closed from somewhere the shell does not
see — and this one stops once the process has gone and nothing answers on the pipe.

**Keep the program's API in one file.** `office.py` is the only file that imports `uno`, and it
turns every UNO failure into a sentence. The connection is made when first needed, and made again
after LibreOffice quits and comes back; a LibreOffice that is not there is a sentence too:

<!-- from: adapters/libreoffice/yantrik_libreoffice/office.py -->
```python
        try:
            context = resolver.resolve(self.connect_string())
        except Exception as e:  # noqa: BLE001 - every UNO failure becomes a sentence
            if uno_error_name(e) in ("NoConnectException", "ConnectionSetupException"):
                raise NotReachable(
                    "LibreOffice is not running, or not listening on the pipe `%s` — open it from "
                    "the desktop, which starts it listening, or start it with "
                    "--accept=\"pipe,name=%s;urp;\"" % (self.pipe, self.pipe),
                    "not running") from None
            raise NotReachable("LibreOffice would not connect: %s" % uno_message(e),
                               "would not connect") from None
        self._desktop = context.ServiceManager.createInstanceWithContext(
            "com.sun.star.frame.Desktop", context)
```

**Read the view once.** Summary and state come from one read of LibreOffice, by overriding
`snapshot`; two reads could straddle a change and publish a revision of a view that never existed.
When LibreOffice is not there, the surface still answers, and says so:

<!-- from: adapters/libreoffice/yantrik_libreoffice/surface.py -->
```python
    def snapshot(self):
        try:
            documents, front = self.office.overview()
        except NotReachable as e:
            return ("LibreOffice — %s" % e.short,
                    {"connected": False, "pipe": self.office.pipe, "problem": str(e),
                     "documents": []})
```

**Do the slow part outside the turn.** An export is checked in the surface's turn — the document
exists, the path is new — and written after it, with `Later`, so a long export holds up nobody who
is only reading ([Designing a describe](describe.md#settles-later-or-answered-later)).

**Grade what the program can do to the person, not what it is.** Reading is `safe`; writing into a
document is `standard`, because nothing reaches the disk until `save`; `save` is `sensitive`
because it replaces a file; `save_as` and `export_pdf` stay `standard` by refusing to replace
anything. The adapter's README argues each grade; [Choosing a grade](grades.md) generalises them.

**Test against a fake of the API.** `tests/fake_uno.py` stands in for the slice of UNO the adapter
uses — the resolver, the Desktop, text, sheets, cells, `store`, a LibreOffice that quits — and the
tests drive the real adapter code and the real SDK dispatch through it, then serve it on a socket
and hold it to the protocol with `yos check`. `tests/test_live.py` runs the same path against a
real LibreOffice, headless, where one is installed.

## A checklist for your program

1. **Find the API.** Scripting host, plugin system, D-Bus, a remote-control socket, a command line
   with machine-readable output. No API means no surface — the accessibility tree is a different,
   weaker door.
2. **Inside or beside?** Inside when the program runs your code and you can ship it with the
   program; beside when you can only reach it from outside, or should not modify it.
3. **Whose thread?** If the program's state belongs to one thread, override `run_on_app_thread`.
4. **One read per view**: override `snapshot` when summary and state come from the same place.
5. **Name things the way the actions take them**, and describe what the program is showing, not
   what your adapter is doing.
6. **Say when the program is not there.** A surface that answers "not running" is more use than one
   that times out.
7. **Grade every action**, and refuse the arguments that would make one worse than its grade.
8. **Fake the API in tests**, run `yos check`, and ship a `.desktop` file with the keys
   ([Being found while closed](found-while-closed.md)).
