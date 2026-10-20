"""`pytest apps/browser`: the package and the surface SDK from this tree."""

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
for path in (os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python"), HERE):
    if path not in sys.path:
        sys.path.insert(0, path)
