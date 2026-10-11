# #587 review fixes

- SHOULD-FIX 1/2: the late-answer notification is a fixed sentence, "<Mind> answered after you left. Open the chat to read it.", and never quotes the answer. The mind's name is the catalogue display name when there is one, else the harness id with control and bidi characters stripped and cut to 40 characters. No notification is raised in Private mode.
- SHOULD-FIX 3: `describe shell` publishes `conversation_status` only when `!agent_reading`, flattened, stripped of invisible characters and clipped to 120 (`status_for_describe`).
- NIT 1: covered by the same status sanitising on the way out.
- NIT 2: the send-failure path uses the capped `Flight::keep_late`.
- Tests: harness 115 passed; yantrik-ui bin 1163 passed, 1 failed (`harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`: the sandbox shell prints "nvm" from its profile; the file is untouched by this PR).
- Not done: NIT 3 (failed turns and undetected departures stay silent; the notice does not name the conversation).
