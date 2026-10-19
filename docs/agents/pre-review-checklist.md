# Before you open a PR: the pre-review gate

Work like a careful developer. A reviewer should confirm your work, not discover its problems. The target is one review round; on the 17 Oct 2026 count, six of sixteen PRs needed a second round, almost always because a fix covered the reported path and missed a sibling.

Do these five things before `gh pr create`, and again before you push fixes for a review:

1. **Acceptance check first.** Before you write code, write down what the person will see and how you will prove it works. Build toward that, not toward "it compiles".
2. **Run what CI runs.** `cargo clippy` and `cargo test` for every crate you touched, `tests/app-lints/run.py --app <name>` for an app, `bash -n` and the exec bit on scripts, and the selftests named in `CONTRIBUTING.md`. A PR must not open with red CI. If something can't run in your environment (a missing system library, not enough memory), say which check and why, and name the CI job that covers it.
3. **Answer the checklist below** in the PR description, item by item, with the evidence it asks for: file:line, a test name, a command and its output. "N/A" needs one sentence of reason. When you answer review findings, answer them the same way, and add item 8's grep for every sibling path.
4. **Use the feature.** Drive it the way a person would and attach the evidence: a rendered screen from `tests/ui-preview` (look at the PNG), `yos describe <app>` output, the test log. A button that only logs, or a number that isn't real, is a failure even when every test passes.
5. **A fresh, hostile second look.** Re-read your whole diff as a reviewer who wants to find the hole: every new input, every new path, every new caller. Fix what you find before you push.

The checklist was distilled from the 104 findings in sixteen reviews that returned CHANGES NEEDED (#727, #728, #820, #822, #823, #825, #827, #828, #829, #830, #831, #835, #840, #844, #846, #850). Its items are ordered by how often each kind of problem came back. When a new kind of finding starts to recur, add it here.

## The checklist

Answer each item in the PR description, with the evidence named. "N/A" needs one sentence of reason. Items are ordered by how often the category caused CHANGES NEEDED.

1. **Is every place an agent can act, read or see covered by one shared guard?** (A, 15 findings)
   Question: for each piece of data your feature hides or protects from agents (hidden sheet, protected folder, private capture, another mind's notice), list every action, tool, deferred answer, `describe` field, `list`/`since` variant and file-open path that touches it. Does one function enforce the rule, with the action dispatch going through it and failing closed on unknown names?
   Evidence: the file:line of the single guard and the dispatch that calls it; a test that tries each listed route as an agent and asserts the secret is absent from every reply (like `security_tests.rs` in #727). Also state whether deferred (`answer_later`) code runs with the caller scope captured on the UI thread (#728).

2. **Is identity used for a limit, namespace or ownership bound to a verified credential?** (A, in the 15)
   Question: does anything key a cap, a dedupe space, a feedback queue or "only yours" on a name or id the caller announced itself (attach `id`, display name "an agent")? Could two ids multiply an allowance?
   Evidence: the line that binds id to uid/executable/token; a test with two ids from one uid and a squatter that arrives first.

3. **If a read, parse or write fails halfway, does the code fail closed and finish later?** (B, 14)
   Question: for each persisted file, what happens on EACCES, EIO, parse error, a crash between two writes, and a second run? Does anything commit or remember before the success it depends on? Is any error swallowed (`|| true`, `let _ =`)?
   Evidence: a test per case: unreadable file leaves original bytes intact (`a_sidecar_that_cannot_be_read_is_never_written_over`), killed-then-rerun state converges, retry that fails remembers nothing. Cite the line where state changes happen after success.

4. **Does every write or open of a path that someone else could influence refuse links and non-regular files, and is it created with the right mode?** (C, 11)
   Question: root writes, temp files, notes, screenshots, archives, user-supplied paths: do they use `O_EXCL`/`O_NOFOLLOW`/`mktemp` + `mv -T`, `fstat` on the open fd instead of `exists()` or `metadata()`, and an explicit mode (0600/0700)?
   Evidence: file:line of the open flags or `mktemp`/`mv -fT`; a test that plants a symlink, FIFO and dangling link and shows the target untouched.

5. **Does every loop, recursion, buffer and queue fed by a file, the network or an agent have a size, depth, count, rate or time limit, checked before the work is done?** (D, 10)
   Question: what is the worst input that fits in the allowed file or message size? Is the limit applied while reading (not after parsing)? Is a nested call holding a copy at every level? Is there a timeout on every external process (`stty`, mpv, grim)?
   Evidence: the constants and where they are checked; a test using the review-style worst input (doubling formula, 16,000-deep nesting, 16 MB of newlines, 90k-element array) with a time or peak-memory assertion.

6. **If a file, URL or text is handed to an external tool or a model, can its contents (not its name or extension) make that tool read files, open sockets or follow references?** (E, 8)
   Question: mpv, ffmpeg, Chromium, a tool-call parser, the prompt history. Is the tool started with the narrowest options (`--access-references=no`, `protocol_whitelist=file`, `format_whitelist`, no `file://`)? Does text the model quoted or a provider returned count only as text, never as a call or as the assistant's own history turn?
   Evidence: the argv or config line and a test with a hostile file renamed to look benign (`evil.mp3` with `#EXTM3U`, `sdp.mp3`, fenced `<invoke>`) that checks no connection, no call, no file read.

7. **Does a forwarding, gate or filter rule list what is allowed, rather than what is blocked or capped?** (G, 6)
   Question: when you pass fields on to a shared service, or decide what a text-parsed call may run, is there a positive allowlist of fields and values? Does a value that affects shared state (context size, `keep_alive`, GPU options) get pinned, not just ceilinged?
   Evidence: the allowlist constant; a test that walks the registry or sends unknown fields and asserts each is dropped.

8. **Did you apply the fix, rule and test to every sibling path, config source and caller of the same thing?** (H, 5; the same failure behind most second-round findings)
   Question: `list` and `since`; BIOS and UEFI; `/etc/default/grub` and `grub.d/*.cfg` and `*_custom`; Rust installer and shell installer; person's window and agent surface; first run and resume. Have you grepped for every other reader or writer of the thing you changed?
   Evidence: paste the `grep` command and its hit list, with a line per hit saying "covered by test X" or "not affected because ...".

9. **Is every string an outside party chose (provider error, notice body, URL, file name, caption) cleaned before it is drawn, stored or logged, and are secrets and credentials in URLs stripped?** (K, 3)
   Question: control characters, bidi overrides, zero-width characters, newlines faking a second line, `user:pw@`, `?api-key=`. Is there one shared sanitiser rather than a new one?
   Evidence: file:line of the sanitiser call at each draw/log site; a test with `\u{202e}`, `\x1b[2J`, `https://user:pw@host/?api-key=x`.

10. **Does a malformed input produce a refusal for the whole thing, and is every pattern an exact match?** (L, 3)
    Question: a policy or config file with one bad line, a name check written as a glob or substring (`<invoke` vs `<invoked`, `[0-9]*`). Does a trusted file get checked for owner and mode on the open fd?
    Evidence: a test with one bad line among good ones; the regex/pattern anchors.

11. **Is every new network hop confined and verified?** (M, 2)
    Question: a new `proxy_pass`, fetch or browser load: does it have certificate verification, an egress deny (`IPAddressDeny`, proxy flags), and a fixed host?
    Evidence: the config lines; the `openssl s_client -verify_return_error` output or the unit property listing.

12. **Does each hostile input from the issue have a regression test that fails without your fix, and does it build the state it claims to test?** (J, 4; nearly every fixed finding asked for one)
    Question: negative tests (echoed caller key, still-400 retry, kill mid-run), and a test that was run against the unfixed code. Do the test's setup commands succeed (check the log for errors)?
    Evidence: test names; "fails on <parent sha>" output; list any test you could not run and why (reviewers could not build `yantrik-ui` in several PRs, so name the CI job that covers it).

13. **Do the comments, README, design note, PR text and commit message say exactly what the code now does?** (I, 5)
    Question: mode and ownership comments, "caps" vs "pins", "never reaches X", "as they were". Is a behaviour that surprises a reader (a standing rule covers every caller; CSV formula injection passes through) written down once?
    Evidence: the doc lines you changed; a grep for the old wording.

14. **UI and scope: is every user-visible string, locale file, property and callback you touched consistent, and is the PR limited to its stated causes?** (F, 8, from #844 only)
    Question: renames done in all locales (`ar/de/es/fr/hi/ja/ko/pt/zh`), no unreadable property or dead branch left, placeholders still honest, shared module used rather than a copy, unrelated sweep items split out.
    Evidence: `grep` of the old term across `locales/`; the clippy/slint warnings output; the list of files outside the stated scope.

## Which items a machine can check

| Item | Check | Kind |
|---|---|---|
| 1 | Test that enumerates the surface action table and runs each as an agent against a hidden fixture; fails if an action name is not covered by the guard (fail-closed on unknown). A grep for `agent_is_calling()` inside closures passed to `answer_later`/`finish_later`. | Test + grep; deciding which data is protected needs judgement |
| 2 | None cheaply. A grep for `announced.id`/`harness` used as a map key without a uid or exe check can flag candidates. | Judgement, grep hint |
| 3 | Grep for `|| true`, `let _ =` on rename/write, and `unwrap_or_default()` on a read result. Fault-injection tests (mode 000, EISDIR) per persisted file. | Grep + test |
| 4 | Grep for `.exists()` followed by a write, `File::create`, `OpenOptions::...create(true)` without `create_new`/`mode(`, `fs::read(`/`fs::metadata(` on a user path, and shell `> "$file"` as root. shellcheck for unquoted paths. | Lint/grep |
| 5 | Grep for recursive parse/eval functions without a depth parameter, `read_to_string`/`fs::read` without `take(`, `Vec::with_capacity(user)`. Property or fuzz test with a worst-case input and `ulimit -v` or peak-allocator assertion. CI timeout on the test. | Lint + test |
| 6 | Unit test that asserts the exact argv/flag list of each spawned tool (as `mpv_is_told_never_to_follow_what_a_file_points_at` does). Integration test with the renamed hostile files. Which options are narrow enough needs judgement. | Test |
| 7 | Test that walks the default registry/field list and asserts the accepted set equals a reviewed constant (as `a_written_call_reaches_only_reviewed_tools_of_the_whole_registry`). | Test |
| 8 | `grep -rn` for the changed identifier or option name; a CI script that fails when a file path in a list (installer, `grub.d`, `*_custom`) is read in one place and not another is not realistic. | Grep; judgement for completeness |
| 9 | Grep that every `Text`/`tracing`/`error!` site fed by provider or notice data passes through the shared sanitiser. Test with fixed hostile strings. Grep for `base_url` in display code. | Grep + test |
| 10 | Test with one bad line; clippy `regex` anchors; grep for `case` globs on user data in shell. | Test |
| 11 | Grep for `proxy_pass https` without `proxy_ssl_verify on` in the same location block; `systemd-analyze security` / unit grep for `IPAddressDeny`. | Lint |
| 12 | A pre-merge job that checks the new tests fail on the parent commit (mutation-style) is possible but not present. Cheaper: scan test logs for `error:` lines (a failed `cp` in setup). | Partly automatable |
| 13 | Judgement. A grep for numbers/words in a comment against constants is weak. | Judgement |
| 14 | A script that checks every key in `en.yaml` renamed exists in all nine locale files; Slint compile warnings for unused properties; `tests/app-lints/run.py --app <name>` (0 new). | Lint |

Already automated in this repo (cite the output in the PR): `cargo test -p <crate>`, `cargo clippy -p <crate> --all-targets`, `tests/app-lints/run.py --app <name>`, `bash -n` and exec-bit checks on every script, `yantrik-update selftest`, `yos-mcp-selftest.py`, `yos-selftest.py`. Run `yantrik-update selftest` by absolute path and as a non-root user; reviewers saw 10 channel checks fail in root containers on `main` as well, and that is not your regression.

## Where the checklist comes from

Ordered by number of findings, with PR count as tie-break. Category codes (A, B, ...) are used in the index.

| # | Category | Findings | PRs | Examples |
|---|---|---|---|---|
| A | **Agent authority and identity.** A guard on hidden or protected data, or on who may act, misses one action, path or caller. Includes caller identity that is self-declared or compared by name. | 15 | 7 (727, 728, 823, 828, 830, 831, 840) | #727: `hidden_from_caller` was checked only in `get_range`. `select`, `set_cell` formulas, `save` to the home all still read or copied the hidden sheet. #828: saved screenshots were readable by any mind through `files_open` and `screenshot_mind_view`, bypassing the `screenshot_screen` card. #831: any mind could `dismiss_all` another mind's notices, and harness identity was the self-announced attach `id`. |
| B | **Crash, failure and partial-state handling.** Fails open on a read error, commits before success, never converges after interruption, swallows errors, or leaves temp files or processes behind. | 14 | 6 (822, 825, 827, 828, 829, 831) | #831: an unreadable sidecar was silently overwritten by the next write, losing bindings and settings. The fix then left a "blocked" store still binding ids in memory (two delta rounds). #825/#827: a refused parameter was remembered before the retry succeeded, so one caller could poison shared state. #829: an interrupted GRUB migration never finished. |
| C | **Filesystem safety.** Root or the user writes or opens a path that can be a symlink, FIFO or device. TOCTOU between check and use. Permissions and umask. | 11 | 6 (727, 728, 822, 828, 829, 831) | #829: root did `printf > $note`, which follows a planted link and truncates `/boot/grub/grub.cfg`. #828: `exists()` then grim/wf-recorder with O_TRUNC, files 0644. #727: open follows a FIFO or `/dev/zero` and blocks the window thread. |
| D | **Missing limits.** No size, depth, count, rate, concurrency, timeout or growth cap on something an agent, file or network peer controls. | 10 | 6 (727, 828, 829, 830, 846, 850) | #727: `=A1&A1` doubling used all memory, a 16,000-deep `((((` overflowed the stack, `=SUM(An:An)` recalculation was quadratic, and nested SUMIF cloned the range at every level. #828: no rate limit or runtime cap on mind screenshots and recordings. #850: `stop` array of 90k strings. |
| E | **Untrusted content reaches a powerful interpreter.** Content is treated as code or instruction: file contents sniffed by mpv, quoted tool markup run as a call, a `file://` page shown on a public stream, provider text replayed into model history. | 8 | 4 (728, 827, 830, 840) | #728: a `.mp3` containing `#EXTM3U` made mpv fetch URLs. After the fix, a `.mp3` containing SDP made mpv bind UDP ports. #830: a quoted `<invoke name="run_command">` inside a code fence ran as a real call. #840: a game page on `file://` could display any file `yantrik` can read. |
| F | **UI polish and product consistency** (all from #844). Layout reflow, partial i18n rename, dead ternary, placeholder understates, unconfirmed product call. | 8 | 1 (844) | #844: `Bond` rename done in English only (9 locale files still say "Bond"). Dead `color:` ternary in `mind_panel.slint`. |
| G | **Allowlist vs denylist, clamp vs pin.** A forwarding or gate rule that blocks known-bad things, or caps a value, instead of listing what is allowed. | 6 | 3 (830, 846, 850) | #850: `num_ctx` clamped to a ceiling, but a smaller value reloads the shared model. Other load-time options pass through. #830: a risk gate built on a `from_tool` denylist let 186 of 222 tools through. |
| H | **Same rule missed on a sibling path or config source.** | 5 | 2 (820, 835) | #835: BIOS guard read `/etc/default/grub` but not `grub.d/*.cfg`. A delta then found `/etc/grub.d/*_custom` missed. #820: the text installer still themed legacy BIOS after the Rust installer was fixed. |
| I | **Docs, comments, README or commit message do not match the code.** | 5 | 4 (727, 823, 846, 850) | #727: the comment said "only this user can read" over mode 0o644. #850: README said "caps num_ctx" after the code pinned it. |
| J | **Test gaps.** A hostile case is untested, a test does not build its own setup, or a negative case is missing. | 4 | 4 (823, 825, 829, 835) | #835: the "interrupted theme" test's `cp` failed, so the state it claimed to test was never built. #825: no test for an upstream echoing a caller key. |
| K | **Untrusted or sensitive strings drawn or logged raw.** Control and bidi characters, userinfo in a URL. | 3 | 3 (827, 831, 844) | #827: provider error text with `\u{202e}` or `\x1b[2J` reached chat, IPC and logs. #844: Settings Details printed `base_url` with `user:key@`. |
| L | **Loose parsing or validation.** A malformed input is partly accepted, or a pattern is a glob where an exact match was meant. | 3 | 3 (822, 823, 830) | #823: a file with one bad line still granted its other lines. #830: `<invoke` matched `<invoked`, and `name=` matched inside `data-name=`. |
| M | **Network egress or TLS not confined or verified.** | 2 | 2 (822, 846) | #846: new `proxy_pass https://aig...` without `proxy_ssl_verify on`. #822: headless Chromium loading a Mind-written page ran outside the egress policy. |
| - | Informational or operational notes with no action requested. | 10 | 7 (820, 823, 835, 840, 846 and others) | #823: a shell restart undoes the person's cancel. #835: a serial setup identical to the installer's cannot be told apart (accepted). |

Not seen at all: dead buttons and unconnected callbacks. #844's wiring check found none. CI failures were not review findings. Reviewers did report `yantrik-update selftest` failing in their containers, identical on `main`. Reviewers could not build `yantrik-ui` in three reviews (#828, #831 and #844 reported the build as OOM-killed or missing `libspeechd`). Their UI tests were read, not run.

Six PRs (#727, #728, #830, #831, #835, #850) got a second CHANGES NEEDED on the delta review. In each, the fix covered the reported path and missed a sibling route. See items 7 and 8 below.
