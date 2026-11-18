"""Three listeners the adapter leaves outside an agent's sandbox while it starts, for the launcher
to fail to reach (fence.py's proof, #953).

A proof that tries only the person's own sockets proves little on a machine where there are none to
try. These are always there: a UNIX socket at a path in the adapter's private folder that the
sandbox does not show (the person's LibreOffice pipe and the session bus are the same kind), an
abstract UNIX socket (the display's and some buses' kind, which belong to a network namespace),
and a TCP port on the host's loopback. Each is first reached from outside, so a canary that nobody
could reach proves nothing and the start is refused; then, inside, the launcher must fail to reach
every one before LibreOffice runs.
"""

import os
import secrets
import socket

from .sandbox import SandboxError


class Canaries:
    """The listeners, open from `open()` until `close()`."""

    def __init__(self, folder):
        self.path = os.path.join(folder, "canary.sock")
        self.abstract = "yantrik-libreoffice-canary-%s" % secrets.token_hex(8)
        self.port = None
        self._sockets = []

    def open(self):
        try:
            self._listen(socket.AF_UNIX, self.path)
            self._listen(socket.AF_UNIX, "\0" + self.abstract)
            tcp = self._listen(socket.AF_INET, ("127.0.0.1", 0))
        except OSError as e:
            self.close()
            raise SandboxError("the sandbox's canaries could not be set up (%s)" % e) from None
        os.chmod(self.path, 0o600)
        self.port = tcp.getsockname()[1]
        for family, address in ((socket.AF_UNIX, self.path),
                                (socket.AF_UNIX, "\0" + self.abstract),
                                (socket.AF_INET, ("127.0.0.1", self.port))):
            if not _reaches(family, address):
                self.close()
                raise SandboxError("the sandbox's canary at %r cannot be reached even from "
                                   "outside, so failing to reach it from inside would prove "
                                   "nothing" % (address,))
        return self

    def _listen(self, family, address):
        sock = socket.socket(family, socket.SOCK_STREAM)
        self._sockets.append(sock)
        sock.bind(address)
        sock.listen(8)
        return sock

    def sockets(self):
        """The UNIX socket paths the launcher must fail to reach."""
        return [self.path]

    def abstract_names(self):
        return [self.abstract]

    def ports(self):
        return [self.port] if self.port else []

    def close(self):
        for sock in self._sockets:
            try:
                sock.close()
            except OSError:
                pass
        self._sockets = []
        try:
            os.remove(self.path)
        except OSError:
            pass


def _reaches(family, address):
    sock = socket.socket(family, socket.SOCK_STREAM)
    sock.settimeout(2.0)
    try:
        sock.connect(address)
        return True
    except OSError:
        return False
    finally:
        sock.close()
