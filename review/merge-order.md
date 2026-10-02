# Merge order and conflict notes for #581 to #585

Reviewed against `origin/main` at d8cae62 (after #576 and #578). Conflicts below come from real trial merges, not from reading file lists.

## Verdicts
| PR | Branch | Verdict |
|---|---|---|
| #581 Alt+Tab | ui/alt-tab-switcher | DO NOT MERGE |
| #582 mind apps (security) | fix/mind-apps-in-mind-view | MERGE AFTER FIXES (3 blockers) |
| #583 colour system (security) | ui/colour-system | MERGE AFTER FIXES (1 blocker) |
| #584 Agents workroom | ui/agents-workroom | MERGE AFTER FIXES |
| #585 grounded dock | ui/grounded-dock | MERGE AFTER FIXES (1 blocker) |

## Recommended order
1. **#582**. It merges cleanly with main and with every other PR (it touches `control.rs` and `main.rs` only in places that don't collide). It is a security fix, so it should not wait behind UI work. Fix B1 to B3 first.
2. **#583**. Clean against main. Land it before the layout PRs so they pick up the new tokens. Fix the AT-SPI `accessible-action-default` hole in `y_button.slint` first, because every later PR uses `YButton` for approvals.
3. **#585**. Conflicts with main in `app.slint` (1 hunk) and deletes `taskbar.slint`. #583 edits one line of `taskbar.slint` (modify/delete conflict): accept the deletion and carry the token change into `grounded_dock.slint`. Fix the `mod dock_model` / `#[allow(dead_code)]` placement in `main.rs` while rebasing.
4. **#581**. It is already stale against main (conflicts in `rc.xml`, `app.slint` x2, `control.rs`, `docs/app-control.md`) and needs real fixes, so it goes after #585. After #585 it also loses its one-line `taskbar.slint` edit, and `window_switcher.slint` is deleted by #581 but edited by #583: take the deletion and re-apply the colour tokens in `alt_tab.slint`. Check that #578's cheat sheet, which is generated from `rc.xml`, still lists the new Alt+Tab binding.
5. **#584**. Largest and most isolated. Merge last and take its version of `agents.slint`; #583's 4-line edit there must be re-applied by hand (it is a token swap). It also touches `app.slint`, `control.rs`, `main.rs`, `wire/mod.rs` and `docs/app-control.md` with small additive hunks.

I tried 582, 583, 581, 585, 584 in that order: 581 conflicted in `rc.xml`, `app.slint`, `window_switcher.slint`, `control.rs` and `docs/app-control.md`; 585 conflicted in `app.slint`, `taskbar.slint`, `tests/ui-preview/src/main.rs` and `validate.sh`; 584 conflicted in `agents.slint`. The order above puts the two layout-heavy PRs (#585, #581) before #584 and keeps the modify/delete cases to one each.

## Files touched by more than one PR
| File | PRs | Notes |
|---|---|---|
| `config/labwc/rc.xml` | 581, 585 | Different regions (keybind vs 48px margin); no textual conflict between them. #581 conflicts with main. |
| `crates/yantrik-design-tokens/slint/theme.slint` | 583, 584, 585 | Additive hunks; merged cleanly in trial. |
| `crates/yantrik-ui-slint/ui/agents.slint` | 583, 584 | #584 rewrites it. Conflict. Take #584, re-apply #583's tokens. |
| `crates/yantrik-ui-slint/ui/app.slint` | 581, 584, 585 | The main conflict spot (581 x main, 585 x main, 581 x 585). Add-only wiring; resolve by keeping both. |
| `crates/yantrik-ui-slint/ui/components/taskbar.slint` | 581, 583, 585 | #585 deletes it. Modify/delete conflict. |
| `crates/yantrik-ui-slint/ui/components/window_switcher.slint` | 581, 583 | #581 deletes it. Modify/delete conflict. |
| `crates/yantrik-ui/src/control.rs` | 581, 582, 584, 585 | Each adds a module line or field; main's #578 also touched it. Keep all. |
| `crates/yantrik-ui/src/main.rs`, `wire/mod.rs` | 581, 582, 584, 585 | `mod` lines; keep all. Mind the `#[allow(dead_code)]` attribute order in #585. |
| `docs/app-control.md` | 581, 584 | One-line hunks; #581 conflicts with main. |
| `tests/ui-preview/{preview.slint,src/main.rs,validate.sh}` | 581, 583, 584, 585 | Registration lines; keep all. |

Each PR also adds a `review/*-report.md` author report on its branch (582, 583, 585). Those are separate files and don't conflict, but they shouldn't land on main.

## Cross-PR concerns
- Every PR that raises a window needs `hold_windows`: #581 (`switcher_cancel`) lacks it. #584 and #585 have it.
- #583 changes `Theme.accent` to blue, which recolours mind surfaces that used it. Check #584's new workroom doesn't pick up blue where the spec says teal.
- #582's launch routing and #585's rule "mind apps stay off the dock" should agree on how a mind's app is identified; #585 does its own exclusion in `dock_model.rs`.
- Nothing was built or run for any of these reviews (the Slint crate is too heavy for this sandbox); all findings come from reading the diffs.
