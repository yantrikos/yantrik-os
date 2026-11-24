"""The sentence for a failed model call (#1386): the gateway's own words, nothing invented."""

import json
import unittest

import support  # noqa: F401  (puts lib on the path)
from provider_failure import provider_failure


def failed(error, **rest):
    return {"role": "assistant", "stopReason": "error", "errorMessage": error, **rest}


class ProviderFailureTests(unittest.TestCase):
    def test_a_good_message_is_no_failure(self):
        self.assertIsNone(provider_failure({"role": "assistant", "stopReason": "stop"}))
        self.assertIsNone(provider_failure({"role": "assistant"}))

    def test_the_gateways_sentence_is_used_as_it_is(self):
        body = {"error": {"message": "cloud provider is kept for you."}}
        got = provider_failure(failed("429 " + json.dumps(body), provider="Groq (free)"))
        self.assertEqual(got, "Groq (free) refused this request (429): cloud provider is kept for you.")

    def test_a_body_that_is_not_the_usual_one_says_only_the_status(self):
        got = provider_failure(failed("503 <html>Authorization: Bearer sk-secret</html>"))
        self.assertEqual(got, "The model's provider refused this request (503) and gave no reason.")
        self.assertNotIn("sk-secret", got)


if __name__ == "__main__":
    unittest.main()
