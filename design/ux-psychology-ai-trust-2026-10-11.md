# Trusting a machine that acts for you: the AI layer of the psychology (2026-10-11)

A companion to `ux-psychology-2026-10-11.md`. That note covers why people return to software: earned pull, Self-Determination Theory, speed, peak-end. This one covers what is new on a desktop where minds **act on the person's behalf**: how people come to trust, over-trust and under-trust an agent, and how the OS can keep that trust **calibrated**. Calibrated trust is the craving that lasts. "I know exactly when I can let it run" feels better than either worry or blind faith.

Claude wrote this when Pranab asked it to pick up the research ("we need to understand human psychology and craving and appeal").

## 1. The core problem: trust calibration

Decades of human-automation research (Lee & See, *Trust in Automation: Designing for Appropriate Reliance*, 2004) find that people rarely trust a machine at the right level:
- **Over-trust (complacency, automation bias)** (Parasuraman & Manzey, 2010). After a few good results people stop checking, accept a confident wrong answer, and miss the one failure that mattered. Fluent language makes this worse: a mind that *sounds* sure is believed.
- **Under-trust (disuse).** After one visible failure, people stop delegating even what the machine does well, and the product loses its value.

The design goal is not "more trust". It is **trust that tracks reliability**. The Witness is exactly this tool: it separates *Recorded* from *says*, so trust can attach to what was done rather than to how confidently it was said. That is a real advantage over every chat product, and it should be the emotional centre of the experience.

**What follows for the UI:**
- Show **calibration cues, not confidence theatre.** "Recorded: 3 files moved" is a calibration cue. "I've organised everything!" is confidence theatre. The Lens already leads with Recorded; keep a mind's self-praise visibly secondary (Says).
- **Let reliability be seen over time, per mind and per kind of task.** A quiet line in a mind's details ("This week: 41 file tasks, all recorded as asked; 2 web tasks needed you") lets people learn where to delegate. It must be computed from the ledger, never from the mind's own account.
- **Make the rare failure loud and the routine success quiet.** Automation bias thrives when everything looks the same. A refused, stale or unconfirmed act gets its own plain sentence (the four failure sentences in the experience spec). Routine success collapses to a footer.

## 2. The sense of agency: "I did this"

People feel good about outcomes they feel they caused (the sense of agency; Haggard and colleagues). Delegation can quietly erode it: when the machine does everything, the person starts to feel like a spectator in their own work, and that feeling drives people away from tools that work perfectly well.

**What follows:**
- **The person's intent stays visible at the top of every run** ("You asked: organise Downloads"), so the outcome reads as *their* goal achieved, not the machine's initiative.
- **Approvals are moments of authorship, not friction.** "Send this email to Maya?" with the full text makes the send the person's act. That is why approvals are never batched for actions, and why the Approve button never takes focus by default: the person presses it on purpose.
- **"Show me" over "Done".** Opening the result lets the person take it over (Peak-end meets agency).

## 3. Anthropomorphism: warm, never deceptive

People readily treat fluent machines as social beings (Epley, Waytz & Cacioppo, 2007). That can make a mind feel companionable, and it can also manufacture obligation and false intimacy. The UX psychology note already bans flattery and pretend feelings. The AI-specific rules:
- **A mind may be warm in style, never claim experiences it doesn't have.** "Got it" and "you spoke, so I stopped" are fine. "I missed you" is not.
- **Never use social pressure to change behaviour.** No "I'm disappointed you didn't…", and no guilt for declining. "No thanks" is answered with "Understood", and the decline is recorded, never argued with.
- **The bond (companion) changes tone, never loyalty demands.** Its levels may make the voice more casual; they never make the mind needier.

## 4. Learned helplessness versus mastery

When a system fails in ways a person can't understand or fix, they learn not to try. Every failure state must answer three questions, in plain words: **what happened, did anything change, what can I do.** The experience spec's four failure sentences are built on this. The same rule applies to refusals: "the desktop declined that under your current mode", plus the one control that changes it.

## 5. The honest cravings, ranked by how strongly they bring people back

| Craving | The honest mechanism in Yantrik | Where |
|---|---|---|
| **Relief** ("it's handled") | "Nothing needs you", provable from the ledger | Since you left |
| **Mastery** ("I'm good at this") | shortcuts taught at the moment they help; calibrated delegation | Lens, palette |
| **Being understood** | one memory across every mind; the person's own unfinished work resumed | YantrikDB, "you were in the middle of…" |
| **Speed / flow** | first word under 1 s, every press acknowledged within 100 ms | streaming, launch feedback |
| **Control** | Witness, Details, undo, "less of this" on every proactive message | everywhere |
| **Anticipation**, used honestly | an opt-in morning or evening line the person asked for | proactive, opt-in |

Excluded, as in the parent note: variable rewards, streaks, loss framing, social pressure, pretend feelings, badges for things nobody asked to track.

## 6. What this adds to the plan

These come on top of the parent note's ten items:
- **A per-mind reliability line**, from the ledger, in each mind's Details. It needs the ledger (#148), so it comes after phase 1.
- **The person's intent pinned at the top of every run card.** It's small, and it belongs with the Lens work.
- **Proactive messages show why and offer "less of this"; dismissals throttle automatically.** It is grounded today: proactive urges carry a `reason` (`proactive.rs`), and `DeliveryLog::record_outcome` logs dismissals, but `TriggerKind::DismissalSignal` takes no action (`proactive_pipeline.rs:243`). The nagging loop is open.
- **The Bond screen hides its decimal score in Details** (`bond.slint:138` prints "It is 2.37 now"), with a test pinning that the bond never drops for absence. No decay exists today; the test keeps it that way.
