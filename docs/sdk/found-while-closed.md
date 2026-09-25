# Being found while closed

A running surface is found by its socket: `app-<id>.sock` in the session's socket directory. A
closed one is found by its program's `.desktop` file — the file every Debian program already ships
so a launcher can list it — with a few keys added to its `[Desktop Entry]` group. There is no list
in this OS to add your program to; the shell reads the keys, as it reads them for its own apps.

## The keys

The Rust template's entry:

<!-- from: templates/rust-surface/my-surface.desktop -->
```ini
[Desktop Entry]
Type=Application
Name=My Surface
Comment=A to-do list a mind can read, add to, tick off and search
# The program `cargo build --release` makes, installed where the shell looks for programs:
# /opt/yantrik/bin, or anywhere on PATH.
Exec=my-surface
Terminal=false
Categories=Utility;
# The surface this program publishes (docs/sdk/found-while-closed.md). With these keys the
# desktop lists it while it is closed, opens it by any of its names, and routes its
# notifications' buttons to it.
X-Yantrik-Surface=my-surface
X-Yantrik-Purpose=A to-do list: add tasks, tick them off, search them
X-Yantrik-Aliases=my-tasks
```

**`X-Yantrik-Surface`** — required for the rest to count. The id the surface publishes as `app`
and binds as `app-<id>.sock`: lowercase words of letters and digits joined by `-`. A value that is
not one is refused (and logged), not rewritten, and the program is then listed as an ordinary app
with no surface.

**`X-Yantrik-Purpose`** — what the program is *for*, in one line. It is what a mind reads when it
chooses between programs it cannot see, so write it against the one it is most likely to be
confused with: Studio's says it *makes* pictures, Images' that it *views* them; LibreOffice's says
*office files*, beside yDoc's Markdown documents.

**`X-Yantrik-Aliases`** — other names, separated by `;`. The shell links each one at your socket
(`app-<alias>.sock` → `app-<id>.sock`), so `yos describe <alias>` reaches your surface without your
program knowing the name. A name the desktop itself answers to (`shell`, a screen, a section of
Settings) or that another program already holds is not handed out twice; between two programs,
the first in the catalogue's order keeps it.

**`X-Yantrik-Adapter`** — for a program that cannot host a surface itself: a command the shell
starts beside the program when it launches it, with `YANTRIK_SURFACE=<id>` and
`YANTRIK_APP_PID=<pid>` in its environment and the same display environment the program got. The
shell sends it SIGTERM when that process exits, and does not start it while something already
answers on `app-<id>.sock`. The adapter should also exit on its own when the program goes away.

<!-- from: adapters/libreoffice/yantrik-libreoffice.desktop -->
```ini
X-Yantrik-Surface=libreoffice
X-Yantrik-Purpose=office files — .odt, .docx, .ods, .xlsx, .csv — opened, read, written and saved in LibreOffice, and exported to PDF
X-Yantrik-Aliases=writer;calc
X-Yantrik-Adapter=yantrik-libreoffice-adapter
```

A program that hosts its own surface names no adapter — Blender's add-on is loaded by its `Exec`
line itself ([Wrap an app you did not write](wrap-an-app.md)).

## What the shell does with them

With the keys, whether or not the program is running:

- it is listed in `describe shell` → `apps`, with its name, purpose, aliases and whether it is
  running, and `yos ls` shows it as closed with its purpose;
- `open_app` opens it by its id, any alias, or its `Name`;
- a button on one of its notifications reaches it, opening it first if it is closed;
- an approval can be asked for one of its actions while its window is shut.

The shell notices a new or changed entry within a few seconds; `yos act shell refresh_apps` makes
it look at once.

## Where the files go

The shell reads `.desktop` files from `~/.local/share/applications`, `$XDG_DATA_HOME/applications`,
every `applications` directory on `$XDG_DATA_DIRS` (by default `/usr/share/applications` and
`/usr/local/share/applications`), and `/opt/yantrik/share/applications`. It finds the program an
`Exec` or `X-Yantrik-Adapter` names by path, beside the shell, in `/opt/yantrik/bin`, or on `PATH`.

An entry that says `TryExec=<program>` is listed only while that program can be found the same way
— the standard freedesktop rule, and the one an adapter's entry cannot do without: its `Exec` runs
the wrapper that ships with the adapter, which is installed whether or not the app it wraps is, so
`TryExec` is what hides a LibreOffice tile, and the `libreoffice` row of `describe shell`, from a
machine that has no LibreOffice. `yos ls` with no shell to ask, and the bridge's own tool
descriptions, read the same key by the same rule.

For a program of your own:

```sh
sudo install -m755 target/release/my-surface /usr/local/bin/
install -Dm644 my-surface.desktop ~/.local/share/applications/my-surface.desktop
yos act shell refresh_apps
yos ls                       # my-surface, closed, with its purpose
```

The shell reads `Exec` the way the Desktop Entry spec writes it: arguments split on unquoted
whitespace, `"quoted"` sections kept as one argument, backslash escapes honoured, and field codes
such as `%U` answered where they stand — a file being opened goes where the code is, a launch with
no file removes it, and only a line with no code at all gets the file appended last.
`X-Yantrik-Adapter` is still simply split on whitespace: keep it to a program and plain arguments,
and put anything more elaborate in a small script (LibreOffice's `Exec` runs `yantrik-libreoffice`,
a launcher that adds the pipe argument, for this reason).

## Checked by

`samples/tests/desktop_files.rs` reads the templates' entries, LibreOffice's and Blender's with
the shell's own parser, and every `.desktop` file quoted in this guide, and holds them to what this
page says. Each template's own tests fail if its `.desktop` file and its surface disagree about
the id — the mistake a rename makes.
