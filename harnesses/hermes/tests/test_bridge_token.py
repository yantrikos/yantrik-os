"""bridge_token.py: how the tools bridge is given the gateway agent's token without an environment
(#411). Stdlib only; a real unix socket and a real child process, no Hermes."""

import os
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bridge_token  # noqa: E402

TOKEN = "0123456789abcdef0123456789abcdef"


def ask(path):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as conn:
        conn.settimeout(3.0)
        conn.connect(path)
        chunks = []
        while True:
            chunk = conn.recv(256)
            if not chunk:
                break
            chunks.append(chunk)
    return b"".join(chunks).decode()


@unittest.skipUnless(sys.platform.startswith("linux"), "unix sockets and /proc")
class TokenServiceTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "home", "yantrik-bridge.sock")
        self.service = bridge_token.TokenService(self.path)
        self.service.start()
        self.addCleanup(self.dir.cleanup)
        self.addCleanup(self.service.stop)

    def test_it_answers_the_token_to_a_descendant_of_this_process_and_closes(self):
        self.service.set_token(TOKEN)
        self.assertEqual(ask(self.path), TOKEN)
        self.assertEqual(ask(self.path), TOKEN, "every start of the bridge gets it")

    def test_it_answers_nothing_while_there_is_no_token(self):
        self.assertEqual(ask(self.path), "")
        self.service.set_token(TOKEN)
        self.service.set_token(None)
        self.assertEqual(ask(self.path), "", "an attach that is lost takes the token back")

    def test_a_process_that_is_not_ours_is_answered_nothing(self):
        self.service.set_token(TOKEN)
        with mock.patch.object(bridge_token, "descends_from", return_value=False):
            self.assertEqual(ask(self.path), "")
        with mock.patch("os.getuid", return_value=os.getuid() + 1):
            self.assertEqual(ask(self.path), "", "nor another account")

    def test_the_socket_is_private_and_goes_with_the_service(self):
        self.assertEqual(os.stat(self.path).st_mode & 0o777, 0o600)
        self.assertEqual(os.stat(os.path.dirname(self.path)).st_mode & 0o077, 0, "its directory is the account's alone")
        self.service.stop()
        self.assertFalse(os.path.exists(self.path))

    def test_a_real_child_is_a_descendant_and_an_unrelated_pid_is_not(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(5)"])
        self.addCleanup(child.kill)
        self.assertTrue(bridge_token.descends_from(child.pid, os.getpid()))
        self.assertFalse(bridge_token.descends_from(1, os.getpid()), "PID 1 is nobody's child")
        self.assertTrue(bridge_token.descends_from(os.getpid(), os.getpid()))

    def test_the_path_is_the_one_hermes_config_writes_into_the_bridges_entry(self):
        install = Path(__file__).resolve().parents[2] / "lib" / "install"
        sys.path.insert(0, str(install))
        try:
            import hermes_config
        finally:
            sys.path.remove(str(install))
        with mock.patch.dict(os.environ, {"HERMES_HOME": "/h/.hermes"}):
            self.assertEqual(bridge_token.socket_path(), hermes_config.token_socket())
        with mock.patch.dict(os.environ, {"HERMES_HOME": ""}):
            self.assertEqual(bridge_token.socket_path(), hermes_config.token_socket())


@unittest.skipUnless(sys.platform.startswith("linux"), "pass_fds")
class PopenWithTokenTests(unittest.TestCase):
    CHILD = (
        "import os,sys; fd = os.environ.get('YANTRIK_AGENT_TOKEN_FD'); "
        "t = os.read(int(fd), 100).decode() if fd else ''; "
        "print(t, os.environ.get('YANTRIK_AGENT_TOKEN') or '-', open('/proc/self/cmdline').read().replace(chr(0), ' '))"
    )

    def test_the_child_reads_it_from_a_pipe_and_it_is_in_no_environment_or_command_line(self):
        env = dict(os.environ, YANTRIK_AGENT_TOKEN="stale-from-the-parent")
        child = bridge_token.popen_with_token([sys.executable, "-c", self.CHILD], TOKEN, env=env,
                                              stdout=subprocess.PIPE, text=True)
        out = child.communicate(timeout=10)[0]
        token, plain, cmdline = out.split(" ", 2)
        self.assertEqual(token, TOKEN)
        self.assertEqual(plain, "-", "the plain variable is dropped, even one the parent had")
        self.assertNotIn(TOKEN, cmdline)

    def test_no_token_starts_it_with_none_of_the_variables(self):
        env = dict(os.environ, YANTRIK_AGENT_TOKEN="x", YANTRIK_AGENT_TOKEN_FD="9")
        child = bridge_token.popen_with_token([sys.executable, "-c", self.CHILD], None, env=env,
                                              stdout=subprocess.PIPE, text=True)
        self.assertTrue(child.communicate(timeout=10)[0].startswith(" - "))


if __name__ == "__main__":
    unittest.main()
