"""A caller that hangs up before its answer never takes the app down with it.

A standalone Python ignores SIGPIPE, so a reply written to a socket whose caller had already gone
raised an error and nothing more. Blender's embedded Python does not: the same write killed
Blender outright (VM 520, 28 Sep 2026 — a render in progress, a describe that gave up after five
seconds, and the scene gone). Any caller, a mind at the door included, could end the person's app
by hanging up early. Every write the SDK makes to a socket now asks the kernel not to signal.

The surface here runs in a process of its own with SIGPIPE at its default action, as it is in
Blender, so the test fails the way Blender failed if a write is left without the flag.
"""

import os
import socket
import subprocess
import sys
import textwrap
import time
import unittest

import support

SERVER = textwrap.dedent("""
    import signal, sys, time
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)   # as an embedding host leaves it
    sys.path.insert(0, %(package)r)
    from yantrik_surface import Surface
    s = Surface("slow", socket_path=%(path)r)
    s.describe_timeout = 0.6

    @s.view
    def state():
        time.sleep(1.2)          # slower than the caller will wait, and than the turn allows
        return {}

    @s.action("wait", grade="safe")
    def wait() -> dict:
        '''Take a while.'''
        time.sleep(1.2)
        return {"ok": True}

    s.serve_in_thread()
    print("up", flush=True)
    while True:
        time.sleep(0.2)
""")


class TestAHungUpCaller(support.MachineCase):

    def start(self):
        path = os.path.join(self.machine.socket_dir, "app-slow.sock")
        os.makedirs(self.machine.socket_dir, exist_ok=True)
        proc = subprocess.Popen([sys.executable, "-c", SERVER % {"package": support.PACKAGE_ROOT,
                                                                   "path": path}],
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                env=self.machine.env())
        self.addCleanup(proc.kill)
        self.assertEqual(proc.stdout.readline().strip(), b"up")
        return proc, path

    def hang_up_after_asking(self, path, line):
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.connect(path)
        client.sendall(line)
        client.close()

    def ping(self, path):
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(3)
        client.connect(path)
        try:
            client.sendall(b'{"jsonrpc":"2.0","id":9,"method":"rpc.ping"}\n')
            return client.recv(4096)
        finally:
            client.close()

    def test_the_app_outlives_callers_that_hang_up_before_their_answer(self):
        proc, path = self.start()
        for line in (b'{"jsonrpc":"2.0","id":1,"method":"app.describe"}\n',
                     b'{"jsonrpc":"2.0","id":2,"method":"app.act","params":{"action":"wait"}}\n'):
            for _ in range(3):
                self.hang_up_after_asking(path, line)
        time.sleep(2.5)  # every answer has been written, into a socket nobody holds
        self.assertIsNone(proc.poll(), "the app died writing to a caller that had gone "
                                       "(exit %s)" % proc.returncode)
        self.assertIn(b"pong", self.ping(path), "and it still answers")


if __name__ == "__main__":
    unittest.main()
