# Wanting to come back — the psychology of a desktop people love (2026-10-11)

**The ask.** "We need to understand human psychology and craving and appeal to make our OS UI/UX
behavior superior."

**The answer in one line.** People come back to what makes them feel *capable, in control and
understood*, and what answers them *fast*. They leave what nags them, even when it worked. A
desktop that a mind lives in has unusually strong levers on both sides, so this note separates the
levers we should pull from the ones we should never touch, and turns each into a change in this
shell.

Nothing here is a new setting. Every recommendation names the code or the issue it would change.

---

## 1. Craving, honestly

There are two ways to make something people return to:

| | **Earned pull** (do this) | **Manufactured pull** (never) |
|---|---|---|
| What brings them back | The last session left them better off | An itch the product put there |
| Mechanism | Competence, autonomy, being understood, flow | Variable rewards, streaks, loss framing, FOMO |
| How it feels a week later | "I get more done here" | "Why do I keep opening this?" |
| Risk for an OS | None | The machine the person works on becomes adversarial |

An operating system is not a feed. It is where someone's work, money and private life sit, and it
now hosts a mind that can speak first. The trust it needs is higher than any app's, and a single
manipulative pattern spends it. So the rule for this document: **we design for the pull a person
would endorse if they saw exactly how it worked.** If a pattern only works while it is hidden, it
is out.

That is not a constraint on appeal. Self-Determination Theory, the most robust account of lasting
motivation, says people keep doing what feeds three needs: **competence** (I can do this, and well),
**autonomy** (I chose this, and I can stop), and **relatedness** (someone here understands me). A
desktop with a mind in it can feed all three better than anything else on the machine.

---

## 2. The levers, and where they are in this shell

### 2.1 Speed is the first feeling (the Doherty threshold)

When a response comes in under about 400 ms, people stay in flow and feel the machine keeps up
with them. Past a second, attention leaves, and past ten they assume it is broken. Felt latency
matters more than measured latency: progress that is visible from the first moment beats a faster
response that arrives all at once.

Where we lose it today:
- **#737 / #740**: Hermes's replies arrive all at once because the install leaves gateway streaming
  off. The person stares at a spinner, then a wall of text. Merging #740 is the cheapest appeal win
  on this list.
- **#726**: the browser takes a long time to open "and nothing says it is opening". A click with no
  acknowledgement within 100 ms reads as a missed click. People click again, and get two windows.
- **#682**: the Lens spinner is transient rather than bound to `turn.ended`, so "done" is a guess.
- **Voice (#734 / #745)**: the reply is spoken sentence by sentence while it is still arriving,
  which is right. Keep it that way for every mind: speech that starts within a second of the
  person finishing is what makes voice feel like conversation rather than dictation.

**Do:** acknowledge every press within 100 ms (pressed state, a launching tile in the dock, the
person's bubble in the Lens before any network call); show the first token, not a spinner; never
let a long operation be silent.

### 2.2 Competence: make the person feel good at this

People return to tools that make them feel skilled. Three patterns get there honestly:
- **Visible mastery.** Shortcuts shown at the moment they would have helped ("Super+Space talks to
  Hermes"), once, not as a tour. The 2026-10-01 audit found shortcuts the code advertises but never
  binds; an advertised shortcut that does nothing is a small humiliation.
- **Recoverable action.** Undo beats confirmation dialogs. People explore when mistakes are cheap,
  and exploring is how they get good. Approvals belong to what is genuinely risky; everything else
  should simply be undoable.
- **Progress you can see.** The work strip and work cards show what a mind is doing. That is
  competence by proxy: "my machine is getting this done". It should always say *what* and *how far*,
  never only "working…".

### 2.3 Autonomy: the person is always the one deciding

A desktop whose mind can speak first will be judged entirely by when it does. Unprompted speech is
welcome when it is timely, rare and dismissible, and it breeds resentment otherwise.

What we already do right, and must keep:
- The **Synthesis Gate** holds proactive messages while a conversation is live
  (`wire/notifications.rs`, `conversation_active`). That rule is the single most important piece
  of appeal engineering in the shell. It makes the mind feel polite.
- **Amber means a person's answer is pending, and nothing else** (`Theme.chat-needs-you`). One
  colour with one meaning is how a glance becomes trustworthy. Never spend amber on "something new".
- **Private mode** says plainly that no mind is listening, and gives the way out as the reply's link.
- **Mode changes apply to the next run**, and the chip opens the choice rather than flipping it.

Where to push further:
- Every proactive message should carry its *why* ("You asked me to watch the build") and a one-tap
  *less of this*. The control is the trust.
- Quiet hours and Do Not Disturb should hold the mind too, not only notifications.

### 2.4 Being understood (relatedness, and the bond)

The strongest pull a mind can create is the feeling of being known: it remembered the project, the
name of the client, that the person hates long answers in the morning. This is earned pull at its
best, and it compounds.

The **bond** (`yantrik-companion/src/bond.rs`, Stranger → Partner-in-Crime) is the shell's most
psychologically loaded feature, so it deserves the most care:
- **Keep:** the bond grows from real conversation and changes how the companion sounds (voice rate
  and pitch follow the level). Growth you can *feel* rather than *count* is the honest version.
- **Never:** decay that punishes absence, "don't break your streak", or a level that drops to make
  the person come back. Loss aversion is powerful precisely because it hurts. A relationship that
  threatens you when you are busy is not one anyone endorses.
- **Avoid:** making the score the point. A visible number invites grinding ("say goodnight to farm
  points"). The level name and the change in behaviour are enough; the decimal score can stay in
  Settings for the curious.
- **Do:** let the bond unlock *usefulness* (the mind takes more initiative, remembers more, needs
  fewer confirmations within the person's own rules), never cosmetics or flattery.

### 2.5 Peak–end: people remember the best moment and the last one

People judge an experience by its most intense moment and by how it ended, not by its average. For
a desktop that means:
- **The end of a task is the moment to design.** When a mind finishes long work, the ending should
  be a clear, calm "done, here is what changed" with the result one click away, not a toast that
  vanishes. (#682 binds the Lens's finish to the real `turn.ended`; that is this principle.)
- **The end of a day.** A one-line evening summary the person asked for ("what got done today")
  closes the loop and makes tomorrow's first session start warm. Opt-in only.
- **First boot is the first peak.** Issues like #680 (git has no identity, so the first commit
  fails), #713 (the Mind is one manual step from working) and #712 (Hermes's launcher dies after a
  reboot) are tiny in code and enormous in memory. The first hour decides whether someone stays.
  Fixes for all three are in flight.

### 2.6 Unfinished things pull (the Zeigarnik effect), so use them for the person's goals only

People remember and return to unfinished tasks. The honest use is resumption: when the shell
restarts mid-answer, the Lens says what is being continued (`resume_in_lens`), and that is right.
Extend it so that the desktop opens on "you were in the middle of X" when that is true. Never use
it on something the person did not start ("3 suggestions waiting!").

### 2.7 Choice overload, and defaults that mean something

More choices slow decisions and raise regret (Hick's law, the paradox of choice). For this shell:
- One mind answers by default, and switching is explicit, with a handover so nothing is lost
  (#245). That is right, so keep the picker short and named by what each mind is good at.
- Starters in the empty Lens should be *the person's* likely next actions, drawn from what they
  actually do, not a fixed showcase.

### 2.8 Aesthetic–usability: beautiful things are forgiven

People rate attractive interfaces as easier to use, and forgive their faults longer. The shell's
token discipline and motion are already better than most desktops' (2026-10-01 audit). The
remaining wins are consistency (one bar, one tooltip, one icon set, which this tree is converging
on) and **calm**: few colours that each mean one thing, motion that explains state changes rather
than decorates them, and silence as the default sound.

---

## 3. What we will not build

Each item below would raise short-term engagement, and each spends trust we cannot buy back.

- Streaks, daily rewards, or anything that punishes a day off.
- Variable-reward mechanics: random "surprises", loot-style unlocks, slot-machine refresh.
- Unprompted notifications whose purpose is to bring the person back rather than to tell them
  something they asked to know.
- Red badges counting things the person did not ask to track.
- Confirm-shaming ("No thanks, I like doing things the slow way").
- Hiding how to turn something off, or making off a sequence of screens.
- A mind that flatters to raise the bond, or pretends feelings to create obligation.
- Infinite feeds anywhere in the shell.

Test for any new behaviour: *would the person still want this if the Lens explained exactly why it
happened?* If the honest answer is no, it does not ship.

---

## 4. The plan, in order of appeal per line of code

| # | Change | Principle | Where |
|---|---|---|---|
| 1 | Merge Hermes streaming (#740) | Speed | `harnesses/lib/install/hermes.sh` |
| 2 | Launch feedback within 100 ms: pressed state plus a "launching" dock tile until the window maps; say so if it takes over 3 s | Speed | dock / launch registry (#726) |
| 3 | First-boot fixes (#680, #712, #713) | Peak–end | installer, `yantrik-update` |
| 4 | The Lens's "done" bound to `turn.ended`, with a calm summary card | Peak–end | #682 |
| 5 | Every proactive message carries its *why* and a *less of this* control | Autonomy | proactive stream, notifications |
| 6 | Bond: no decay for absence; hide the decimal score from the main Bond screen | Relatedness, no loss framing | `bond.rs`, Bond screen |
| 7 | Shortcuts taught at the moment they would have helped, once each | Competence | Lens, command palette |
| 8 | "You were in the middle of…" on return, only for the person's own unfinished work | Zeigarnik (honest) | desktop home, `resume_in_lens` |
| 9 | Undo for everything not dangerous; approvals only for what is | Competence, autonomy | file ops, mind actions |
| 10 | Opt-in end-of-day line | Peak–end | proactive engine, opt-in only |

## 5. How we will know

Engagement time is the wrong number: an OS that saves people time *should* see less of it. Measure
locally, never sent anywhere, and only what the person could read:
- **Time to first token** in the Lens and to first spoken word in voice (target: under 1 s).
- **Press-to-acknowledgement** for launches (target: under 100 ms).
- **Proactive messages dismissed versus acted on.** A rising dismissal rate means the mind is
  nagging, so it should speak less, automatically.
- **Turns that end answered** (already counted for the bond in `score_conversation_turn`).
- **Undo use versus confirmation-dialog cancels.** Undo should replace the cancels.

And one qualitative check per release, on the Mac mini: use the desktop for a working day and write
down every moment it felt slow, pushy or confusing. Those notes outrank any metric here.
