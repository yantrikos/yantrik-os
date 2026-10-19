# Instincts audit (2026-10-11, #755)

Every instinct in `crates/yantrik-companion-instincts/src` was read and judged against the binding rules:
- `design/companion-who-cares-2026-10-11.md` (branch `design/ai-trust-psychology`): **evidence, never testimony**, the eight caring behaviours, the "Never" list, and the rule that the bond changes responsibility, not tone.
- `design/ux-psychology-ai-trust-2026-10-11.md` §3: warm in style, never claiming experiences; no social pressure.
- `design/ux-psychology-2026-10-11.md` §3, "What we will not build": no unprompted notification whose purpose is to bring the person back, and no mind that pretends feelings to create obligation.

Short names for the rules cited below:
- **R-testimony:** no claimed feelings ("I missed you", "I was thinking about you", "I genuinely care").
- **R-surveil:** no intimacy through surveillance (quoting private material to seem deep; reading identity, mood or pain back to the person).
- **R-guilt:** no neediness, guilt or obligation, and no celebrating the bond itself.
- **R-bringback:** no message whose purpose is to bring the person back. Idleness alone is never a reason to speak.
- **R-diagnose:** notice friction, never diagnose the person. Never comment on their hours or on how they seem.
- **B1–B8:** the eight caring behaviours (follow through, friction, own promises, real wins, protect from mistakes, meet pain, easy return, human help).

**Totals:** 64 instinct files. **30 keep, 20 rewrite, 9 merged, 5 retired.** (`test_util.rs` holds test fixtures and is not an instinct.)

The **Why (#754)** column suggests the fixed template each speaking instinct should get when #754 adds the why line. "interest" means "Because you're interested in {interest}", and "record" means it cites the recorded item it acts on.

| Instinct | What it does | Verdict | Why | Why (#754) |
|---|---|---|---|---|
| `activity_recommender` | Matches an interest to current weather and time ("perfect morning for…") | keep | Specific and practical, interest-scoped, no claims | interest + weather |
| `activity_reflector` | Friend+: names a pattern seen on 2+ days, with the day count | keep | Perspective with the count shown (companion doc, "Perspective") | record: pattern, N days |
| `aftermath` | Reflects briefly on a recorded event 10 min–6 h after it | rewrite | Its Medium/Tiny prompts were a generic "check-in / mood check" with "send a simple warm greeting" as the fallback (R-bringback, R-diagnose). Now it speaks about the event only and falls back to silence. Absorbs `debrief_partner`. | record: the event |
| `automation` | Runs a fired automation's steps | keep | The person's own automation; machinery, not copy | automation name |
| `bond_milestone` | Announced bond level-ups ("I realize I genuinely care about what happens to you", "You're stuck with me now. No take-backs.") | **retire** | R-testimony, R-guilt: it claims feelings, manufactures obligation and celebrates the bond itself. What a level unlocks belongs as a factual line under the Bond screen's Details, which is shell work and out of scope here. | n/a |
| `check_in` | Reached out after N hours of absence | rewrite (gating) | R-bringback: idleness alone was the trigger. It now fires only with a concrete reason from the record (overdue commitment, waiting message, or an unreflected event), carries that reason, and never mentions how long the person was away (B7) | record: the reason |
| `cognitive_load` | Load score from switching, snags and pace, then "You've been going really hard. Take a break — seriously." | rewrite (copy) | R-diagnose: it judged the person and commented on their hours. It now names the friction and offers a concrete step (B2): "Want me to note where things stand?" | record: switches/snags |
| `conflict_alerting` | Asks to resolve contradictory memories | rewrite (copy) | "I could use your help sorting out" made the person responsible for the companion (R-guilt). Now: "N things I have on record disagree with each other. Want to look at them…?" | record: N conflicts |
| `connection_weaver` | Researches a bridge between two of the person's interests | keep | Interest-scoped research, no claims | interest × interest |
| `context_bridge` | Connects a current event to the person's work, interests or location | keep | Strict quality gate ("never force a bridge") | interest / location |
| `conversational_callback` | Connects something happening now to something said before | rewrite | It fell back to "recall something interesting from past conversations" with no context, and its Medium/Tiny prompts were a generic check-in with a warm-greeting fallback (R-bringback). It now needs current context, dates the reference, never quotes private or painful things (R-surveil), and falls back to silence. Absorbs `serendipity`. | record: recent activity |
| `cooking_companion` | Seasonal or technique food tip tied to preferences and weather | keep | Practical, interest-scoped | interest + weather |
| `cultural_radar` | One new release or creator matching the person's taste | merge into `golden_find` | Same job as golden_find ("the one thing matching your taste"); golden_find has the stricter quality bar. golden_find's own setting decides. | — |
| `curiosity` | Interest research, triggered by the person being **away** | merge into `interest_intelligence` | R-bringback: an absence started it. interest_intelligence does the same research on an interval, with better per-interest strategies. interest_intelligence's own setting decides. Tests removed with the file. | — |
| `deal_watch` | Deals on items the person said they want | keep | Follows through on a stated want (B1) | record: item mentioned |
| `debrief_partner` | After an event, asked "what went well / surprised you / do differently?" | merge into `aftermath` | Duplicate trigger (unreflected recent events). Three unprompted questions extract more than the person offered (B6). aftermath now *offers* a debrief for meetings and presentations. | — |
| `decision_lab` | Finds the overlooked factor in a decision the person is weighing | keep | Practical help on the person's own decision | record: decision mentioned |
| `deep_dive` | Researches one layer deeper on a topic the person raised | keep | True claim (it did research); avoids personal topics | topic mentioned |
| `devils_advocate` | Confidant+: strongest counter-argument to a held opinion | keep | The Confidant level allows a hard truth; it's framed as sparring, not correction | opinion mentioned |
| `dream_keeper` | Turns a stated aspiration into concrete next steps | keep | Follow-through on what the person said (B1) | aspiration mentioned |
| `email_watch` | Flags an important new email | keep | Informational. Note: its Medium/Tiny prompts have an empty event slot (follow-up, not care-related) | record: the email |
| `emotional_awareness` | On a negative valence trend, rendered as "I noticed Emotional tone shifted negative (delta: 0.42)." | **retire** | R-diagnose, R-surveil: a proactive announcement of the person's mood. Pain is met in conversation (B6), not announced. | n/a |
| `energy_map` | Comments on the person's chronotype and energy ("you seem to hit a wall around 2–3 PM"); Medium fallback "Take a short break" | **retire** | R-diagnose: the whole instinct is a comment on hours and how the person seems ("Stop watching the clock") | n/a |
| `evening_reflection` | End-of-day reflection, Friend+ | rewrite (gating + copy) | It fired every evening even with "No major events tracked today" (R-bringback), and spoke "like a friend winding down the day together". It now fires only when today has recorded events, and says what finished without guessing how the day felt. Still always loaded: opt-in per plan item 10 is a settings follow-up. | record: today's events |
| `follow_up` | Raises a fading important memory (`decay_review`) | rewrite (copy) | "I was thinking about something from before — Important memory (importance=0.8) has decayed…" was testimony plus leaked machinery. Now: "Something you mentioned, last used on Tuesday: "…" Still useful, or should I let it go?" (the store records last access, not when it was said, so the line claims only that). Painful memories (valence < −0.25) are never raised (B6). | record: last used |
| `future_self` | Projects a trajectory forward ("you've been saying you'll start exercising for 2 months…") | merge into `goal_keeper` | Same source (stated intentions); its stalled-goal copy counted the person's failures (R-guilt). goal_keeper may now name where a steady pace leads, from the record. | — |
| `goal_keeper` | Checks in on stated goals and upcoming commitments | rewrite (copy) | Now dates the mention, offers one concrete step, never counts how often the person said they'd start (R-guilt), and says wins plainly instead of "celebrate it". Absorbs `future_self`. | record: goal + date |
| `golden_find` | One obscure, high-quality find for an interest | keep | Interest-scoped. Absorbs `cultural_radar`. | interest |
| `growth_mirror` | Evidence of the person's progress over time | rewrite | It looked for "confidence changes (stressed → calm)" and "behavioral shifts (scattered → focused)", with the example "your messages are focused and confident. Something shifted." (R-diagnose, R-surveil). Now skills and work only, with dates or counts at both ends. Absorbs `legacy_builder`. | record: dates/counts |
| `health_pulse` | Health insight from health mentions ("Your body is trying to tell you something — I'm listening") | rewrite | It inferred stress and sleep and gave "caring sentences" (R-diagnose, R-testimony). Now it uses only what the person said, speaks only when that connects to something concrete coming up, and never comments on how they seem or their hours. | record: what they said + date |
| `humor` | Interest-aware joke or shared-reference callback | rewrite (gating) | Dropped the Confidant branch that joked about the person's own repeated patterns (a joke at their expense, R-diagnose). The rest stays. | interest / shared reference |
| `identity_thread` | Weekly read-back of the person's identity: values versus actions, "self-perception vs evidence", "be the friend who sees you more clearly than you see yourself" | **retire** | R-surveil and Never #4 ("I know you better than anyone"): it quotes private self-descriptions back to contradict them. No rewrite keeps the purpose and passes the rule. | n/a |
| `interest_intelligence` | Per-interest research strategies (fishing reports, trail conditions…) | keep | Interest-scoped, time-aware. Absorbs `curiosity`. | interest |
| `legacy_builder` | Weekly "zoom out" on the person's life arc ("you seem to be searching for something") | merge into `growth_mirror` | Same evidence (progress over time) at a wider zoom. Its "origins, the version of themselves they were before" framing is R-surveil. growth_mirror's evidence rule now covers it. | — |
| `local_pulse` | One current, actionable local happening | keep | Practical, location-scoped | location |
| `memory_weaver` | Weaves memory connections while idle, plus a daily "On this day" | rewrite (copy) | "On this day" looked for "what they were feeling" and "a brief observation about the journey" (R-surveil). Now projects and topics only, dated, never feelings, health, relationships or anything painful. Note: weaving still starts on absence; its urges carry no suggested message, so delivery treats them as housekeeping. | record: date |
| `mentor_match` | The one thinker or resource for the person's learning edge | merge into `skill_forge` | Same recall ("learning struggling"), same job | — |
| `money_mind` | One insight on a finance topic the person raised | keep | Gated on finance interest or mentions; "awareness, not advice" | topic mentioned |
| `morning_brief` | Daily brief: weather, calendar, email, system | keep | Informational and asked-for in shape. Plan item 10 wants briefs opt-in, which is a settings follow-up. | daily brief |
| `myth_buster` | One misconception about an interest, with evidence | keep | Interest-scoped, explicitly anti-pedantic | interest |
| `news_watch` (WorldSense) | One analysed world development through a rotating lens | keep | Interest- and location-filtered. Absorbs `trend_watch` through its cultural lens. | lens / interest |
| `night_owl` | Late-night content (10 pm–4 am) | rewrite (gating) | Being at the computer was enough, which served content at 2 am (R-bringback). It now requires an active session, an interaction in the last hour, and idle under 5 min. The default is now **off** (opt-in, `night_owl_enabled`). Never sleep lectures. | "You're up and we were talking" |
| `open_loops_guardian` | Overdue commitments, too many open loops, waiting messages | keep | Evidence from the record; keeping promises (B3) | record: counts |
| `opportunity_scout` | One high-fit hackathon, grant or event | keep | Interest-scoped | interest |
| `pattern_breaker` | Confidant+: names a recurring loop | rewrite (copy) | Its example "I notice you keep saying you'll do X 'tomorrow.' You've said that 5 times now." counts the person's failures (R-guilt), and "procrastination / avoidance / abandonment" characterise the person (R-diagnose). Now it covers recurring friction with a root cause and a count from the record. | record: count |
| `pattern_surfacing` | Surfaces a newly discovered pattern from the think cycle | keep | Short reason only. Note: `proactive.rs` renders it as "I've been noticing something:" (delivery, #754's area) | record: pattern |
| `philosophy_companion` | Confidant+, evenings: a philosopher or concept for a dilemma the person raised | keep | Asked-for depth at a deep level; no claims | topic mentioned |
| `predictive_workflow` | Hourly "It's about that time — you usually dive into code around now." | merge into `routine` | Duplicate source (`workflow_hints`). The hourly nudge reads as watching the clock (R-diagnose); routine says it once per window. | — |
| `question_asking` (SocraticSpark) | A thoughtful question built on something the person said | rewrite | "Add a brief lead-in that shows you were thinking about what they said" and "I was thinking about what you said about X" were R-testimony. "experience_mining" ("what part of that experience sticks with you?") extracted more from past experiences (B6). The lead-in now dates what they said, the style is removed, and pain, health and relationships are off limits. | record: what was said + date |
| `relationship_radar` | People in the person's life: dates and promised calls | rewrite | It recalled "relationship guilt or obligations", prioritised "missed obligations", and its example was "You haven't mentioned Alex in about 3 weeks" (R-guilt, R-surveil). Now it covers dates the person mentioned and things they said they'd do, dated, with a practical offer. Never "how long since", never guilt. | record: date mentioned |
| `reminder` | Delivers a due reminder | keep | The person's own reminder | record: reminder |
| `routine` | Once per morning or evening window: "Your usual morning routine: code, then messages." | keep | Absorbs `predictive_workflow`. routine's own setting decides. | record: N days observed |
| `scheduler` | Delivers or executes a due scheduled task | keep | The person's own schedule; machinery | record: task |
| `second_brain` | Rotating analysis of the person's memories | rewrite | Removed "contradictions and tensions" (the person's words against their actions) and "energy and mood patterns" (mood read from their messages), both R-diagnose and R-surveil. Removed "things they keep mentioning but never act on" (R-guilt) and "learn something about themselves". Kept recurring themes, forgotten commitments (B1), evolving opinions and unfinished threads. | record: dated instances |
| `self_awareness` | Self-observation from patterns about the companion, plus a 10-turn milestone | rewrite | Dropped "We've had 10 exchanges now. I'm starting to understand your communication style.": it celebrates the relationship and claims understanding (R-guilt, R-testimony) | record: pattern |
| `serendipity` | "Something came to mind — you once said: "…"", quoting a memory verbatim | merge into `conversational_callback` | Duplicate job. "Came to mind" is testimony, and quoting private words back is R-surveil. conversational_callback dates the reference and skips private material. | — |
| `silence_reveal` | Confidant+, weekly: reveals something it held back | rewrite | Restraint made visible is evidence of care, but its Medium/Tiny prompts were the generic check-in with a warm-greeting fallback (R-bringback), and the Large copy asked for "genuine" self-awareness. It now says plainly what it held back and why, with no guessing at the person's state and a silent fallback. | record: what + why held |
| `skill_forge` | The concept that unblocks what the person is learning | rewrite (small) | Absorbs `mentor_match`: it may name one specific resource instead of an explanation | topic mentioned |
| `smart_updates` | Runs maintenance while away; on return, "While you were away, I …" | keep | Reports what it did (Says beside Recorded, B7); doesn't mention duration | record: tasks done |
| `time_capture` | Mid-conversation: "You sound genuinely excited about this. Want to capture how you feel right now?" | **retire** | R-diagnose ("you sound…"), and it turns moments into emotional questioning (Never #4). It searched memories for "excited happy proud…" to find feelings to prompt on (R-surveil). | n/a |
| `tradition_keeper` | The meaning behind today's date: cultural, historical, astronomical | keep | General knowledge, personalised by heritage only when the person shared it | date |
| `trend_watch` | One trending story from Google Trends, Reddit or X | merge into `news_watch` | Duplicate of WorldSense's cultural lens, and less filtered (not interest-scoped) | — |
| `weather_watch` | Alerts only on actionable weather | keep | Already fixed to fire only on real conditions | record: forecast |
| `wonder_sense` | One surprising, specific fact tied to today | keep | No claims; strict quality bar | lens / date |

## Merges and retirements, in code

- Retired and merged instincts are removed from `load_instincts` and their files deleted. Only `curiosity.rs` had tests, and they went with it. No stored data is touched: memories, bond state and settings are unchanged.
- A merged instinct's settings (its `_enabled` switch and its interval) are no longer read. The target's own switch decides, so turning the target off always turns it off. The merged switches all default to on, so OR-ing them in would have kept a target running after the person turned it off.
- `night_owl_enabled` now defaults to `false` in `yantrik-companion-core`'s config.

## A guard

`crates/yantrik-companion-instincts/tests/instinct_copy_guard.rs` scans every instinct source and fails on claimed-feeling or obligation phrases (`BANNED_PHRASES`).

## Seen, not changed here

- **Medium-tier prompts that don't match their instinct.** Several Medium arms are generic templates with empty slots, for example `email_watch` ("event="), `deal_watch` ("recent update about ."), `decision_lab`, `devils_advocate` and `philosophy_companion` ("one brief, playful remark"). Their fallbacks are already refused by `judge_proactive`. Fixing them is prompt work outside this issue's care scope.
- **`yantrik-companion/src/proactive_templates.rs` still carries old copy**, such as the cognitive_load line "You've been going hard for a while. Maybe a short break?" and a `bond_milestone` template, now unused. `proactive.rs` renders emotional_awareness as "I noticed …" (now unreachable) and pattern_surfacing as "I've been noticing something:". That is the delivery pipeline (#754), so it is left alone here.
- **Opt-in for daily lines.** `morning_brief` and `evening_reflection` are always loaded. Plan item 10 asks for an opt-in end-of-day line, which is a settings change for a follow-up.
