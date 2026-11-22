"""The actions every mode but full bypass asks about (#164, decision D): the Python side of
`yantrik_ipc_transport::always_asked` and of `gate::always_asks`. The Rust file is the source of
truth; `tests/test_always_asked.py` and a Rust test each fail if these tables differ from it."""

# The sentence that makes an action the person's to allow on a card, every time, in every mode
# but full bypass (`control_surface::ALWAYS_ASKS`).
ALWAYS_ASKS = "The person allows each one on a card, in every mode."

ALWAYS_ASKED = (
    ("shell", "allow_location_lookup"),
    ("shell", "allow_background_research"),
    ("shell", "report_problem"),
    ("shell", "delete_for_good"),
    ("shell", "send_chat_message"),
    ("shell", "plan_work"),
    ("shell", "change_item"),
    ("shell", "send_mail"),
)
# The names the browser surface answers to.
BROWSER_APPS = ("browser", "chromium", "app-browser")
# The browser's actions that run without a card; every other one asks, a future one included.
BROWSER_SILENT = ("describe", "read", "find", "text", "console", "screenshot", "tabs", "media",
                  "wait", "scroll", "back", "key_up")


def always_asks(purpose):
    """Does an action's own description declare ALWAYS_ASKS? Case aside — `gate::always_asks`."""
    return ALWAYS_ASKS.lower() in (purpose or "").lower()


def always_asked(app_id, action):
    """Does this action ask in every mode but full bypass — `gate::always_asked`."""
    return ((app_id, action) in ALWAYS_ASKED
            or (app_id in BROWSER_APPS and action not in BROWSER_SILENT))


def refusal(app, action, plan, how, plan_how):
    """The refusal for a declared always-asks action, `Why::Always` in the Rust gate."""
    if plan:
        return ("GRANT: %s.%s is the person's to allow on a card, and this machine is in plan "
                "mode, which raises none — so it was not run. %s" % (app, action, plan_how))
    return ("GRANT: %s.%s is the person's to allow on a card every time, in every mode and "
            "whatever was allowed for the session before — so it was not run. %s"
            % (app, action, how))
