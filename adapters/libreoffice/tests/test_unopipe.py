"""The agents' UNO pipe is connected to only when it is the socket the sandbox made (unopipe.py;
#953 review H2).

The sandbox can write the folder its pipe is made in. Without Landlock it can put a link there,
to the person's own LibreOffice pipe, and a client connecting by that name from outside would
reach the person's unsandboxed LibreOffice. Here the sandbox's LibreOffice is a child process
listening on a socket in a `served` folder, the person's is a listener in this process, and the
real /tmp is a folder of the test's. The control shows the link reaches the person's listener
when connected to by name, as before the fix."""

import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest import mock

import support  # noqa: F401 - puts the adapter on the path
from yantrik_libreoffice import unopipe
from yantrik_libreoffice.office import Office

PERSONS = "SENTINEL-the-persons-libreoffice"
SANDBOXED = "the-sandboxed-libreoffice"

LISTENER = r"""
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(8)
print("ready", flush=True)
while True:
    c, _ = s.accept()
    try:
        c.sendall(sys.argv[2].encode())
    except OSError:
        pass
    c.close()
"""


def serve_here(test, path, said):
    """A listener in this process, which is not below the sandbox: the person's LibreOffice."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.bind(path)
    s.listen(8)
    test.addCleanup(s.close)

    def serve():
        while True:
            try:
                c, _ = s.accept()
            except OSError:
                return
            try:
                c.sendall(said.encode())
            except OSError:
                pass
            c.close()
    threading.Thread(target=serve, daemon=True).start()


def hear(path):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(5)
    try:
        s.connect(path)
        return s.recv(100).decode()
    finally:
        s.close()


@unittest.skipUnless(sys.platform.startswith("linux"), "SO_PEERCRED and /proc are Linux's")
class TestThePipe(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="yantrik-unopipe-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.served, self.pinned, self.real_tmp = (os.path.join(self.tmp, n)
                                                   for n in ("pipes", "uno", "tmp"))
        for folder in (self.served, self.pinned, self.real_tmp):
            os.mkdir(folder, 0o700)
        self.persons = os.path.join(self.real_tmp, "OSL_PIPE_%d_yantrik-libreoffice" % os.getuid())
        serve_here(self, self.persons, PERSONS)
        # The sandbox's bwrap stands in as this process: only its children are in the sandbox.
        self.pipe = unopipe.Pipe("agent-test", self.served, self.pinned, os.getpid(),
                                 tmp=self.real_tmp)
        self.addCleanup(self.pipe.remove)
        self.socket = os.path.join(self.served, self.pipe.file)

    def sandbox_listens(self):
        child = subprocess.Popen([sys.executable, "-I", "-c", LISTENER, self.socket, SANDBOXED],
                                 stdout=subprocess.PIPE, text=True)
        self.addCleanup(child.stdout.close)
        self.addCleanup(child.wait)
        self.addCleanup(child.kill)
        self.assertEqual(child.stdout.readline().strip(), "ready")

    def plant(self, where, target):
        """What the sandbox's code does: its name swapped for a link."""
        if os.path.lexists(where):
            os.remove(where)
        os.symlink(target, where)

    def refused(self, call):
        with self.assertRaises(unopipe.PipeRefused) as caught:
            call()
        return str(caught.exception)

    def test_control_a_link_in_the_sandboxs_tmp_leads_to_the_persons_libreoffice(self):
        self.plant(self.socket, self.persons)
        self.assertEqual(hear(self.socket), PERSONS)

    def test_a_link_where_the_socket_should_be_is_never_pinned(self):
        self.plant(self.socket, self.persons)
        self.assertIn("not a socket", self.refused(self.pipe.pin))
        self.assertEqual(os.listdir(self.pinned), [])
        self.assertFalse(os.path.lexists(self.pipe.link))

    def test_a_socket_served_from_outside_the_sandbox_is_refused(self):
        serve_here(self, self.socket, PERSONS)
        self.assertIn("is not in the sandbox", self.refused(self.pipe.pin))
        self.assertFalse(os.path.lexists(self.pipe.link))

    def test_once_pinned_the_name_reaches_the_sandbox_whatever_is_done_to_it_inside(self):
        self.sandbox_listens()
        self.pipe.pin()
        self.assertEqual(hear(self.pipe.link), SANDBOXED)
        self.plant(self.socket, self.persons)
        self.pipe.check()
        self.assertEqual(hear(self.pipe.link), SANDBOXED)

    def test_a_uno_name_or_pinned_socket_changed_after_pinning_is_refused(self):
        self.sandbox_listens()
        self.pipe.pin()
        self.plant(self.pipe.link, self.persons)
        self.assertIn("no longer the adapter's link", self.refused(self.pipe.check))
        self.plant(self.pipe.link, self.pipe.pinned)
        self.pipe.check()
        self.plant(self.pipe.pinned, self.persons)
        self.assertIn("pinned socket was replaced", self.refused(self.pipe.check))

    def test_a_name_already_held_in_tmp_is_refused_and_kept(self):
        self.sandbox_listens()
        self.plant(self.pipe.link, self.persons)
        self.assertIn("could not be made", self.refused(self.pipe.pin))
        self.assertEqual(os.readlink(self.pipe.link), self.persons)

    def test_a_socket_swapped_for_a_link_while_it_is_pinned_is_refused(self):
        self.sandbox_listens()
        real = os.link

        def swapped_first(*args, **kwargs):
            self.plant(self.socket, self.persons)
            return real(*args, **kwargs)
        with mock.patch.object(unopipe.os, "link", swapped_first):
            self.assertIn("changed while it was pinned", self.refused(self.pipe.pin))
        self.assertEqual(os.listdir(self.pinned), [])
        self.assertFalse(os.path.lexists(self.pipe.link))

    def test_descent_is_strict_and_bounded(self):
        tree = {5: 4, 4: 3, 3: 1, 1: 0}
        self.assertTrue(unopipe.descends(5, 3, tree.get))
        self.assertFalse(unopipe.descends(3, 3, tree.get))
        self.assertFalse(unopipe.descends(5, 9, tree.get))
        self.assertFalse(unopipe.descends(1, 2, {1: 1}.get), "a loop ends")


class TestReach(unittest.TestCase):
    def test_office_asks_reach_before_every_connection_and_connects_to_nothing_refused(self):
        uno = mock.MagicMock()
        refusal = unopipe.PipeRefused(unopipe.REFUSED % "a test")
        office = Office("p", uno_module=uno, reach=mock.Mock(side_effect=refusal))
        with self.assertRaises(unopipe.PipeRefused):
            office.desktop()
        resolver = uno.getComponentContext.return_value.ServiceManager \
            .createInstanceWithContext.return_value
        resolver.resolve.assert_not_called()
        office.reach = mock.Mock()
        office.desktop()
        office.reach.assert_called_once_with()
        resolver.resolve.assert_called_once()


if __name__ == "__main__":
    unittest.main()
