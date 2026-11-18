"""Where an agent's LibreOffice work is done: a LibreOffice of its own, in a sandbox (#942, #953).

The person's LibreOffice runs with all of the person's rights. Whatever a document in it asks for
(a formula, a link, a macro, an import filter, an export) it reads and reaches as the person, so an
agent's calls never go there. They go to this one: headless, with a private profile and pipe,
started on an agent's first `open` inside the sandbox sandbox.py builds, through the launcher in
fence.py, which puts Landlock up inside where the kernel offers it and proves the sandbox holds
before soffice runs. It works on copies of the person's files (workspace.py, copies.py).

Where the sandbox cannot be built (no bwrap, user namespaces off, LibreOffice installed in the
person's own files) or does not prove, every agent call is refused with `CLOSED_TO_AGENTS` and the
reason, and nothing is started: there is no unsandboxed way round.

The UNO pipe: LibreOffice always makes its pipe in /tmp, and inside the sandbox /tmp is the
adapter's private `pipes` folder. Outside, the adapter pins the socket made there into a folder
the sandbox is not shown, checks who listens on it, and connects through a link in the real /tmp,
at the name a UNO client looks for, to that pinned socket (unopipe.py). Nothing inside can follow
it back out, or point it anywhere else.

The work folder is the sandbox's to change, so the adapter reaches into it only through
workdir.py, by handles, never following what it finds there.
"""

import os
import secrets
import shutil
import subprocess
import tempfile
import threading
import time

from . import fence, places, sandbox
from .basics import OfficeError, PathMap, absolute, apply_rule
from .unopipe import Pipe, PipeRefused
from .canaries import Canaries
from .instance import soffice_command, stop_headless, write_profile_settings
from .office import Office
from .spawner import Spawner
from .workdir import WorkFolder
from .workspace import HELD, AgentWorkspace, lock_file  # noqa: F401 - the tests' names for them

# How long the sandboxed LibreOffice gets to start answering on its pipe.
START_PATIENCE = 60.0
# How long a successful check that bwrap can make the sandbox stands before it is made again.
CHECKED_FOR = 60.0

CLOSED_TO_AGENTS = ("LibreOffice isn't open to agents yet: its sandbox is still being built "
                    "(#953). Nothing was run.")
UNSANDBOXED = CLOSED_TO_AGENTS + " This machine cannot build it: %s."
NO_DOCUMENT = "no document is open in LibreOffice; `open` one first"
NOT_SHOWN = ("the agents' LibreOffice works only on its copies of the person's files, and %s is "
             "not one of them")
LAUNCHER = fence.__file__


class AgentOffice:
    """The sandboxed LibreOffice for agents' calls, started on the first `open` and stopped by
    `stop`.

    `person` is the person's own `Office`, asked which files are open there. `admit` is the file
    rule, set by the surface. `home` and `env` stand in for the session's in a test, and `bwrap`
    for the places bwrap is looked for."""

    def __init__(self, soffice="soffice", person=None, uno_module=None, home=None, env=None,
                 patience=START_PATIENCE, bwrap=None):
        self.soffice = soffice
        self.person = person
        self.uno_module = uno_module
        self.home = home
        self.env = env
        self.patience = patience
        self.bwrap = tuple(bwrap) if bwrap else sandbox.BWRAP
        self.admit = None
        self.landlock = None    # what the launcher put up inside, as it said it
        self._lock = threading.Lock()
        self._spawner = Spawner()
        self._checked = (None, 0.0)
        self._process = self._office = self._workspace = self._private = None
        self._pipe = self._work = None

    # ── what an agent's call is served by ───────────────────────────────────

    def workspace(self, start=False):
        """The sandboxed LibreOffice's workspace; started with `start`, otherwise only when it is
        running. Raises `OfficeError`; never answers with an unsandboxed one."""
        with self._lock:
            if self._process is not None and self._process.poll() is not None:
                self._clean_up()
            if self._workspace is not None:
                return self._workspace
            bwrap = self._sandbox()
            if not start:
                raise OfficeError(NO_DOCUMENT)
            self._start(self._env(), bwrap)
            return self._workspace

    def open(self, path, read_only=False):
        """An agent's `open`: refused first where no sandbox can be built, then the file rule,
        and only then is anything started."""
        path = absolute(path)
        if self._workspace is None:
            with self._lock:
                self._sandbox()
        apply_rule(self.admit, "read", path)
        return self.workspace(start=True).open(path, read_only)

    def documents(self):
        """What describe lists for the agents' LibreOffice: its documents, if it is running, by
        the person's paths. Read without the lock, so describe never waits on a start."""
        workspace = self._workspace
        if workspace is None:
            return []
        try:
            return workspace.overview()[0]
        except OfficeError:
            return []

    def stop(self):
        with self._lock:
            self._stop_locked()
        self._spawner.stop()    # the thread's end kills any sandbox still under it

    # ── the sandbox ─────────────────────────────────────────────────────────

    def _sandbox(self):
        """The bwrap to build the sandbox with, once it has been seen to make one here."""
        bwrap, when = self._checked
        if bwrap and time.monotonic() - when < CHECKED_FOR:
            return bwrap
        try:
            bwrap = sandbox.find_bwrap(self.bwrap)
            sandbox.check(bwrap)
        except sandbox.SandboxError as e:
            raise OfficeError(UNSANDBOXED % e) from None
        self._checked = (bwrap, time.monotonic())
        return bwrap

    def _env(self):
        return dict(os.environ if self.env is None else self.env)

    def _home(self, env):
        home = self.home or env.get("HOME") or os.path.expanduser("~")
        if not os.path.isabs(home):
            raise OfficeError(UNSANDBOXED % "the session has no home folder to keep out")
        return os.path.normpath(home)

    def _inner_rule(self, work):
        """The sandboxed `Office`'s rule: its files are its copies, and the scan's verdicts
        (formulas, links, references outside the document) are the surface's rule's."""
        def admit(what, path):
            if what in ("read", "write", "document"):
                if path is not None and not places.within(path, work):
                    raise OfficeError(NOT_SHOWN % path)
                return
            apply_rule(self.admit, what, path)
        return admit

    def _start(self, env, bwrap):
        program = shutil.which(self.soffice)
        if program is None:
            raise OfficeError("there is no LibreOffice (`%s`) on this machine for an agent's work"
                              % self.soffice)
        home = self._home(env)
        try:
            root = places.install_root(program, home)
        except places.Unsuitable as e:
            raise OfficeError(UNSANDBOXED % e) from None
        if not os.access(sandbox.PYTHON, os.X_OK):
            raise OfficeError(UNSANDBOXED % ("there is no %s to prove the sandbox with"
                                             % sandbox.PYTHON))
        private = sandbox.Private(tempfile.mkdtemp(prefix="yantrik-libreoffice-agent-",
                                                   dir="/tmp")).make()
        self._private = private
        self._work = WorkFolder(private.work)
        pipe = "yantrik-libreoffice-agent-%d-%s" % (os.getpid(), secrets.token_hex(6))
        canaries = Canaries(private.root)
        deadline = time.monotonic() + self.patience
        try:
            canaries.open()
            write_profile_settings(private.profile)
            argv = sandboxed(bwrap, private, root, home, env, canaries,
                             soffice_command(program, pipe, sandbox.PROFILE))
            with open(private.log, "wb") as log:
                # On the spawner's thread, which outlives the request's: --die-with-parent
                # follows the thread that started bwrap (spawner.py).
                self._process = self._spawner.run(lambda: subprocess.Popen(
                    argv, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                    env=sandbox.environment(env), start_new_session=True, close_fds=True))
            self._pipe = Pipe(pipe, private.pipes, private.uno, self._process.pid)
            self._await_pipe(private, deadline)
            self._pipe.pin()
        except sandbox.SandboxError as e:
            self._stop_locked()
            raise OfficeError(UNSANDBOXED % e) from None
        except OfficeError:
            self._stop_locked()
            raise
        finally:
            canaries.close()
        office = Office(pipe, uno_module=self.uno_module, hidden=True,
                        admit=self._inner_rule(private.work), agents_only=True,
                        paths=PathMap(private.work, sandbox.WORK), files=self._work,
                        reach=self._pipe.check)
        self._office = office
        self._await_answer(office, deadline)
        self.landlock = _said(private.log, "proved, with ")
        self._workspace = AgentWorkspace(office, self._work, admit=self.admit,
                                         person=self.person)

    def _await_pipe(self, private, deadline):
        """Until the sandboxed LibreOffice has made its pipe, once the launcher proved the
        sandbox."""
        while not self._pipe.made():
            if self._process.poll() is not None:
                refused = _said(private.log, "refused: ")
                if refused:
                    raise sandbox.SandboxError(refused)
                raise OfficeError("the agents' LibreOffice would not start inside its sandbox: %s"
                                  % _last_line(private.log))
            if time.monotonic() >= deadline:
                raise OfficeError("the agents' LibreOffice did not answer within %ds; nothing "
                                  "was done" % int(self.patience))
            time.sleep(0.1)

    def _await_answer(self, office, deadline):
        while True:
            try:
                office.desktop()
                return
            except PipeRefused:
                self._stop_locked()
                raise
            except OfficeError:
                if self._process.poll() is not None or time.monotonic() >= deadline:
                    said = _last_line(self._private.log)
                    self._stop_locked()
                    raise OfficeError("the agents' LibreOffice did not answer within %ds (%s); "
                                      "nothing was done" % (int(self.patience), said)) from None
                time.sleep(0.2)

    def _stop_locked(self):
        if self._process is not None and self._process.poll() is None:
            if self._office is not None:
                stop_headless(self._process, self._office)
            else:
                _kill(self._process)
        self._clean_up()

    def _clean_up(self):
        if self._pipe:
            self._pipe.remove()
        if self._work:
            self._work.close()
        if self._private:
            shutil.rmtree(self._private.root, ignore_errors=True)
        self._process = self._office = self._workspace = self._private = None
        self._pipe = self._work = None
        self.landlock = None


def sandboxed(bwrap, private, root, home, env, canaries, argv):
    """The command that runs `argv` (in the sandbox's paths) in an agent's sandbox, through the
    launcher's Landlock and proof: what `AgentOffice` starts soffice with, and what the tests
    start their stand-ins with."""
    inner = fence.command(sandbox.PYTHON, sandbox.LAUNCHER, sandbox.inner_rules(root),
                          places.probes(home, env), argv,
                          sockets=places.persons_sockets(env) + canaries.sockets(),
                          abstract=canaries.abstract_names(), ports=canaries.ports())
    return sandbox.command(bwrap, private, root, LAUNCHER, places.hidden(home, env), inner)


def _kill(process):
    try:
        os.killpg(process.pid, 9)
    except OSError:
        process.kill()
    process.wait()


def _lines(path):
    try:
        with open(path, "rb") as f:
            return [ln.strip() for ln in f.read().decode("utf-8", "replace").splitlines()
                    if ln.strip()]
    except OSError:
        return []


def _said(path, marker):
    """What the launcher said after `marker` on its line in the log, or None."""
    for line in reversed(_lines(path)):
        if line.startswith("yantrik-libreoffice-sandbox: ") and marker in line:
            return line.split(marker, 1)[1]
    return None


def _last_line(path):
    lines = _lines(path)
    return lines[-1] if lines else "it exited without saying why"
