# Review of PR #580 — Chat v2, branch `ui/chat-v2` (SECURITY REVIEW)

Method: `git diff origin/main...origin/ui/chat-v2` read for every file that touches approvals, questions, focus or keys. The rest of the Slint was checked for timers, literals and key handling. Compared against `docs/agents/playbook.md` and `crates/yantrik-ui/src/card_watch.rs`.
Not run: `yantrik-ui` tests, `ui-preview` and the Slint build. `bash -n tests/ui-preview/validate.sh` is OK. Nothing below has been seen rendered.

## Verdict: MERGE AFTER FIXES (no security blocker; 3 SHOULD-FIX that are about honesty and visibility of a decision)

## Approval behaviour: what stays exactly the same
| Requirement | Result |
|---|---|
| No autofocus on Approve, no Enter-to-approve | **Holds.** The card still has no `FocusScope`, no `key-pressed` and no `forward-focus` (the "Enter cannot reach either of them" comment is untouched). The only focus calls added are `composer.focus-input()`, which is the text field, as before. Return in `chat_composer.slint:86` sends the typed message. It does not touch the card. |
| `card_watch` press guard | **Intact.** `card_watch.rs` is unchanged by the diff. `approval-allow` / `approval-allow-session` / `approval-deny` are still wired one to one in `intent_lens.slint`, and the Rust handlers are untouched. |
| `control_approvals.rs` / `card_watch.rs` behaviour | `card_watch.rs`: 0 lines changed. `control_approvals.rs`: +4 lines, only `decided_at: card.decided_at.into()` in `row_for` and in three test constructors. `approvals.rs`: +8, a `decided_at: String` on `Card`, filled from `record.decided_at`, which already existed. No decision, grade or guard logic changed. I did not compile it, so confirm `cargo check` (every other `Card { … }` literal needs the field). |
| An approval binds to the action as displayed | **Holds.** The identity and target rows are unchanged; only a heading `Approval needed` was added and the buttons were renamed. The card is still built from the store's record. |
| No path for a mind or synthetic input to approve | **No new path.** No new callback calls `allow`. "View action" and "View desk" only navigate (`open-run`, `switch-window("Mind View")`). `describe` and `act` approval actions are untouched. |
| Auto-scroll never moves a card under the pointer | **Holds for auto-scroll.** Approval cards stay outside the transcript, in their own block between the transcript and the composer. The new `ChatTranscript` (a `Flickable`) cannot move them. See SHOULD-FIX 3 for a different kind of movement. |

## Findings

### SHOULD-FIX 1 — the resolved line no longer says which action, or how long the grant lasts
`intent_lens.slint` ApprovalCard, the `!root.waiting` block (diff lines ~186-250), with `decided-at` set from `approvals.rs::Store`.
Before: the line was the record, e.g. "Allowed for this session: app.action — 10:42". Now: "Approved by you · 10:42" (+ "— View action"). `decision == "allowed"` covers both "Allowed once" and **"Allowed for this session"** (a standing rule, `record.session`), so a standing grant reads exactly like a one-off.
Failure scenario: a person approved "Allow files.delete for this session". Later the transcript just says "Approved by you · 10:42". They cannot tell from the transcript that a standing rule exists, or what it covers. The full sentence is only an `accessible-description`, and the "View action" link is hidden if `agent` is empty.
This is the "did I allow that?" question the old comment protects.
Fix: keep the app.action and the scope in the visible line ("Approved once · files.delete · 10:42" / "Approved for this session · …"). Elide the end, not the facts.

### SHOULD-FIX 2 — sensitive and standard requests now look the same
`intent_lens.slint` card border: `grade == "dangerous" ? color-danger : chat-needs-you`. Before: `dangerous` → danger, `sensitive` → warning, otherwise neutral. The old comment said the grade colour is "the one thing that changes how hard you should look, visible before you start reading".
Failure scenario: a `sensitive` and a `standard` request are both amber. The grade is still in the text ("Graded sensitive. Allowing covers this one action, once.", line 700), but approval fatigue is the risk this card was designed against. The distinction was the cheap visual cue.
Fix: keep a distinct edge or header accent for `sensitive`, or put the grade next to the "Approval needed" heading.

### SHOULD-FIX 3 — the card's position depends on the work strip and the composer
`intent_lens.slint`: `height-limit` subtracts `(root.strip-shown ? chat-strip-height : 0)` and `composer.bar-height`. The strip appears when a run is live, which is when requests arrive.
Failure scenario: the person aims at "Decline"; a run goes live or leaves, or the composer grows, and the card moves up or down by 48px or more between press and release. Slint's `clicked` needs press and release inside the same `TouchArea`, so the worst outcome is a missed click, not a wrong approval: the Approve and Decline buttons share a row and a vertical move cannot put one in the place of the other. Still, a decision control that shifts under a pointer is the thing this design is guarding against.
Fix: reserve the strip's height while any approval waits (`strip-shown || approvals.length > 0`), and don't grow the composer layout while a card is up; or keep the card anchored to the bottom block. Add a preview check in `chat_tests.rs`: card y is the same before and after a strip appears.

### NIT 1 — vocabulary is now inconsistent
Buttons read "Approve once" / "Decline" but the session button still reads `"Allow " + app + "." + action + " for this session"` (`intent_lens.slint:806`), and `card_watch.rs`, the design docs, `control_approvals.rs` strings and the Rust docs still say "Allow"/"Deny". Pick one word, and rename the session button with the others. A test or doc that matches on the old label text should be updated in the same PR.

### NIT 2 — handoff cards are not in this diff
The brief says the handoff cards were re-laid out. Nothing under `provider_handoff/`, `handoff_card_tests.rs` or any handoff Slint file is changed, so I cannot confirm or deny a regression there. If the intent was to restyle them, that is not in this PR; if the PR text claims it, it should be corrected.

### NIT 3 — `view-action` passes an agent id to `open-run`
`intent_lens.slint`: `view-action => { root.open-run(item.agent); }`. The callback is documented as taking a run row id (`mind:main#n`) and "Also an agent's id from a resolved approval". I did not find the Rust handler to check that a bare agent id is accepted (grep on the branch found only the `desktop.slint` forward). Confirm it, and that an unknown id is a no-op.

### NIT 4 — question card
`question_card.slint`: "Write another answer" opens the same free-text box a question with no options had. That is a new path only for questions that have options, and the answer goes through the existing `answered` callback once. No default key, no auto-focus. Fine. Its 24px height and `600` weight are literals; use tokens.

### NIT 5 — size
`intent_lens.slint` is still the host of ApprovalCard (about 2,200 lines). The PR already splits the chat into new files. Moving ApprovalCard to its own file would be a separate PR, as the playbook says.

## Playbook checks for the rest of the PR
- Timers: the old 160 ms typing-indicator `Timer` is removed. `chat_transcript.slint` says nothing repeats or animates. I found no `Timer` in the new chat files except the existing one-shot `focus-owed` timer. OK.
- Tokens: the new colours are `Theme.chat-*` tokens. There are px literals in the new components (`24px`, `16px`, `6px`), the same NIT as above.
- Honest copy: the work card states come from the Agents store ("Queued", "Working", "Needs you" …). No invented progress. `lens_work.rs` says why there is no percentage. Good.
- Off the UI thread: no blocking call added. `publish_lens` runs in the existing `refresh`.
- Parity: purely visual. The one change visible to `describe` is the `transcript-*` outs. No new controls.
