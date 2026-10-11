# The Minds panel — accounts, what each has left, and "make something"

29 September 2026. Code: `crates/yantrik-ui/src/accounts/`, `crates/yantrik-ui/src/wire/minds_panel.rs`,
`crates/yantrik-ui-slint/ui/components/minds_panel.slint`. Preview test: `verify-minds`.

## What it is

The mind chip in the status bar opens a panel, in the manner of Omarchy's agents panel:

```
┌ ◉ Minds                       ⚙  + ┐
│   1.9M tokens today                │
│ ✳ Claude                           │
│   Main · Max 20x           ACTIVE  │
│   Today            1.2M tokens     │
│   Account 2 · Max 20x         Use  │
│ ◎ Codex                            │
│   Main · Pro               ACTIVE  │
│   Session ▓▓▓░░░░░░░░      4h 42m  │
│   Weekly  ▓▓▓▓▓▓▓▓▓▓░      4d 12h  │
│ MAKE SOMETHING                     │
│   [ App ]  [ Theme ]  [ Recipe ]   │
└────────────────────────────────────┘
```

- **The accounts** a person has brought: Claude and Codex subscriptions, a Gemini sign-in, a
  Qwen (Alibaba Coding/Token Plan) or xAI key.
- **Which one each vendor answers with.** "Use" points programs started from now on at that
  account.
- **What each plan has left**, from the vendor's own numbers.
- **"+"** lists what would bring a vendor here: Sign in, Add account, Install, or Set up a key.
- **The gear** goes to Settings → Minds.
- **The tiles** open the Lens with "Make me an app that …" and leave it for the person to finish.

## Decisions

1. **The person signs in, with the vendor, in the vendor's program.**
   - Sign in opens `foot --hold` running the vendor's own command: `claude`, `codex login
     --device-auth`, `gemini`.
   - The desktop never receives, copies or holds a credential.
   - Every command is a constant in `accounts/vendors.rs`; a test holds them to plain words.

2. **A second account is a second directory.**
   - It lives at `~/.local/share/yantrik/accounts/<vendor>/account-N` (0700), and the program is
     pointed there with its own variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`).
   - Labels are written by the desktop, never typed, so `accounts.json` names no path a person
     could aim elsewhere. A file that names anything else is ignored whole.

3. **"Use" is for programs started from now on.**
   - The variable goes into every app the shell launches (`session_env`) and into the user
     manager (`systemctl --user set-environment`), where the harnesses run.
   - Using the first account unsets the variable. A program already running keeps its account.

4. **No sign-in file is opened, and no vendor is called with a person's token.** Research on
   29 September 2026 (sources below) found each vendor saying another program must not do this:
   - **Anthropic** (Claude Code legal page): developers "may not collect, store, or intermediate
     Claude.ai credentials or session tokens".
   - **OpenAI** (Codex CI/CD auth guide): run Codex and keep the `auth.json` it refreshes; do not
     "call the refresh API yourself".
   - **Google** (Gemini CLI terms): using its OAuth from third-party software is "grounds for
     suspension".

   So `.credentials.json`, `auth.json` and `oauth_creds.json` are only checked for existence.
   A test in `claude.rs` keeps the file's name out of that module.

5. **What is not known is not drawn.** The panel shows only these numbers:
   - **Codex:** plan and meters from its own session logs. `token_count` events carry
     `rate_limits` (plan type, used percent, window length, reset time) exactly as the server
     reported them.
   - **Claude:** the plan from `.claude.json`, Claude Code's settings file, which holds the
     profile and no secret (`organizationType`, `organizationRateLimitTier`).
   - **Today's tokens:** from each program's own transcripts, each message counted once.

   A meter the vendor's own program does not write down is not estimated.

6. **Qwen is a key.** Qwen Code's OAuth free tier ended on 15 April 2026. What remains is Alibaba's
   Coding Plan and Token Plan, and both restrict use to interactive sessions: "Do not use the
   plan's API key for automated scripts, application backends, or other non-interactive
   scenarios." The desktop shows the key when it is the companion's provider. It must not hand
   such a key to an autonomous agent.

7. **Everything the panel reads was written by something running as the person, so it is read defensively.**
   - **Opening files:** every file is opened with `O_NOFOLLOW`, then checked on the open file itself: a regular file, one name, owned by the person. A link or a hard link to a sign-in file is never read (`logs::open_own`).
   - **Budgets:** each refresh has a byte budget, 16 MB per file and 64 MB in all. A line that never ends is skipped once and never re-read. At most 20,000 directory entries are visited, and at most 200,000 messages are counted per day. `.claude.json` is read again only when it changes.
   - **One refresh at a time:** at most one refresh is ever queued.
   - **Account directories:** each is walked from HOME without following a link. Every part must be the person's own and writable by nobody else. This happens both before a directory is made and before "Use" exports it; one that fails is not exported.
   - **Plan names:** a plan name from a log is shown only if it is one short word.

## Next

- **Claude's Session and Weekly bars.** Claude Code's status line receives `rate_limits.five_hour`
  and `seven_day` (`used_percentage`, `resets_at`), which is the vendor's sanctioned place for
  them. The plan: an opt-in status-line command that writes those two numbers to
  `~/.local/state/yantrik/meters/`, for the panel to read. It only moves while Claude Code is in
  use, and the panel will say how old the numbers are.
- **Codex exact meters without a turn.** `codex app-server` answers `account/rateLimits/read` and
  `account/usage/read`. This is sanctioned: it runs the vendor's own binary.
- **Failover:** when the active account's window is used up, offer the next account with room
  left.
- **Balance keys:** OpenRouter (`/api/v1/key`), DeepSeek (`/user/balance`) and Kimi, all
  documented endpoints called with the person's own key, where the desktop already has one.
- **`describe shell`**: the accounts, their states and meters, for agents to read. It will show
  no directories and no plans an agent doesn't need.

## Sources

- Anthropic: code.claude.com/docs/en/legal-and-compliance, /authentication, /statusline
- OpenAI: learn.chatgpt.com/docs/auth, /auth/ci-cd-auth; openai/codex `codex-rs/app-server-protocol`
- Google: github.com/google-gemini/gemini-cli `docs/resources/tos-privacy.md`
- Alibaba: github.com/QwenLM/qwen-code `docs/users/configuration/auth.md`; Model Studio plan docs
  (28 Sep 2026)
