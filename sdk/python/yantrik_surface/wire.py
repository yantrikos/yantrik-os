"""The wire: newline-delimited JSON-RPC 2.0 over a unix socket, and the revision hash.

A port of `yantrik-ipc-transport`'s server and of the envelope half of
`yantrik-ipc-contracts::control_surface`. A caller has to be unable to tell a surface built on
this package from one built on the Rust runtime, and these are the things it can observe:

  * the socket directory — the chain `server::socket_dir()` walks, in its order
    (`$XDG_RUNTIME_DIR/yantrik` → `/run/yantrik` → `/tmp/yantrik-<uid>`), tightened the same
    way (directory 0700, socket node 0600), so `yos` and every other client find a surface
    here exactly where they find the rest;
  * the framing — one JSON object per line, request and reply alike, several requests per
    connection, `rpc.ping` and `rpc.service_id` answered like the transport answers them;
  * the error codes — -32700 parse, -32601 unknown method, -32602 the app refusing, -32000
    the app failing to answer;
  * the revision — FNV-1a-64 over the summary, a zero byte, and the state rendered the way
    `serde_json::Value::to_string` renders it, which is what `View::revision()` hashes.

Framing is the protocol's (docs/surface-protocol.md, section 1): a request is a JSON object with
`jsonrpc`, `method` and `id` present — a batch or a notification is answered as a parse error with
`"id": null`, as the transport's serde parse answers it. Names are owned (section 3): before
binding, whatever is at the path is asked `rpc.ping`, and a socket that answers — or accepts and
stays silent for a second — keeps its name; only a socket nobody listens on, a symlink or a stray
file is replaced (`owner::claim` in the transport).
"""

import contextlib
import errno
import json
import math
import os
import socket
import socketserver
import stat
import struct
import sys
import threading
import time
import traceback
from collections import namedtuple

from . import mind_door, privacy

# JSON-RPC error codes, the transport's own constants.
RPC_PARSE_ERROR = -32700
RPC_INVALID_REQUEST = -32600
RPC_METHOD_NOT_FOUND = -32601
RPC_INVALID_PARAMS = -32602
RPC_TRANSPORT_ERROR = -32000

# FNV-1a, 64-bit. Written out rather than taken from a hash library for the reason the Rust
# side writes it out: this value crosses a socket and turns up in logs, so it has to mean the
# same thing on both sides and in tomorrow's build.
FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x00000100000001B3
_MASK64 = (1 << 64) - 1

# The range serde_json holds a JSON integer in exactly (i64 below zero, u64 above). Past it a
# JSON number is read as an f64, so the canonical rendering renders it as one.
_I64_MIN = -(1 << 63)
_U64_MAX = (1 << 64) - 1

_encode_str = json.encoder.encode_basestring  # ensure_ascii=False: serde_json's escaping


# ── JSON values ──────────────────────────────────────────────────────────────


def jsonable(value, where="the value"):
    """`value` as plain JSON data: dicts with string keys, lists, str, int, float, bool, None.

    What a Rust caller can hold is a `serde_json::Value`, so that is what goes on the wire: a
    tuple becomes a list, an int or str subclass (an `IntEnum`, a `StrEnum`) its plain value,
    and a float that is not finite becomes `null` — `serde_json::json!(f64::NAN)` is `Null`,
    and `NaN` is not JSON at all. Anything else is the app's bug, said as a `TypeError` that
    names where it was found, rather than a reply the caller cannot parse.
    """
    if value is None or value is True or value is False:
        return value
    if isinstance(value, str):
        return str.__str__(value)
    if isinstance(value, int):
        return int(value)
    if isinstance(value, float):
        value = float(value)
        return value if math.isfinite(value) else None
    if isinstance(value, dict):
        return {_key(k, where): jsonable(v, where) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [jsonable(v, where) for v in value]
    raise TypeError("%s holds a %s, which is not JSON and cannot be sent over the socket"
                    % (where, type(value).__name__))


def _key(key, where):
    """A dict key as JSON has it — the conversions `json.dumps` makes, made up front."""
    if isinstance(key, str):
        return str.__str__(key)
    if key is True:
        return "true"
    if key is False:
        return "false"
    if key is None:
        return "null"
    if isinstance(key, int):
        return int.__repr__(int(key))
    if isinstance(key, float):
        return float.__repr__(float(key))
    raise TypeError("%s has a %s as a key; JSON keys are text" % (where, type(key).__name__))


def format_float(x):
    """One f64 the way serde_json 1.0 writes it.

    The shortest digits that read back as the same double (Python's `repr` finds the same
    ones), laid out as serde_json lays them out: plain decimal from 1e-5 up to 1e16, always
    with a fractional part (`100.0`); scientific outside that, with an explicit sign on the
    exponent and no padding (`1e+16`, `1.5e-7`). Python's own `repr` differs at both ends
    (`1e-05`, `2.5e-05`), which is why this exists. Checked against serde_json 1.0.149 over
    ~900 doubles; the vectors in `sdk/python/tests/test_revision.py` hold it there.
    """
    if not math.isfinite(x):
        return "null"
    if x == 0.0:
        return "-0.0" if math.copysign(1.0, x) < 0 else "0.0"
    sign = "-" if x < 0 else ""
    text = repr(abs(x))
    mantissa, _, exp = text.partition("e")
    exponent = int(exp) if exp else 0
    whole, _, fraction = mantissa.partition(".")
    digits = (whole + fraction).lstrip("0")
    exponent -= len(fraction)
    stripped = digits.rstrip("0")
    exponent += len(digits) - len(stripped)
    digits = stripped
    length = len(digits)
    kk = length + exponent  # 10^(kk-1) <= |x| < 10^kk
    if 0 <= exponent and kk <= 16:
        body = digits + "0" * exponent + ".0"
    elif 0 < kk <= 16:
        body = digits[:kk] + "." + digits[kk:]
    elif -5 < kk <= 0:
        body = "0." + "0" * (-kk) + digits
    else:
        e = kk - 1
        tail = ("e+%d" % e) if e >= 0 else ("e%d" % e)
        body = (digits if length == 1 else digits[0] + "." + digits[1:]) + tail
    return sign + body


def canonical_state(state):
    """The state as the revision hashes it: `serde_json::Value::to_string`, byte for byte.

    Compact, object keys sorted (serde_json's map is a BTreeMap; sorting Python strings by
    code point is sorting their UTF-8 bytes), non-ASCII kept raw, control characters escaped
    as `\\u00xx`, floats as `format_float` writes them.
    """
    out = []
    _render(jsonable(state, "the state"), out)
    return "".join(out)


def _render(value, out):
    if value is None:
        out.append("null")
    elif value is True:
        out.append("true")
    elif value is False:
        out.append("false")
    elif isinstance(value, str):
        out.append(_encode_str(value))
    elif isinstance(value, int):
        if _I64_MIN <= value <= _U64_MAX:
            out.append(int.__repr__(value))
        else:
            try:
                out.append(format_float(float(value)))
            except OverflowError:
                out.append("null")
    elif isinstance(value, float):
        out.append(format_float(value))
    elif isinstance(value, list):
        out.append("[")
        for i, item in enumerate(value):
            if i:
                out.append(",")
            _render(item, out)
        out.append("]")
    else:  # dict, already made plain by jsonable
        out.append("{")
        for i, key in enumerate(sorted(value)):
            if i:
                out.append(",")
            out.append(_encode_str(key))
            out.append(":")
            _render(value[key], out)
        out.append("}")


def revision(summary, state):
    """A short fingerprint of everything a view reports. `View::revision()` in Python.

    Not a counter: two revisions can only be compared for difference. That is all a caller
    needs — "has what I looked at changed since I looked" — and `expect_revision` on `app.act`
    is where the comparison is made atomically.
    """
    h = FNV_OFFSET
    data = (summary.encode("utf-8", "surrogatepass") + b"\x00"
            + canonical_state(state).encode("utf-8", "surrogatepass"))
    for byte in data:
        h ^= byte
        h = (h * FNV_PRIME) & _MASK64
    return "%016x" % h


# ── where sockets live ───────────────────────────────────────────────────────


def socket_dir():
    """The session's socket directory, by the chain `server::socket_dir()` walks.

    `$XDG_RUNTIME_DIR/yantrik` → `/run/yantrik` → `/tmp/yantrik-<uid>`: the first candidate
    that exists or can be created, and can be tightened to 0700. Like the Rust original it
    falls through to the last candidate when none can be prepared, which will then fail to
    bind and say where.
    """
    candidates = []
    xdg = os.environ.get("XDG_RUNTIME_DIR", "").strip()
    if xdg:
        candidates.append(os.path.join(xdg, "yantrik"))
    candidates.append("/run/yantrik")
    last_resort = "/tmp/yantrik-%d" % os.getuid()
    candidates.append(last_resort)

    for directory in candidates:
        try:
            if not os.path.isdir(directory):
                os.makedirs(directory)
        except OSError:
            continue
        try:
            _harden(directory)
        except OSError:
            continue
        return directory
    return last_resort


def _harden(directory):
    """Restrict a socket directory to its owner — `server::harden`, including its no-op.

    The early return is what makes asking twice safe: on an already-private directory this
    must not re-issue a chmod, because a second chmod is exactly what a sandbox or a
    read-only mount denies (the comment on `harden()` in server.rs is the account).
    """
    mode = stat.S_IMODE(os.stat(directory).st_mode)
    if mode & 0o777 == 0o700:
        return
    os.chmod(directory, 0o700)


def socket_name(service_id):
    """The file a service id binds: `<id>.sock`. An app's service id is `app-<app id>`."""
    return "%s.sock" % service_id


def default_socket_path(app_id):
    """Where an app's surface binds, `app-<id>.sock`, as `RpcServer::default_address` has it."""
    return os.path.join(socket_dir(), socket_name("app-%s" % app_id))


# ── the server ───────────────────────────────────────────────────────────────


class RpcError(Exception):
    """A refusal or failure that travels as a JSON-RPC error object rather than a result."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


class SocketBusy(OSError):
    """A live process already answers at the path: binding over it would take its name.
    Its text is the sentence alone, as the transport's error reads."""

    def __str__(self):
        return self.strerror or super().__str__()


class PeerRefused(ConnectionError):
    """The process listening at a path is not the one a caller may talk to (the shell's rule);
    nothing was written to it. The message is the rule's sentence."""


PeerCred = namedtuple("PeerCred", "pid uid gid")
PeerCred.__doc__ = """Who opened a connection, as the kernel says it (`SO_PEERCRED`), not as
the caller says it. Read at accept, like the transport reads it."""


def peer_cred(sock):
    """The peer of an accepted unix socket, or None where the kernel will not say."""
    try:
        raw = sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("iII"))
        pid, uid, gid = struct.unpack("iII", raw)
    except (AttributeError, OSError, struct.error):
        return None
    return PeerCred(pid, uid, gid)


def answers(path, timeout=0.5):
    """Whether a live process accepts connections at `path` right now (a symlink is followed).

    A connect that succeeds is a live owner; one that is refused, finds nothing, or finds a
    file that is not a socket is a dead name. Nothing is sent.
    """
    probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    probe.settimeout(timeout)
    try:
        probe.connect(path)
        return True
    except OSError:
        return False
    finally:
        probe.close()


class Handler:
    """What the server asks when a request arrives. `surface.Surface` implements it.

    `handle_from` is told who is on the other end; a handler that only defines `handle`
    is called without it, the way the Rust trait's default drops the credentials.
    """

    service_id = "app"

    def handle(self, method, params):
        raise NotImplementedError


class Server:
    """The unix-socket server: a thread of its own, one thread per connection.

    Line-delimited both ways, like the transport. The name is owned: `start()` refuses with
    `SocketBusy` when a live process answers at the path, and replaces only a dead socket or a
    stale file — so a second copy of an app cannot silently take the first one's name, and a
    crashed run's leftovers never stop the next run. The node comes out 0600 (best effort, as
    the transport's `private_socket_file`); the directory's 0700 is what keeps other users out.

    `links` are other names for the same surface: symlinks beside the socket with a relative
    target, made before the bind like `link_names` makes them, never over a live socket of
    somebody else's, and taken away again on `stop()` if they still point here.
    """

    def __init__(self, path, handler, links=()):
        self.path = path
        self.handler = handler
        self.links = list(links)
        self.linked = []
        self._server = None
        self._thread = None
        self._inode = None
        # The mind door (#411), when this machine has one: the same surface, for callers the
        # kernel says are the mind account and nobody else. None when there is no door.
        self.door = None
        self._door_server = None
        self._door_inode = None

    def start(self):
        directory = os.path.dirname(self.path)
        if directory and not os.path.isdir(directory):
            try:
                os.makedirs(directory)
            except OSError as e:
                raise OSError(e.errno, "cannot create socket directory %s: %s (set "
                              "XDG_RUNTIME_DIR to a writable per-user path)"
                              % (directory, e.strerror)) from e
        claim(self.path)
        self.linked = [link for link in self.links if self._link(link)]
        try:
            self._server = _UnixServer(self.path, _Connection)
        except OSError as e:
            self._unlink_links()
            raise OSError(e.errno, "cannot bind %s: %s" % (self.path, e.strerror)) from e
        self._server.handler = self.handler  # read by _Connection.handle
        try:
            os.chmod(self.path, 0o600)
        except OSError as e:
            print("[yantrik] could not take the group and world bits off %s (%s); it stays at "
                  "the umask default, and the directory's 0700 still keeps other users out"
                  % (self.path, e), file=sys.stderr)
        self._inode = _inode(self.path)
        self._thread = threading.Thread(
            target=self._server.serve_forever,
            name="%s-rpc" % getattr(self.handler, "service_id", "app"),
            daemon=True,
        )
        self._thread.start()
        self._open_door()
        return self

    def _open_door(self):
        """Listen at the mind door as well, when the machine has one set up exactly as the
        updater makes it (`mind_door.serving_dir`) and this surface's socket is one that gets a
        door. A door that cannot be opened is said and skipped: the person's socket still works,
        and minds cannot reach this surface — as `bind_door` failing does in the transport."""
        door = mind_door.serving_dir()
        path = mind_door.door_for(self.path, socket_dir(), door) if door else None
        if path is None:
            return
        try:
            claim(path)
            server = _DoorServer(path)
        except OSError as e:
            print("[yantrik] the mind door could not be opened at %s (%s); minds cannot reach "
                  "this surface" % (path, e), file=sys.stderr)
            return
        server.handler = self.handler
        try:
            # The minds' group writes it (the directory's setgid gave it the group); nobody else.
            # Bound closed while the person is in Private mode; the shell opens it when they
            # leave it (mind_door::close_door in the transport).
            os.chmod(path, 0o600 if privacy.is_private() else 0o660)
        except OSError as e:
            server.server_close()
            with contextlib.suppress(OSError):
                os.unlink(path)
            print("[yantrik] the mind door at %s could not be made the minds' (%s); minds cannot "
                  "reach this surface" % (path, e), file=sys.stderr)
            return
        self.door = path
        self._door_server = server
        self._door_inode = _inode(path)
        threading.Thread(target=server.serve_forever, daemon=True,
                         name="%s-door" % getattr(self.handler, "service_id", "app")).start()
        print("[yantrik] also answering minds at %s" % path, file=sys.stderr)

    def _link(self, link):
        target = os.path.basename(self.path)
        try:
            existing = os.readlink(link)
        except OSError:
            existing = None
        if existing == target:
            return True
        if os.path.lexists(link):
            if answers(link):
                print("[yantrik] %s is another live surface's name; this one does not take it"
                      % link, file=sys.stderr)
                return False
            try:
                os.unlink(link)
            except OSError:
                return False
        try:
            os.symlink(target, link)
        except OSError as e:
            print("[yantrik] could not link %s (%s); callers holding that name cannot reach "
                  "this surface" % (link, e), file=sys.stderr)
            return False
        return True

    def stop(self):
        if self._server is not None:
            self._server.shutdown()
            self._server.server_close()
            self._server = None
        # Only what is still ours: a path now bound by someone else is theirs.
        if self._inode is not None and _inode(self.path) == self._inode:
            try:
                os.unlink(self.path)
            except OSError:
                pass
        self._inode = None
        self._unlink_links()
        if self._door_server is not None:
            # The mind connections still open end with the surface, not after it.
            self._door_server.close_all()
            self._door_server.shutdown()
            self._door_server.server_close()
            self._door_server = None
            if self._door_inode is not None and _inode(self.door) == self._door_inode:
                with contextlib.suppress(OSError):
                    os.unlink(self.door)
            self._door_inode = None
            self.door = None

    def _unlink_links(self):
        target = os.path.basename(self.path)
        for link in self.linked:
            try:
                if os.readlink(link) == target:
                    os.unlink(link)
            except OSError:
                pass
        self.linked = []


def _inode(path):
    try:
        st = os.lstat(path)
    except OSError:
        return None
    return (st.st_dev, st.st_ino)


# How long a bind waits for whatever is on its path to answer `rpc.ping`, as `CLAIM_PING`.
CLAIM_PING = 1.0


def who_holds(path, patience=CLAIM_PING):
    """Who holds a socket path now: ("nobody", None) when nothing listens there, ("answers",
    service_id or None) when something answered `rpc.ping`, ("silent", None) when something
    accepted the connection and said nothing in time — `owner::who_holds`."""
    probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    probe.settimeout(patience)
    try:
        try:
            probe.connect(path)
        except OSError:
            return "nobody", None
        reader = probe.makefile("rb")

        def ask(method):
            try:
                send_all(probe, ('{"jsonrpc":"2.0","id":1,"method":"%s"}\n' % method).encode())
                return json.loads(reader.readline().decode("utf-8"))
            except (OSError, ValueError):
                return None

        reply = ask("rpc.ping")
        if isinstance(reply, dict) and ("result" in reply or "error" in reply):
            named = ask("rpc.service_id")
            name = named.get("result") if isinstance(named, dict) else None
            return "answers", name if isinstance(name, str) else None
        return "silent", None
    finally:
        probe.close()


def claim(path):
    """Make `path` free to bind, or raise `SocketBusy` saying whose it is — `owner::claim`,
    in its sentences. A symlink or a regular file is not a listener and is removed; a socket
    nobody listens on (a crashed process's) is removed; one that answers, or accepts and keeps
    quiet, belongs to a running process and is left alone."""
    try:
        st = os.lstat(path)
    except FileNotFoundError:
        return
    if not stat.S_ISSOCK(st.st_mode):
        os.unlink(path)
        return
    holder, name = who_holds(path)
    if holder == "nobody":
        try:
            os.unlink(path)
        except FileNotFoundError:
            pass
        return
    if holder == "answers":
        raise SocketBusy(
            errno.EADDRINUSE,
            "another instance owns %s: it answered rpc.ping%s. Refusing to start rather than take "
            "the name from a running process — stop that one first, or talk to it."
            % (path, " as `%s`" % name if name else ""))
    raise SocketBusy(
        errno.EADDRINUSE,
        "something is listening on %s but did not answer rpc.ping within %ds. Refusing to start "
        "rather than take the name from a process that may only be busy — stop it first."
        % (path, int(CLAIM_PING)))


class _UnixServer(socketserver.ThreadingUnixStreamServer):
    # Connection threads must not outlive the process: a caller that hangs up mid-line would
    # otherwise hold the interpreter open in threading._shutdown.
    daemon_threads = True
    block_on_close = False


class _Connection(socketserver.StreamRequestHandler):
    """One caller. Reads lines until the caller goes away."""

    def handle(self):
        self.serve(peer_cred(self.connection))

    def serve(self, peer, max_line=None, door=False):
        """Answer one request per line until the caller goes away. With `max_line`, a line longer
        than that is answered as a parse error and the connection is closed: nothing past the
        limit is buffered. At the `door`, every request is refused while the person is private."""
        handler = getattr(self.server, "handler", None)
        if door:
            handler = _PrivateDoor(handler)
        while True:
            try:
                raw = self.rfile.readline(max_line + 1) if max_line else self.rfile.readline()
            except OSError:  # the caller went away, or sat silent past the connection's timeout
                return
            if not raw:
                return
            if max_line and len(raw) > max_line and not raw.endswith(b"\n"):
                with contextlib.suppress(OSError):
                    send_all(self.connection, encode(_error(
                        None, RPC_PARSE_ERROR, "Parse error: request too large — at most %d bytes "
                        "on one line here; this one was longer, so it was not read and the "
                        "connection is closed. Nothing was run." % max_line)))
                return
            line = raw.decode("utf-8", errors="replace").strip()
            if not line:
                continue
            reply = answer(handler, line, peer)
            try:
                send_all(self.connection, encode(reply))
            except OSError:
                return


# Never a signal for writing to a socket whose other end has gone. A standalone Python ignores
# SIGPIPE; an embedding host need not, and Blender does not — a reply written to a caller that had
# already hung up killed Blender outright, render and unsaved scene with it (VM 520, 28 Sep 2026),
# and any caller, a mind at the door included, could do that on purpose. With this flag the write
# fails with EPIPE, which is an OSError like any other lost caller.
_NO_SIGNAL = getattr(socket, "MSG_NOSIGNAL", 0)


def send_all(sock, data):
    """`sock.sendall(data)`, without SIGPIPE."""
    sock.sendall(data, _NO_SIGNAL)


# The longest request line a mind may send at the door, and how long a door connection may sit
# silent before it is closed. A mind that has not yet shown it is an attached agent could
# otherwise make the person's app buffer one endless line, or hold every slot open forever.
DOOR_MAX_LINE = 4 * 1024 * 1024
DOOR_IDLE = 120.0


class _DoorServer(_UnixServer):
    """The mind door's listener. The peer is checked on the accepting thread, before a thread is
    started for it, and anyone but the mind account is closed unread. At most `DOOR_CONNECTIONS`
    are served at once; one more waits to be accepted, as the transport's semaphore makes it wait
    — a mind holding connections open uses up its own share and nothing of the person's."""

    def __init__(self, path):
        super().__init__(path, _DoorConnection)
        self.slots = threading.BoundedSemaphore(mind_door.DOOR_CONNECTIONS)
        self.held = set()
        self.held_lock = threading.Lock()
        self.stopping = False
        self._refused = 0
        self._said = None

    def verify_request(self, request, client_address):
        peer = peer_cred(request)
        if peer is not None and mind_door.is_mind(peer.uid):
            return True
        # Said at most every ten seconds, with a count: a caller that knocks in a loop does not
        # get to fill the person's log.
        self._refused += 1
        now = time.monotonic()
        if self._said is None or now - self._said >= 10:
            print("[yantrik] refused %d caller(s) at the mind door that are not the mind account "
                  "(the last: uid %s)" % (self._refused, peer.uid if peer else "unknown"),
                  file=sys.stderr)
            self._refused, self._said = 0, now
        return False

    def process_request(self, request, client_address):
        while not self.slots.acquire(timeout=0.5):
            if self.stopping:
                self.shutdown_request(request)
                return
        with self.held_lock:
            self.held.add(request)
        super().process_request(request, client_address)

    def shutdown_request(self, request):
        with self.held_lock:
            mine = request in self.held
            self.held.discard(request)
        if mine:
            self.slots.release()
        super().shutdown_request(request)

    def close_all(self):
        """End every mind connection still open, and accept no more."""
        self.stopping = True
        with self.held_lock:
            held = list(self.held)
        for request in held:
            with contextlib.suppress(OSError):
                request.shutdown(socket.SHUT_RDWR)


class _DoorConnection(_Connection):
    """One caller at the mind door, already checked on accept to be the mind account. Checked
    again here, so the uid the surface is told is the one the kernel gave for this connection."""

    timeout = DOOR_IDLE

    def handle(self):
        peer = peer_cred(self.connection)
        if peer is None or not mind_door.is_mind(peer.uid):
            return
        self.serve(peer, max_line=DOOR_MAX_LINE, door=True)


class _PrivateDoor:
    """A door handler that refuses every request while the person is in Private mode, asked per
    request so turning it on reaches a mind already connected, and passes it on otherwise."""

    def __init__(self, inner):
        self.inner = inner
        self.service_id = getattr(inner, "service_id", "app")

    def handle_from(self, method, params, peer):
        if privacy.is_private():
            raise RpcError(RPC_INVALID_PARAMS, privacy.REFUSAL)
        if hasattr(self.inner, "handle_from"):
            return self.inner.handle_from(method, params, peer)
        return self.inner.handle(method, params)


def encode(reply):
    """One reply line. A result that is not JSON is the app's fault, said as -32000."""
    try:
        text = json.dumps(reply, ensure_ascii=False, allow_nan=False)
    except (TypeError, ValueError) as e:
        text = json.dumps(_error(reply.get("id"), RPC_TRANSPORT_ERROR,
                                 "the app answered with something that is not JSON: %s" % e),
                          ensure_ascii=False)
    return (text + "\n").encode("utf-8", "surrogatepass")


def answer(handler, line, peer=None):
    """One request line, one response object. Framing only; no policy lives here."""
    try:
        request = json.loads(line)
    except ValueError as e:
        return _error(None, RPC_PARSE_ERROR, "Parse error: %s" % e)
    # What the transport's `RpcRequest` requires: an object with `jsonrpc` and `method` as text
    # and an `id` (a notification has none, and is not served). A line that is not such a
    # request is a parse error, answered with `"id": null` as serde's refusal is.
    if not isinstance(request, dict):
        return _error(None, RPC_PARSE_ERROR,
                      "Parse error: a request is a JSON object, not %s" % _kind(request))
    for key in ("jsonrpc", "method"):
        if key in request and not isinstance(request[key], str):
            return _error(None, RPC_PARSE_ERROR,
                          "Parse error: `%s` must be a string, not %s"
                          % (key, _kind(request[key])))
    for key in ("jsonrpc", "method", "id"):
        if key not in request:
            return _error(None, RPC_PARSE_ERROR, "Parse error: missing field `%s`" % key)
    request_id = request["id"]
    method = request["method"]
    params = request.get("params")
    if params is None:
        params = {}

    if method == "rpc.ping":
        return _result(request_id, "pong")
    if method == "rpc.service_id":
        return _result(request_id, getattr(handler, "service_id", "app"))

    try:
        if hasattr(handler, "handle_from"):
            value = handler.handle_from(method, params, peer)
        else:
            value = handler.handle(method, params)
        return _result(request_id, value)
    except RpcError as e:
        return _error(request_id, e.code, e.message)
    except Exception as e:  # noqa: BLE001 - an unhandled fault is an answer, not a crash
        traceback.print_exc(file=sys.stderr)
        return _error(request_id, RPC_TRANSPORT_ERROR,
                      "the app failed while answering %s: %s: %s"
                      % (method, type(e).__name__, e))


def _kind(value):
    if isinstance(value, list):
        return "an array"
    if isinstance(value, str):
        return "a string"
    if isinstance(value, bool):
        return "a boolean"
    if isinstance(value, (int, float)):
        return "a number"
    return "null"


def _result(request_id, value):
    return {"jsonrpc": "2.0", "id": request_id, "result": value}


def _error(request_id, code, message):
    return {"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}}


# ── the client half, for spending grants and for tests ───────────────────────


def call_once(path, method, params, timeout=10.0, request_id=1, peer_rule=None):
    """One request over a socket, the way `yos` sends one; the whole reply object back.

    `peer_rule`, when given, is asked about the process listening at `path` (its `PeerCred`, or
    None) before anything is written; a sentence back raises `PeerRefused` with it."""
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(timeout)
    try:
        client.connect(path)
        if peer_rule is not None:
            problem = peer_rule(peer_cred(client))
            if problem:
                raise PeerRefused(problem)
        payload = json.dumps(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params},
            ensure_ascii=False) + "\n"
        send_all(client, payload.encode("utf-8"))
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = client.recv(65536)
            if not chunk:
                break
            buf += chunk
        if not buf:
            raise ConnectionError("Connection closed before response")
        return json.loads(buf.decode("utf-8"))
    finally:
        client.close()
