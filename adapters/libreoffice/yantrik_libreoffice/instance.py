"""A LibreOffice of the adapter's own: no window, a private profile, a pipe of its own.

`--headless` starts one from here (adapter.py). The one that does an agent's work
(agent_office.py) runs the same `soffice_command`, with this profile's settings, in its sandbox.
"""

import os
import signal
import subprocess

# A LibreOffice of ours never refreshes a link when it opens a document, and never asks. The two
# apps count differently (officecfg's Calc.xcs and Writer.xcs): for Calc 0 is always, 1 never
# and 2 on request; for Writer 0 is always, 1 on request and 2 never (#942, review L1).
CALC_NEVER, WRITER_NEVER = 1, 2
NO_LINK_UPDATES = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<oor:items xmlns:oor="http://openoffice.org/2001/registry" '
    'xmlns:xs="http://www.w3.org/2001/XMLSchema" '
    'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">\n'
    '<item oor:path="/org.openoffice.Office.Calc/Content/Update">'
    '<prop oor:name="Link" oor:op="fuse"><value>%d</value></prop></item>\n'
    '<item oor:path="/org.openoffice.Office.Writer/Content/Update">'
    '<prop oor:name="Link" oor:op="fuse"><value>%d</value></prop></item>\n'
    '</oor:items>\n') % (CALC_NEVER, WRITER_NEVER)


def write_profile_settings(profile):
    """Settings in a private profile, before LibreOffice reads it. The profile is only ever one
    this adapter made, so its settings file is ours to write."""
    os.makedirs(os.path.join(profile, "user"), exist_ok=True)
    with open(os.path.join(profile, "user", "registrymodifications.xcu"), "w",
              encoding="utf-8") as f:
        f.write(NO_LINK_UPDATES)


def soffice_command(soffice, pipe, profile):
    """soffice with no window, a profile of its own, listening on `pipe`."""
    return [soffice, "--headless", "--invisible", "--nologo", "--norestore", "--nodefault",
            "--nolockcheck", "-env:UserInstallation=file://%s" % profile,
            "--accept=pipe,name=%s;urp;" % pipe]


def start_headless(soffice, pipe, profile, env=None, stderr=None):
    """Start it."""
    write_profile_settings(profile)
    return subprocess.Popen(soffice_command(soffice, pipe, profile), stdin=subprocess.DEVNULL,
                            stdout=subprocess.DEVNULL,
                            stderr=stderr if stderr is not None else subprocess.DEVNULL,
                            env=env, start_new_session=True, close_fds=True)


def stop_headless(process, office, patience=10.0):
    """Ask our LibreOffice to quit, then make sure it has."""
    office.terminate()
    try:
        process.wait(timeout=patience)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except OSError:
            process.kill()
        process.wait()
