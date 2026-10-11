# Fixes for the review of PR #580 (Chat v2), branch `ui/chat-v2`

Rebased on `origin/main` (one conflict, the long `export { ... }` line in `app.slint`: main's list kept, `WorkCardData` added).

| Finding | What changed | Tests |
|---|---|---|
| SHOULD-FIX 1: resolved line lost the action and the grant's length | The line reads `Approved by you · 10:42 · files.move (this session)` or `(once)`; a decline reads `Declined by you · 10:42 · files.move`. The action text is the line's stretching part, so a narrow panel elides it, never the time or the verdict. To know "session or once" the card needed one display-only bool: `Card.session` (from `record.session`, which `record_line` already used) and `ApprovalRequest.session`, set in `row_for`. Nothing reads it to decide anything. | `approvals::approvals_a_session_grant_says_so_in_the_transcript` (card.session), `wire::lens_card_layout::the_resolved_line_names_the_action_and_how_long_the_grant_lasts`; preview check 2 in `chat_tests.rs` |
| SHOULD-FIX 2: sensitive and standard looked the same | Edge and heading follow the grade, with tokens only: `dangerous` red, `sensitive` amber with a "Sensitive" label on the heading, `standard`/`safe` the neutral `chat-border` edge with a neutral heading. The buttons are untouched, so the colour-system PR can restyle them. | `wire::lens_card_layout::the_edge_and_heading_tell_the_grades_apart`; preview check 1c |
| SHOULD-FIX 3: the card moved with the strip and the composer | The approvals block moved from above the strip and reply box to directly under the header, above the transcript, so everything above the card is the header's fixed height. Its height limit reads the composer's and the strip's worst case (`composer.bar-height-max` = 272px, `chat-strip-height`), not their current size, so the buttons' edge cannot move either. | `lens_card_layout::the_cards_height_does_not_read_the_strip_or_the_reply_box_as_they_are_now`, `the_card_sits_directly_under_the_header_above_everything_that_changes`; preview check 1b compares the card's pixels with and without the strip and with a grown reply box |

## Approval properties, unchanged
No `FocusScope`, `key-pressed`, `forward-focus`, `focus()` or `init` in `ApprovalCard` (now a test: `the_card_still_takes_no_key_and_no_focus`). Button wiring and the `card_watch` press guard are untouched; `card_watch.rs` has no diff. `control_approvals.rs` gets `session: card.session` in `row_for` plus `session: false` in three test constructors, and `approvals.rs` the one field: no decision, grade or guard logic changed.

## Not verified
- The preview checks in `tests/ui-preview/src/chat_tests.rs` (1b, 1c, the resolved-line check) were written but not built or run: the ui-preview crate needs its own full Slint build and the sandbox disk could not hold it next to the main build. Nothing here has been seen rendered. The click scan for "View action" was widened to y 100..700 because the resolved line now sits under the header.
- The card is now at the top of the panel rather than the bottom; whether that reads well beside a long transcript is a visual call nobody has made.
- NITs 1 to 5 left alone.
