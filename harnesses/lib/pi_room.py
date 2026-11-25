"""Pi reads its models.json and settings.json at its start; the desktop rewrites them when the
model picked in the ask bar changes (#1402), and Pi, which runs with `--no-session`, keeps its
conversation only in its process, so it is never restarted for that. Between turns this sends it
`/yantrik-room` (extension/yantrik-room.ts), which makes the running Pi read them again."""

import os
import threading
from typing import Any, Callable, Dict, Optional, Tuple

COMMAND = "/yantrik-room"
# How long a turn waits for Pi to answer the command before going on without it.
WAIT = 20.0
FILES = ("models.json", "settings.json")


class RoomReload:
    def __init__(self, env: Dict[str, str], enabled: bool) -> None:
        home = env.get("HOME") or os.path.expanduser("~")
        self.dir = env.get("PI_CODING_AGENT_DIR") or os.path.join(home, ".pi", "agent")
        self.enabled = enabled
        self.loaded: Tuple[Any, ...] = ()
        self._asked: Optional[int] = None
        self._answered = threading.Event()

    def stamp(self) -> Tuple[Any, ...]:
        out = []
        for name in FILES:
            try:
                st = os.stat(os.path.join(self.dir, name))
                out.append((st.st_mtime_ns, st.st_size))
            except OSError:
                out.append(None)
        return tuple(out)

    def before_turn(self, was_alive: bool, send: Callable[[Dict[str, Any]], int],
                    log: Callable[[str], None]) -> None:
        """Pi is idle here: it was started just now (it read the files), or a turn ended."""
        now = self.stamp()
        if not was_alive or not self.enabled or now == self.loaded:
            self.loaded = now if not was_alive else self.loaded
            return
        # Taken before Pi reads, so a write during the reload shows again at the next turn.
        self.loaded = now
        self._answered.clear()
        self._asked = send({"type": "prompt", "message": COMMAND})
        if not self._answered.wait(WAIT):
            log("pi did not answer %s in %d seconds; this turn goes on" % (COMMAND, int(WAIT)))

    def saw(self, event: Dict[str, Any]) -> bool:
        """Pi's answer to the command, on Pi's reader thread: true when it was that one."""
        if event.get("type") == "response" and self._asked is not None and str(event.get("id")) == str(self._asked):
            self._answered.set()
            return True
        return False
