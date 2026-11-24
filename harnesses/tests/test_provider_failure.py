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

    def test_the_gateways_sentence_is_used_as_it_is_in_each_of_the_three_forms(self):
        sentence = "Groq (free) is rate-limited right now (too many requests). It said: slow down."
        body = json.dumps({"error": {"message": sentence}})
        for form in ("429 " + sentence, "429: " + body, "429 " + body):
            got = provider_failure(failed(form, provider="yantrik"))
            self.assertEqual(got, sentence, form)

    def test_the_text_form_keeps_its_reason(self):
        self.assertEqual(provider_failure(failed("429 Rate limit reached")), "Rate limit reached.")

    def test_no_reason_says_only_the_status(self):
        for form in ("503", "503 ", "503: {}", "503 {not json", '503 {"error": {}}'):
            got = provider_failure(failed(form))
            self.assertEqual(got, "The model's account refused this request (503) and gave no reason.", form)
        self.assertEqual(provider_failure(failed("")), "The model's account refused this request and gave no reason.")


if __name__ == "__main__":
    unittest.main()
