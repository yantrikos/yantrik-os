# PR #583 review fixes

Security: this PR changes the approval buttons. The review is `review/pr-583-review.md` on `origin/review/pr-583-review`.

| # | Finding | Fix |
|---|---|---|
| 1 | BLOCKER: the accessibility default action pressed Allow once through `pointer-only` | `y_button.slint`: `accessible-action-default` is gated on `!pointer-only`, and `accessible-enabled` is false for a pointer-only button so a screen reader does not offer an action it will ignore. Test: `the_accessibility_default_action_does_not_press_a_pointer_only_button` (also pins the number of callers of `clicked()`). Mutation-checked: removing the gate fails it. |
| 2 | The test guarded a hand-built pair | New scene `verify-approval-pointer-only` drives the real `IntentLens` card: Tab, Backtab, Enter, Return and Space (12 rounds) press nothing; a click on each button answers. Source scan `the_real_approval_card_keeps_pointer_only_on_both_buttons` fails if either button loses `pointer-only: true`, or the card gains focus, key or accessible-action code. |
| 3 | Settings swatch showed teal and applied blue | Swatches read new `AccentPreset.swatch-0..4` tokens, which the presets also read, so they cannot drift. Label is "Soft blue", the saved id stays `"cyan"` for compatibility. `ThemeOverrides.accent-override` default and `AccentPreset.label` updated. |
| 4 | Teal-for-minds weakened | The mind's thinking dot (`agents.slint`) and the companion mark (`intent_lens.slint`) use `Theme.cyan`. The other `Theme.accent` uses in `agents.slint` (selection, links, focus ring) are shell chrome, not mind identity, and stay blue. `onboard_kit.slint` uses of `Theme.cyan` are pre-existing and not changed (not a mind surface, noted for the owner). |
| 5 | Email flagged state lost | Flagged is the primary kind, unflagged secondary. |
| 6 | Blast radius | Not split in this pass; the review's split is a decision for the author. Quiet icon buttons are still outlines, as the spec decided. Recorded as open. |
| 7 | Unverified claims | See "What was run" below. `preview.slint` lines 110-111 moved to variant 1. |
| 8 | Behind main | Rebased on origin/main (clean). |
| 9 | Other approval-like surfaces | `AgentProposalCard` Apply is now `pointer-only` (a proposed change is a consent). Test `a_proposals_apply_button_is_pointer_only`. |
| 10 | Dead variant 2 | Removed; any unknown kind draws as secondary. |
| 11 | Keyboard-only users cannot answer | Documented in the `pointer-only` comment in `y_button.slint`. This is the spec's choice. |
| 12-14, 15, 17 | Labels, duplicate red tokens, Restart not red, magic px, amber border | Not changed: pre-existing or spec calls; not this PR's concern. |
| 16 | Empty Trash secondary | Toolbar button is destructive (variant 3). |

## What was run
See the final message of the run for exact counts. Anything not run is listed there.
