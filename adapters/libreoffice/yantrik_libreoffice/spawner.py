"""One long-lived thread that starts the agents' sandbox (#953).

bwrap's `--die-with-parent` is PR_SET_PDEATHSIG, and the kernel sends that signal when the
*thread* that spawned the process exits, not the process. The surface answers each call on a
request thread, so a sandbox started there died as soon as that thread ended: `open` said OK and
the next call found no document. Every sandbox is therefore started on this thread, which lives as
long as the adapter (or until `stop`), and request threads send it their start and wait, with a
limit. The flag stays, so the sandbox still dies with the adapter; stopping the spawner ends the
thread and the kernel takes the sandboxes with it.
"""

import queue
import threading

from .basics import OfficeError

WAIT = 30.0     # seconds a request waits for the spawner to start something


class Spawner:
    def __init__(self, wait=WAIT):
        self.wait = wait
        self._lock = threading.Lock()
        self._jobs = None
        self._thread = None

    def run(self, work):
        """Call `work()` on the spawner thread and answer what it returns or raises."""
        job = (work, queue.Queue(1))
        with self._lock:
            if self._thread is None or not self._thread.is_alive():
                self._jobs = queue.Queue()
                self._thread = threading.Thread(target=self._serve, args=(self._jobs,),
                                                name="libreoffice-spawner", daemon=True)
                self._thread.start()
            self._jobs.put(job)
        try:
            ok, value = job[1].get(timeout=self.wait)
        except queue.Empty:
            raise OfficeError("the agents' sandbox was not started within %ds; nothing was done"
                              % int(self.wait)) from None
        if ok:
            return value
        raise value

    def alive(self):
        return self._thread is not None and self._thread.is_alive()

    def stop(self):
        """End the thread; the kernel then kills the sandboxes it started."""
        with self._lock:
            jobs, thread, self._jobs, self._thread = self._jobs, self._thread, None, None
        if thread is not None:
            jobs.put(None)
            thread.join(timeout=5)

    @staticmethod
    def _serve(jobs):
        while True:
            job = jobs.get()
            if job is None:
                return
            work, answer = job
            try:
                answer.put((True, work()))
            except BaseException as e:      # handed to the waiting request
                answer.put((False, e))
