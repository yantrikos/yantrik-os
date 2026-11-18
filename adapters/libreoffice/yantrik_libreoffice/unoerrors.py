"""What a UNO exception is called and what it said."""


def uno_error_name(error):
    """The UNO type of an exception, unqualified (`NoConnectException`), however pyuno named the
    Python class it made for it."""
    for owner in (type(error), error):
        name = getattr(owner, "__pyunostruct__", None) or getattr(owner, "typeName", None)
        if isinstance(name, str) and name:
            return name.rsplit(".", 1)[-1]
    return type(error).__name__.rsplit(".", 1)[-1]


def uno_message(error):
    """What a UNO exception said, or its type when it said nothing."""
    message = getattr(error, "Message", None)
    if isinstance(message, str) and message.strip():
        return message.strip()
    text = str(error).strip()
    return text or uno_error_name(error)
