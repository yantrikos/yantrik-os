# Mirror the person's AI-accounts sharing into the Yantrik Mind (POST /consent)

Pranab, 11 Oct 2026: the Mind follows the OS's private-context rule. A private-grounded turn may go to a cloud provider only after the person agreed to share with it, and one consent is set once. The Mind side is built (yantrik-mind E.PRIV1, `os-shared-memory` 18fe179). This is the shell side. **Base: main after #675 (feat/model-gateway) has merged.** It needs `ai_accounts/consent.rs`.

## The contract (pinned in the Mind's tests; use it exactly)
- `POST /provider`'s neighbour on the person's memory socket: `POST /consent`, sent the same way `provider_handoff/mind.rs` `post()` sends `/provider`. Reuse that code path; do not write a second socket client. Only the person's uid is accepted; anything else gets a 403.
- Body, always a full snapshot:
  `{"source":"yantrik-os","accounts":[{"provider":"ollama-cloud","share":true},{"provider":"nanogpt","share":false}]}`
- `provider`: the catalogue provider id of each **cloud** account the person has. Ids are case-folded; `nvidia-nim` maps to `nim`; the Mind ignores ids and keys it doesn't know. Local and LAN accounts are not sent; the Mind keeps those itself.
- Reply: `{"saved":true,"shared":[…],"restarting":bool}`. A supervised Mind restarts about 0.5 s later.

## What `share` means
- `share: true` only where `consent::sharing_for(account)` resolves to **Share**. Private lane and Off are `false`. A free tier that trains on prompts is never Share (`sharing_for` already says so).
- Several accounts of one provider: `share` is true only if **every** account of that provider resolves to Share; otherwise false.
- Private mode does not change the snapshot. The harness socket already refuses everything while it is on.

## When it is sent (security: only the shell, only on the person's act)
- At the end of onboarding's sharing step (#760's `agree_to_share` / `keep_personal_things_on`), and whenever the person changes Share / Private lane / Off for an account in Settings → AI accounts.
- **Only from the shell process's in-memory consent state**, which the person's own presses changed. **Never** from a file watch, a reload of `ai-accounts.json`, or any value a harness could write: harnesses run as the person and can edit that file (security review of #675, finding 4).
- **Debounce:** each snapshot restarts the Mind, so send once, 1 s after the last change, not per click. Within one attach, skip a snapshot identical to the last one the Mind acknowledged.
- **On every attach of the Mind, send the current snapshot unconditionally**, even if it matches the last acknowledged one. An attach is exactly when the Mind may have lost its state (a reinstall or migrate-minds with a fresh settings file, a restored backup, a hand edit). Deduplicating across attaches would leave a reinstalled Mind refusing every private turn for ever (yantrik-mind-72).
  - **Loop guard:** after a /consent the Mind restarts and re-attaches. Skip that one resend only when the snapshot equals what the Mind's reply said it saved **and** the attach comes within about 10 s of that ack.
- If the Mind's socket isn't there (no Mind installed, or not running), keep the snapshot and send it when the Mind next attaches. Never fail the person's press.

## Also close finding 4 for the gateway with the same baseline
The gateway reads consent through the shell. Hold the person's consent in memory, as `memory_grants.rs` does for grants: a widening (toward Share) that the shell did not make itself is ignored and logged once. Narrowing from the file is always honoured. Follow `memory_grants.rs`'s pattern and tests.

## Tests
- The body for a fixture of accounts is exactly the pinned JSON, with field order and ids.
- Resolution: Share→true; Private lane→false; Off→false; a training free tier with Share set→false; two Groq accounts where one is Off→`groq` false.
- Debounce: three changes in 300 ms give one send; an unchanged snapshot within one attach gives no send.
- Attach: a fresh attach sends even an unchanged snapshot. The re-attach within 10 s of an ack, carrying what the Mind said it saved, does not.
- Only the in-process path sends: editing `ai-accounts.json` on disk to Share sends nothing and does not widen the gateway (the finding-4 test).
- No socket: nothing fails, and the snapshot is sent on the next attach.

## 520 check (from yantrik-mind-72)
On a fresh onboarding where the person agrees to share with the picked cloud provider, the Mind's **first** turn answers rather than refusing. That proves the mirror arrived before the first turn. Then switch that account to Off in Settings → AI accounts. Within about 60 s (a restart is about 0.5 s, plus the Mind's 10–20 s boot on 520 with the memory engine), a private-grounded turn to the Mind stops with the refusal sentence rather than hanging. Also reinstall or migrate the Mind and check that its first private turn after the re-attach answers, which proves the attach resend.

Before it reaches 520: send the head to yantrik-mind-72. It touches the Mind's socket, so it is HELD until mind-72 clears it.

**Ordering (yantrik-mind-72, 11 Oct): this must never reach a machine before a Mind bundle carrying E.PRIV1b (`os-shared-memory` b43a14a).** That bundle is now `yantrik-mind-fc5aa7b` (sha256 `f4784c51…5b64`, 12 Oct); install it on 520 (with Pranab's OK, via mind-3e) before this PR's 520 run. The 6a6006c Mind (on 520 and the Mac) keeps spec-style entries such as `ollama-cloud:deepseek-v4.1-flash` when a snapshot withdraws `ollama-cloud`, so turning sharing Off would not take effect. mind-72 builds that bundle when this PR is ready, so the two land together.

**Test case from 520's real list:** the Mind holds `YM_PRIVATE_PROVIDERS=ollama-cloud:deepseek-v4.1-flash`; a snapshot with `{"provider":"ollama-cloud","share":false}` must leave Ollama Cloud uncleared, and a private-grounded turn then gets the refusal sentence.
