"""The adapter against the Hermes it plugs into: every method it overrides still takes what Hermes
passes.

Hermes calls these methods; we only define them. So when Hermes changes a signature nothing here
fails, and the first sign is a gateway log line nobody reads — which is how `connect()` stopped
accepting Hermes's new `is_reconnect` and the desktop platform silently never came up. This test
binds a call shaped like Hermes's own abstract signature to ours, for each override.

It needs Hermes itself, so it runs only where Hermes is importable — inside Hermes's own runtime:

    hermes --run-module unittest discover -s harnesses/hermes/tests -p test_adapter_contract.py

`HERMES_AGENT_DIR` names a checkout other than ~/.hermes/hermes-agent. Anywhere else it skips.
"""

import importlib.util
import inspect
import os
import sys
import unittest
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]
HERMES = Path(os.environ.get("HERMES_AGENT_DIR") or Path.home() / ".hermes" / "hermes-agent")


def _load():
    if HERMES.is_dir() and str(HERMES) not in sys.path:
        sys.path.insert(0, str(HERMES))
    try:
        from gateway.platforms.base import BasePlatformAdapter
    except Exception as exc:  # no Hermes here, or not its interpreter
        raise unittest.SkipTest(f"Hermes is not importable here: {exc}")
    # As a package, the way Hermes loads a plugin directory, so `from . import desktop` resolves.
    spec = importlib.util.spec_from_file_location(
        "yantrik_desktop_plugin", PLUGIN / "__init__.py", submodule_search_locations=[str(PLUGIN)]
    )
    package = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = package
    spec.loader.exec_module(package)
    from yantrik_desktop_plugin.adapter import YantrikAdapter

    return BasePlatformAdapter, YantrikAdapter


def _call_shaped_like(signature: inspect.Signature):
    """Arguments the way a caller of this signature may pass them: every positional parameter
    positionally, every keyword-only one by name."""
    args, kwargs = [], {}
    for name, param in list(signature.parameters.items())[1:]:  # past self
        if param.kind in (param.POSITIONAL_ONLY, param.POSITIONAL_OR_KEYWORD):
            args.append(object())
        elif param.kind == param.KEYWORD_ONLY:
            kwargs[name] = object()
    return args, kwargs


class AdapterContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.base, cls.ours = _load()

    def test_every_override_accepts_whatever_hermes_may_pass_it(self):
        checked = []
        for name, ours in vars(self.ours).items():
            theirs = getattr(self.base, name, None)
            if not callable(ours) or theirs is None or name.startswith("__"):
                continue
            base_sig = inspect.signature(theirs)
            args, kwargs = _call_shaped_like(base_sig)
            with self.subTest(method=name):
                try:
                    inspect.signature(ours).bind(object(), *args, **kwargs)
                except TypeError as exc:
                    self.fail(f"Hermes calls {name}{base_sig}; ours is {name}{inspect.signature(ours)}: {exc}")
            checked.append(name)
        # The ones Hermes cannot run without. If a rename made any of these stop being overrides,
        # this is the line that says so rather than a gateway that quietly has no desktop.
        for required in ("connect", "disconnect", "send", "get_chat_info"):
            self.assertIn(required, checked)

    def test_nothing_hermes_requires_is_left_abstract(self):
        missing = sorted(getattr(self.ours, "__abstractmethods__", ()))
        self.assertEqual(missing, [], f"Hermes requires these and the adapter lacks them: {missing}")


if __name__ == "__main__":
    unittest.main()
