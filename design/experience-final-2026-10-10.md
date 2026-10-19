# The experience, finalized: effortless for a newcomer, deep for an expert (2026-10-10)

Pranab's bar: "a novice, someone old and not into tech, as well as a seasoned AI user, can both use it effortlessly and enjoy it."

This is the final pass over everything designed on 10 Oct: the Witness, many minds, the board, onboarding, voice and the shell as windows. DeepSeek V4 Pro, GLM 5.3 and GPT-6 Astra each reviewed the whole flow against that bar (`F:/yantrik/qwen/lead-work/ui-final*`). They agreed on the principle. This doc is what Claude took from them, with the conflicts resolved.

**Read with `design/ux-psychology-2026-10-11.md`** (PR #747, written at Pranab's ask: "understand human psychology and craving and appeal"). It is the why behind this doc. People return for **earned pull**: what leaves them capable, in control and understood, and answers fast. They never return for **manufactured pull**: streaks, variable rewards, guilt-trip buttons, nagging. Its test is binding on every behaviour here: *would the person still want this if the Lens explained exactly why it happened?* Its "What we will not build" list is a hard rule.

## The principle: one sentence on top, proof underneath

Every card, message and row has a **complete plain sentence** a newcomer can act on. Behind it is the **proof**, which is the same object, not a second interface.
- **Reaching the proof:** a small visible **Details** link (so it's discoverable; DeepSeek's risk), or hold or right-click anywhere on the card, or `?` on the keyboard.
- **What the proof holds:** what the OS recorded, exact paths and arguments, which mind, which program, timings and request ids.
- **The Witness is this gesture applied to everything at once.** Holding Witness shows only the proof.
- **The newcomer never has to open Details; the expert never has to switch modes.** There is no "expert mode".
- **The OS draws these cards, never a mind,** so a mind cannot imitate an approval button or a receipt (Astra).

## One vocabulary for everyone

GLM's warning decides this: two vocabularies split support. If grandma's screen says "Just the facts" and her grandson's says "Witness", he can't help her over the phone. So both audiences read the **same plain words**. Internal terms appear only inside Details, beside the plain word, so an expert who is helping can bridge the two at a glance.

| Internal | Everyone reads | Inside Details |
|---|---|---|
| Lens | **Chat** (the panel's placeholder is "Ask anything") | Lens |
| harness / attached mind | **mind**, by name: "Claude", "Hermes", "Yantrik Mind" | harness, program line, origin |
| Witness | button **"What really happened"**; hold to see it | Witness |
| Seen / Says | **"Recorded"** / **"Claude says"** | Seen (kernel record) / Says (claim) |
| threads | **conversations**, named by content ("Website") | threads, lanes |
| private lane | **"Keep personal things on this computer"** | private lane (local model) |
| private context | **"what your minds know about you"** | private context fields |
| attach (a file) | **"Add a file"** | attach |
| approval | a question: **"Send this email to Maya?"** | the action, grade, grant |
| rate limit | **"Claude has reached today's limit. It comes back at 3:00 pm."** | status, retry-after, quota |

**Mind, not helper.** Astra and DeepSeek suggested "helper". The OS's identity is minds ("Yantrik Mind", the minds panel), and "mind" is plain English, so it stays. That is Pranab's call to overrule.

## First run, minute by minute

The times are a walkthrough, never a countdown on screen.
1. **0:00, Welcome.** "Welcome to Yantrik. Make yourself at home. Your files are safe." Two buttons first: **Make text bigger** (it applies live, before anything else) and **Read this to me** (the screen is spoken). Then **Get started**. Hidden: AI words, mind names, shortcuts.
2. **0:30, "Who should do the thinking?"** Named cards, each with one plain line and its honest requirements:
   - "Claude (online, careful with words · needs an Anthropic account)";
   - "On this computer (private, slower · downloads 2.5 GB)".

   Choose one. Then **Connect account**, with browser sign-in where the provider supports it, otherwise **Paste a key**, with **Show me where to find it**. Credentials are never disguised as effortless (Astra). For experts: an **Advanced** link under the cards for a custom endpoint and model id.
3. **1:30, "What your minds share."** Pre-checked: "Your minds will send what they remember about you, and where you are, to Anthropic when they ask it something." **What's shared?** lists it plainly: your name, your town, the app you're in. Below it, the offer: **Keep personal things on this computer instead** (the private lane), which states its size and that answers may be slower. It never silently falls back to a remote model.
4. **2:15, "Getting your minds ready."** Three factual rows per mind: **Can it answer?**, **Can it remember?**, **Can it reach the desktop?**. Each reads *Not checked yet*, then *Checking (started 2:16)*, then *Passed* or *Needs attention: <reason>*. No percentages, no spinners pretending to know. The memory check writes a temporary note and deletes it.
5. **3:15, "Your minds."** One row per mind: "Yantrik Mind: Ready". **Add another mind** (it inherits the model and the memory). **Continue**.
6. **3:45, the first success.** The chat opens **beside** the desktop: "What would you like help with?" One suggestion to tap, "Make a note reminding me to call Sam", plus **Type** · **Talk**. When the note appears it carries a quiet footer, "Recorded: note created · **Show me**". Right there, once: "Want proof of what happened? Hold **What really happened**." The Witness is taught at the moment it means something.
7. **4:30.** "You're ready. The chat is always on the right; the microphone is in it." Done.

**Experts, in the same screens:** the Advanced link, paste-key, Details on every check (raw output, latency, model id), typing into the chat to skip the suggestion, and `?` for the shortcut sheet. They can skip the example, never the sharing decision or the three checks.

## Everyday moments

### Asking by voice for the first time
- The first press of the mic or Super+Space explains once: "Your microphone turns on only when you press this. Speech is understood **on this computer**." That's true here: local Whisper.
- **Listening · Stop.** Words appear in large type as you speak.
- The **first time only**, the transcript is editable before it's sent, with **Send automatically next time**.
- The reply is spoken sentence by sentence as it appears, and both turns are marked **spoken**.
- Talk over it and it stops at once; the transcript says "you spoke, so it stopped", and offers **Finish that thought?** (GLM).
- Experts: hold-to-talk on Super+Space, and Details on a spoken turn shows the raw transcript and timings.

### A mind needs permission
- **Title as a question,** in plain words: "**Send this email to Maya?**" Then the exact content: To, Subject, the full message.
- **One consequence line, only if true:** "Once sent, it can't be unsent." or "Copies were made at 10:42; you can undo this."
- **Approve once** · **No thanks**, and Details. Keyboard focus never starts on Approve. The approval binds to exactly what is shown; a changed plan needs a new card.
- **Details** shows the mind, program line, origin, conversation, exact operation and grade.
- **A decision is a different card:** "Claude recommends the cheaper train · **Go with recommendation**". It picks an option and authorizes nothing (Astra). Never "always allow".

### "What did my minds do while I was away?"
- The header: "**Since you left** · 2 hours".
- Rows in plain sentences, each with its time:
  - **Needs your answer:** "Send Maya the invitation?"
  - **Recorded:** "Created 'Lunch ideas'."
  - **Claude says:** "Compared three restaurants."
- Nothing needs you: "**Your minds worked for 2 hours. Nothing needs you.**" (GLM). It's warm because the OS can prove it.
- Experts: Details on any row shows files touched, diffs, receipts and timings. "Open the full record" goes to the ledger.

### Something went wrong
Each message says what happened, whether anything changed, and what to do:
- "Claude couldn't finish your letter. **Your draft is saved.** [Try again]"
- "We can't reach Claude right now. **Your message hasn't been sent.** [Try again]". If another mind is ready: "Pi is here if you need someone."
- "Claude has reached today's limit. It comes back at 3:00 pm." When the provider gave no time: "It didn't say when it comes back."
- An interrupted action: "**We haven't confirmed whether it finished.** Check before trying again." The OS never blindly repeats an outside action.
- Experts: Details shows the status code, request id and **Copy for support**.

## Small, honest delights
1. **Bigger text, live**, from the first screen, before anything else is asked.
2. **Being heard instantly:** talk over a reply and it stops mid-word, then offers "Finish that thought?".
3. **"Nothing needs you."**, said only when the OS can prove it.
4. **The board's stamp:** a note from a mind lands with "Delivered 10:42", the real recorded time.
5. **Show me:** every "Recorded: created X" opens X directly.

## Hidden until it's needed (from a newcomer's sight, not from the product)
- **Conversations (threads):** one per mind until a second exists; then lanes named by content, never "Thread 2".
- **The routing chip:** only when two or more conversations could fit and the message is ambiguous. Otherwise routing is silent and recorded.
- **Seen and Says marks:** not on every message, which would be noise. A message shows a quiet footer, "Recorded · 3 actions", and the full receipts are in Details or under Witness. This refines the prototype's always-on rail: the rail is the Witness view and the Details view, not the default.
- **Model versions:** "Claude", not a full version string, outside Details.
- **The Agents screen, the ledger and memory settings:** reached from Details or Settings, never pushed at a newcomer.
- **Cold words** become warm and exact: "Decline" becomes "No thanks"; "Error" becomes what happened.

## Risks we accept and watch
- **Recorded is not the same as correct** (Astra). "Recorded: email sent" proves the act, not that the email was right. Receipts say exactly what was observed and nothing more.
- **Discoverability** (DeepSeek). The visible Details link plus a one-time hint on the first right-click, plus the `?` sheet.
- **Vocabulary creep** (GLM). Every new screen's words are checked against the table above before it ships.

## What changes in the work
- **The Witness prototype:** add the first-run and voice moments, and switch the default view to plain messages with a Recorded footer, with the rail under Witness and Details.
- **#734 voice:** the first-time mic explainer, the editable first transcript, and "Finish that thought?".
- **#675 + onboarding:** the first-run screens above, browser sign-in where supported, and the Share / Private lane / Off choice.
- **The approval card** (`control_approvals`): the title as a question, "No thanks", focus never on Approve, the consequence line only when true.
- **Error copy** across the Lens: the four sentences above.
