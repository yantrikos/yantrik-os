"""Controls whose label reads as a commitment: pressing one spends money, sends something, or
removes something, and what it did cannot be taken back.

The same words as `yos web` and yantrik-mind's browser driver, so every path draws the same line.
Broad on purpose: a false positive costs the person one card; a false negative costs money or a
sent message.
"""

import re
import unicodedata

COMMIT_WORDS = (
    "buy", "purchase", "order", "checkout", "check out", "pay", "payment", "subscribe",
    "place order", "send", "submit", "post", "publish", "confirm", "book now", "reserve",
    "delete", "remove", "cancel subscription", "deactivate", "close account", "transfer",
    "withdraw", "sign contract", "agree and", "accept and", "apply now", "donate", "tweet",
    "reply", "share", "unsubscribe", "empty trash", "sign up", "create account", "register",
)

# Whole words that make "order", "post", "share" or "reply" a place rather than an act:
# "Order history", "Post details". Whole words: "view" inside "review" is not one.
HARMLESS = ("history", "status", "details", "track", "summary", "help", "learn more", "sort",
            "view", "filter")
SOFT = ("order", "post", "share", "reply")

# What a link with an address may say and still only be a way somewhere: "submit" opens a form,
# "reply" a reply box. A link that says delete, pay or unsubscribe is judged like a button,
# because following it is often the act itself.
LINK_WORDS = ("submit", "reply", "post", "share", "sign up", "create account", "register",
              "subscribe")

# Longest first, so a phrase is found before a word inside it: "place order" is not the soft
# "order" of "order history".
_WORD = {w: re.compile(r"(?<![a-z])" + re.escape(w) + r"(?![a-z])")
         for w in sorted(COMMIT_WORDS, key=len, reverse=True)}
_HARMLESS = [re.compile(r"(?<![a-z])" + re.escape(h) + r"(?![a-z])") for h in HARMLESS]
_INVISIBLE = dict.fromkeys(map(ord, "\u00ad\u200b\u200c\u200d\u200e\u200f\u2060\ufeff"))


def fold(label):
    """A label as its words: Unicode compatibility-folded (a fullwidth or styled letter is its
    plain one), invisible characters dropped, lower case, single spaces."""
    text = unicodedata.normalize("NFKC", str(label or "")).translate(_INVISIBLE)
    return " ".join(text.lower().split())


def reads_as_commitment(label):
    """The commitment word a control's label carries, or None."""
    text = fold(label)
    if not text:
        return None
    for word, pattern in _WORD.items():
        if pattern.search(text):
            if word in SOFT and any(h.search(text) for h in _HARMLESS):
                continue
            return word
    return None


def same_label(a, b):
    """Whether two labels are the same words, whatever the spacing and case — and not empty."""
    a, b = fold(a).strip(" .…"), fold(b).strip(" .…")
    return bool(a) and a == b
