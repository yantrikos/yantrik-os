"""The attach line `_detail()` builds, without Hermes installed.

The Settings map names a provider from this line, so the host of the endpoint Hermes calls
belongs on it — the host alone, since a URL can carry a key in its user info or query string.
Hermes is not needed to see that: gateway is stubbed to let adapter.py import, and hermes_cli
is replaced per test with one that reports a chosen config.
"""

import importlib.util
import sys
import types
import unittest
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]
# Not the contract test's name: pytest may load both in one process, and only this one stubs gateway.
PACKAGE = "yantrik_hermes_detail_plugin"
STUBBED = ("gateway", "gateway.config", "gateway.platforms", "gateway.platforms.base", "gateway.session")
FAKE = ("hermes_cli", "hermes_cli.config")


def _stub(name, **attrs):
    module = types.ModuleType(name)
    for key, value in attrs.items():
        setattr(module, key, value)
    sys.modules[name] = module
    return module


def _load_detail():
    """adapter.py as a plugin inside a fake Hermes: gateway imports stubbed, nothing else."""

    class BasePlatformAdapter:  # the one name adapter.py needs at class-definition time
        pass

    for name in STUBBED:
        _stub(name)
    sys.modules["gateway.config"].Platform = object
    sys.modules["gateway.config"].PlatformConfig = object
    base = sys.modules["gateway.platforms.base"]
    base.BasePlatformAdapter = BasePlatformAdapter
    for name in ("MessageEvent", "MessageType", "ProcessingOutcome", "SendResult"):
        setattr(base, name, object)
    sys.modules["gateway.session"].build_session_key = lambda *a, **kw: ""

    spec = importlib.util.spec_from_file_location(
        PACKAGE, PLUGIN / "__init__.py", submodule_search_locations=[str(PLUGIN)]
    )
    package = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = package
    spec.loader.exec_module(package)
    return sys.modules[PACKAGE + ".adapter"]._detail


def _hermes_reporting(config):
    """A hermes_cli 0.14.0 whose load_config returns this; None means Hermes is not installed."""
    for name in FAKE:
        sys.modules.pop(name, None)
    if config is None:
        return
    _stub("hermes_cli", __version__="0.14.0")
    _stub("hermes_cli.config", load_config=lambda: config)
    sys.modules["hermes_cli"].config = sys.modules["hermes_cli.config"]


class DetailTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.detail = _load_detail()

    @classmethod
    def tearDownClass(cls):
        gone = [n for n in sys.modules if n in STUBBED or n.startswith(PACKAGE + ".") or n in FAKE]
        for name in gone:
            sys.modules.pop(name, None)

    def setUp(self):
        _hermes_reporting(None)

    def test_the_endpoint_hermes_calls_rides_the_line_as_a_host(self):
        _hermes_reporting({"model": {"default": "deepseek-v4.1-flash", "provider": "ollama-cloud",
                                     "base_url": "https://ollama.com/v1"}})
        self.assertEqual(self.detail(), "Hermes 0.14.0 · deepseek-v4.1-flash · ollama.com")

    def test_no_endpoint_named_is_exactly_what_was_ever_sent(self):
        _hermes_reporting({"model": {"default": "deepseek-v4.1-flash", "provider": "ollama-cloud"}})
        self.assertEqual(self.detail(), "Hermes 0.14.0 · deepseek-v4.1-flash")

    def test_only_the_host_survives_a_url_that_carries_a_key(self):
        _hermes_reporting({"model": {"default": "m1", "base_url": "https://user:pass@API.example.com:8443/v1?key=SECRET"}})
        detail = self.detail()
        self.assertEqual(detail, "Hermes 0.14.0 · m1 · api.example.com")
        for leaked in ("SECRET", "user", "pass", "8443", "/v1"):
            self.assertNotIn(leaked, detail)

    def test_a_name_that_is_no_url_names_no_host(self):
        _hermes_reporting({"model": {"default": "m1", "base_url": "ollama-cloud"}})
        self.assertEqual(self.detail(), "Hermes 0.14.0 · m1")

    def test_a_model_section_that_is_a_bare_name_still_reports(self):
        _hermes_reporting({"model": "deepseek-v4.1-flash"})
        self.assertEqual(self.detail(), "Hermes 0.14.0")

    def test_without_hermes_there_is_still_a_line(self):
        self.assertEqual(self.detail(), "Hermes")


if __name__ == "__main__":
    unittest.main()
