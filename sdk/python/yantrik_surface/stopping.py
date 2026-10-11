"""A stop asked for by a signal, waited for without the handler ever taking a lock.

`threading.Event.set()` takes the event's lock. A Python signal handler runs on the main thread
between bytecodes, so when the main thread is itself inside `Event.wait()` and holds that lock (on
its way in or out), the handler's `set()` waits for a lock its own thread holds: forever. Seen in
CI as the LibreOffice adapter never exiting on SIGTERM, its clean-stop test timing out at 30 s on
a loaded runner. The handler here only sets a flag, which takes no lock.
"""

import signal
import time


class StopOnSignal:
    """Installed at construction; `wait(timeout)` is the main loop's tick."""

    def __init__(self, signums=(signal.SIGTERM, signal.SIGINT)):
        self.asked = False
        self._previous = {signum: signal.signal(signum, self._ask) for signum in signums}

    def _ask(self, *_):
        self.asked = True

    def wait(self, timeout):
        """True once a stop has been asked for; otherwise sleeps up to `timeout` first.

        A signal does not cut the sleep short (PEP 475), so a stop is seen within one tick."""
        if not self.asked:
            time.sleep(timeout)
        return self.asked

    def restore(self):
        """Put back the handlers that were there before."""
        for signum, previous in self._previous.items():
            if previous is not None:
                signal.signal(signum, previous)
