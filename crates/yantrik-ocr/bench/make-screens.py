#!/usr/bin/env python3
"""Draw the screens of a terminal session as images, with the text each one shows.

usage: make-screens.py <out dir>
Draws off screen (nothing appears on the desktop, and nothing on the desktop is captured):
  a.png  a terminal 40 lines deep
  b.png  the same terminal after one more line of output: everything moved up one line
  c.png  as b, with a command being typed at the prompt
  d.png  exactly c again
  e.png  a different screen altogether
and a <name>.txt beside each with its lines, for checking what was read.
"""
import os
import sys

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk  # noqa: E402

WIDTH, HEIGHT, ROWS = 1280, 800, 40

CSS = b"""
window { background-color: #1d1f21; }
label { color: #d8dee9; font-family: "DejaVu Sans Mono", monospace; font-size: 11pt; }
window#form { background-color: #f4f5f7; }
"""

WORDS = "build cache link test fetch merge index parse write close guard route frame queue shard vault".split()


def output_line(n):
    """A line of build output, different for every n and the same every time for the same n."""
    words = [WORDS[(n * 7 + k * 3) % len(WORDS)] for k in range(4)]
    return f"[{n:04d}] {words[0]} {words[1]}-{words[2]} ok in {(n * 37) % 900 + 12} ms ({words[3]} {n * 13 % 97})"


def terminal(first, typed=""):
    lines = [output_line(n) for n in range(first, first + ROWS - 1)]
    return lines + ["user@yantrik:~/work$ " + typed]


SCREENS = {
    "a": terminal(100),
    "b": terminal(101),
    "c": terminal(101, "cargo test --release"),
    "d": terminal(101, "cargo test --release"),
    "e": [f"Chapter {n}: {' '.join(WORDS[(n + k) % len(WORDS)] for k in range(6))}" for n in range(1, 31)],
}


def draw(lines, path):
    window = Gtk.OffscreenWindow()
    window.set_default_size(WIDTH, HEIGHT)
    window.set_size_request(WIDTH, HEIGHT)
    label = Gtk.Label(label="\n".join(lines))
    label.set_xalign(0)
    label.set_yalign(0)
    label.set_margin_start(8)
    label.set_margin_top(6)
    window.add(label)
    window.show_all()
    # Let it lay out and draw before the picture is taken.
    for _ in range(50):
        while GLib.MainContext.default().iteration(False):
            pass
    pixbuf = window.get_pixbuf()
    pixbuf.savev(path, "png", [], [])
    window.destroy()
    return pixbuf.get_width(), pixbuf.get_height()


FORM = [
    ("Network settings", 20),
    ("Connections", 14),
    ("Wired connection 1 is connected", 11),
    ("Address 192.168.4.44, gateway 192.168.4.1", 11),
    ("Use this connection only for resources on its network", 10),
    ("Proxy", 14),
    ("Manual proxy on port 3128 for http and https", 11),
    ("Ignore hosts: localhost, 127.0.0.0/8, ::1", 9),
    ("Changes apply when you press Save", 12),
]
BUTTONS = ["Cancel", "Apply", "Save"]


def draw_form(path):
    """A window that is not a terminal: a light form with type of several sizes and buttons."""
    window = Gtk.OffscreenWindow()
    window.set_name("form")
    window.set_default_size(WIDTH, HEIGHT)
    window.set_size_request(WIDTH, HEIGHT)
    column = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
    column.set_margin_start(40)
    column.set_margin_top(30)
    for text, size in FORM:
        label = Gtk.Label()
        label.set_name("form-text")
        label.set_markup(f'<span font_family="DejaVu Sans" size="{size * 1024}" foreground="#1b1f23">{GLib.markup_escape_text(text)}</span>')
        label.set_xalign(0)
        column.pack_start(label, False, False, 0)
    row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
    for text in BUTTONS:
        row.pack_start(Gtk.Button(label=text), False, False, 0)
    column.pack_start(row, False, False, 0)
    window.add(column)
    window.show_all()
    for _ in range(50):
        while GLib.MainContext.default().iteration(False):
            pass
    window.get_pixbuf().savev(path, "png", [], [])
    window.destroy()
    return [text for text, _ in FORM] + BUTTONS


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    provider = Gtk.CssProvider()
    provider.load_from_data(CSS)
    from gi.repository import Gdk

    Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
    for name, lines in SCREENS.items():
        size = draw(lines, os.path.join(out, name + ".png"))
        with open(os.path.join(out, name + ".txt"), "w") as text:
            text.write("\n".join(lines) + "\n")
        print(name, size, len(lines), "lines")
    lines = draw_form(os.path.join(out, "f.png"))
    with open(os.path.join(out, "f.txt"), "w") as text:
        text.write("\n".join(lines) + "\n")
    print("f", len(lines), "lines")


if __name__ == "__main__":
    main()
