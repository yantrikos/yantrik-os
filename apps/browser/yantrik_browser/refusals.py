"""What an action says when it declines: the sentence the caller reads, and whether it was the
page that said no (a part of driver.py, moved out of it so every module of the surface can raise
the same two)."""


class Refused(Exception):
    """An action declining, with the sentence the caller reads."""


class PageRefused(Refused):
    """The page said no and nothing was pressed or typed: a ref that is gone, a control covered
    or disabled, a commitment. Answered with `PAGE:` in front, which the MCP bridge reads as a
    policy answer — "REFUSED — nothing was run" — rather than a failure to try again."""

    def __init__(self, sentence):
        super().__init__("PAGE: " + sentence)
