#!/usr/bin/env python3
"""A small GTK window that does what it is told on stdin and prints when it did it.

Each action prints `ACT <name> <unix ms>` just before it happens, so a reader of the screen
model can be timed against the moment the application itself changed.
"""
import sys
import threading
import time

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk  # noqa: E402


def stamp(what):
    print(f"ACT {what} {int(time.time() * 1000)}", flush=True)


class Demo(Gtk.Window):
    def __init__(self):
        super().__init__(title="sight-demo: Expenses")
        self.set_default_size(420, 220)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        box.set_border_width(12)
        self.label = Gtk.Label(label="Merchant")
        self.entry = Gtk.Entry()
        self.button = Gtk.Button(label="Save")
        self.password = Gtk.Entry()
        self.password.set_visibility(False)
        for widget in (self.label, self.entry, self.password, self.button):
            box.pack_start(widget, False, False, 0)
        self.add(box)
        self.dialog = None
        self.connect("destroy", Gtk.main_quit)
        self.connect("map-event", lambda *_: stamp("mapped"))

    def burst(self, left, total):
        # One change per main-loop turn, so the toolkit announces every one of them.
        self.entry.set_text(f"line {total - left} of {total}")
        if left > 1:
            GLib.idle_add(self.burst, left - 1, total)
        else:
            stamp("burst-end")
        return False

    def keystrokes(self, left):
        # One character per 90 ms, the way a person types.
        self.password.set_text(self.password.get_text() + "x")
        if left > 1:
            GLib.timeout_add(90, self.keystrokes, left - 1)
        else:
            stamp("password-end")
        return False

    def do(self, line):
        command, _, arg = line.partition(" ")
        if command == "focus-password":
            stamp("focus-password")
            self.password.grab_focus()
        elif command == "type-password":
            stamp("password-start")
            self.keystrokes(int(arg))
        elif command == "title-long":
            stamp("title-long")
            self.set_title("sight-demo: " + "long " * 600)
        elif command == "title":
            stamp("title")
            self.set_title(arg)
        elif command == "text":
            stamp("text")
            self.entry.set_text(arg)
        elif command == "burst":
            stamp("burst-start")
            self.burst(int(arg), int(arg))
        elif command == "focus-button":
            stamp("focus-button")
            self.button.grab_focus()
        elif command == "focus-entry":
            stamp("focus-entry")
            self.entry.grab_focus()
        elif command == "dialog":
            self.dialog = Gtk.MessageDialog(
                transient_for=self,
                modal=False,
                message_type=Gtk.MessageType.QUESTION,
                buttons=Gtk.ButtonsType.OK_CANCEL,
                text="Delete 3 expenses?",
            )
            self.dialog.set_title("sight-demo: Confirm")
            stamp("dialog")
            self.dialog.show_all()
        elif command == "close-dialog" and self.dialog is not None:
            stamp("close-dialog")
            self.dialog.destroy()
            self.dialog = None
        elif command == "quit":
            stamp("quit")
            self.destroy()
        return False


def main():
    window = Demo()

    def commands():
        for line in sys.stdin:
            GLib.idle_add(window.do, line.strip())

    threading.Thread(target=commands, daemon=True).start()
    stamp("show")
    window.show_all()
    Gtk.main()


if __name__ == "__main__":
    main()
