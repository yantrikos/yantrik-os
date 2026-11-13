"""How a CDP message travels (#1137): a WebSocket to a debugging port, or the pipe a browser
started with `--remote-debugging-pipe` reads on fd 3 and writes on fd 4.

One interface for both, so cdp.py does not know which it has: `send(text)`, `recv() -> text`
(raises on end of stream) and `close()`. A pipe message is JSON ended by a NUL byte.

The person's Browser has no TCP port at all; its pipe is a connected socket pair (the shell made
it; the browser's ends are fds 3 and 4), so nothing can open it again through /proc/<pid>/fd the
way a named pipe could be.
"""

import os
import select
import threading

POLL = 0.25         # seconds between looks at `closed` while waiting for the browser to speak
MAX_MESSAGE = 64 * 1024 * 1024


class Closed(Exception):
    """The other end is gone, or this end was closed."""


class WebSocketTransport:
    def __init__(self, ws):
        self.ws = ws

    def send(self, text):
        self.ws.send(text)

    def recv(self):
        return self.ws.recv()

    def close(self):
        self.ws.close()


class PipeTransport:
    """CDP over a pair of file descriptors (or one socket, for both). Closing it stops this
    reader and closes THIS transport's descriptors; with `shutdown` set (it owns the stream), the
    socket is also shut down so the browser sees the end."""

    def __init__(self, read_fd, write_fd=None, shutdown=None):
        self.rfd, self.wfd = read_fd, read_fd if write_fd is None else write_fd
        self.shut = shutdown
        self.closed = False
        self.released = False
        self.buffer = b""
        self.reading = threading.Lock()     # held while a recv is in progress
        self.writing = threading.Lock()

    @staticmethod
    def frame(text):
        """The bytes of one message. A NUL inside it would end it early, so it is refused."""
        data = text.encode("utf-8")
        if b"\0" in data:
            raise ValueError("a message with a NUL byte cannot travel on the pipe")
        return data + b"\0"

    def send(self, text):
        data = self.frame(text)
        with self.writing:
            while data:
                if self.closed:
                    raise Closed()
                try:
                    n = os.write(self.wfd, data)
                except OSError as e:
                    raise Closed() from e
                data = data[n:]

    def recv(self):
        """The next message. Raises Closed at the end of the stream or after close()."""
        with self.reading:
            while True:
                end = self.buffer.find(b"\0")
                if end >= 0:
                    message, self.buffer = self.buffer[:end], self.buffer[end + 1:]
                    return message.decode("utf-8", "replace")
                if len(self.buffer) > MAX_MESSAGE:
                    raise Closed()
                if self.closed:
                    raise Closed()
                try:
                    ready, _, _ = select.select([self.rfd], [], [], POLL)
                    if not ready:
                        continue
                    chunk = os.read(self.rfd, 65536)
                except (OSError, ValueError) as e:
                    raise Closed() from e
                if not chunk:
                    raise Closed()
                self.buffer += chunk

    def close(self, wait=1.0):
        """Stop, wait for a reader in recv() to leave (within POLL seconds), then close THIS
        transport's descriptors. After that nothing of it reads or writes, which is what makes
        the stream safe to give to the next connection."""
        self.closed = True
        if self.reading.acquire(timeout=wait):
            self.reading.release()
        with self.writing:
            if self.released:
                return
            self.released = True
        for fd in {self.rfd, self.wfd}:
            try:
                os.close(fd)
            except OSError:
                pass
        if self.shut is not None:
            try:
                self.shut.shutdown(2)
            except OSError:
                pass
