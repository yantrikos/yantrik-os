# Security review: PR #577 (live/gate-nim), NVIDIA NIM route on the live gate

Scope: `git diff origin/main...origin/live/gate-nim` (deploy/live/gate-models/, 7 files) plus the unchanged
neighbours (live_models.js, routes.conf, setup-models.sh, pick-keys.py, mind-env.sh, point-mind.sh, README.md,
../guest.sh, ../relay/). Static review only; no VM access. `sh -n` passes on setup-models.sh, mind-env.sh and
point-mind.sh; `py_compile` passes on pick-keys.py; `node --check` on live_models.js is clean (it is an ES module
with `export default`, njs-specific features not exercised; the njs engine itself was not available).

## Verdict: MERGE AFTER FIXES

No path was found by which 561 can read or exfiltrate the NIM key. The NIM route is a faithful copy of the
nanogpt route, and the hardening that makes that route safe carries over. The findings are about operability
(a new hard requirement that blocks key rotation) and about two things in the Mind's settings that cannot be
verified from this repo. None is a key leak. The MEDIUM items should be settled before the first live run.

## What was checked and held (no finding)

1. Key exfiltration. `live_models.js:81-100` returns only: a fixed text body for refusals; for 2xx, the upstream
   `responseText` with Content-Type reduced to one of two constants. No upstream header (Set-Cookie, Location,
   x-*, usage headers) is copied. 401/403 become a generic 502; 4xx/3xx become a generic 502 (so no Location);
   429/5xx pass the status only. The upstream request is built with `proxy_pass_request_headers off`, a fixed
   Host, and the Authorization header only from the root-600 `nim.auth` (routes.conf:87-103). `/_live_upstream/nim`
   is `internal`, and `location = ` exact matches mean no other path, prefix, query or method reaches
   `js_content` (`limit_except POST`). `$live_provider` is set statically per location, so the subrequest URI
   `'/_live_upstream/' + provider` is not user-influenced. No SSRF: the upstream host is a literal in
   `proxy_pass`. TLS to NVIDIA is verified (`proxy_ssl_verify on`, name pinned). The body is re-serialised from
   an allowlist (`PASSED`), so smuggling or header injection via the request body is not possible; duplicate JSON
   keys resolve before the rebuild. The key reaches NVIDIA, which does not echo Authorization; nothing on the
   gate can reflect it.
2. Config injection. NIM regex `^NIM_KEY=(nvapi-[A-Za-z0-9_-]{40,90})$` (setup-models.sh:126) is anchored and
   has no quote, `;`, `$`, `\`, `{` or whitespace; `\r` is stripped first; `head -n 1` takes one line. The
   instance key is checked as hex on the gate before awk substitutes it (setup-models.sh:173-176), so `&` or
   regex metacharacters in the awk replacement are excluded.
3. Key handling. `printf` is a shell builtin; the key is on stdin only (ssh stdin -> qm `--pass-stdin`), never
   in argv or `ps`. Files are written with `umask 077` to `*.new` then `mv` (atomic, 600). The backup dir is 700
   and `cp -p` keeps modes. Nothing is written on node2.
4. Allowlist and limits. Exact-match `includes()` on one model; daily counter is incremented before the call,
   keyed by provider and UTC day, in a shared zone persisted to disk; `max_tokens` capped at 8192; tools limited
   to `type: function`. The 401 check runs before `limit_req`, so unauthenticated traffic cannot spend the
   budget. No route is reachable without the instance key.
5. Differences from nanogpt: none that weaken anything in routes.conf, rates (10/min, burst 5, 100/day) or
   live_models.js.
6. The Mind's settings contain the instance key under `NVIDIA_API_KEY` (mind-env.sh:185), not the NIM key, and
   nothing else new. It is the same value already present as `YM_LOCAL_OLLAMA_KEY`, `OLLAMA_CLOUD_KEY`,
   `NANOGPT_KEY`.

## Findings

### MEDIUM-1: setup-models.sh now hard-requires a NIM key, which blocks rotating the other keys
`setup-models.sh:140` (`[ -n "$nim" ] || keep nim`) and `keep()` at :133-137.
Scenario: the gate has no `/etc/live-gate/nim.auth` yet. Pranab runs the documented step 1 with only
`OLLAMA_CLOUD_KEY=` (a rotation after a leak) or `pick-keys.py` on a keys file that has no NIM entry. The script
exits 1 with "no nim key on stdin, and the gate has none to keep". Nothing is changed (the check runs before any
write, so this does not half-apply), but the other providers' keys cannot be rotated until a valid NIM key is
supplied, and the first-time NIM key has to be in hand. The same happens if the NIM key is present but malformed
(see LOW-1). Also, point-mind.sh/mind-env.sh already hand the Mind a NIM route that is "not in the chain until
chosen", so a NIM key is optional by design.
Fix: make `nim` optional. When there is no key and no `nim.auth`, skip creating the NIM location (or write a stub
`nim.auth` containing `proxy_set_header Authorization "Bearer unconfigured";`, giving upstream 401 -> the gate's
502). Do the same for nanogpt/ollama in a follow-up, so the first run cannot lock anything out.

### MEDIUM-2: the Mind-side names are not verifiable from this repo and a mismatch sends the instance key to NVIDIA
`mind-env.sh:182,185`, README "YM_PRIMARY_BRAIN=nim:...".
In `crates/yantrik-ml/src/provider/catalogue.rs:206-215` the provider id is `nvidia-nim` and its `key_env` is
empty; the repo does not contain the code that reads `YM_PROVIDER_BASE_URL_NIM`, `NVIDIA_API_KEY` or the `nim:`
brain prefix (only `deploy/yantrik-os/yantrik-mind-launch` mentions the YM_ variables). Scenario: the Mind
resolves the provider as `nvidia-nim`, ignores `YM_PROVIDER_BASE_URL_NIM`, falls back to the catalogue default
`https://integrate.api.nvidia.com/v1`, and sends `NVIDIA_API_KEY` (the instance key) straight to NVIDIA. That
leaks the instance key to a third party, and the route does not work. The real NIM key is not at risk, but the
instance key is the credential for every gate route. Today it is dormant (the primary brain is ollama-cloud), so
this bites when someone switches `YM_PRIMARY_BRAIN`.
Fix: confirm the exact env names and provider id in the Mind source; use the id the Mind actually parses
(possibly `nvidia-nim:`), and add a check on the live VM that outgoing NIM traffic goes to the gate address
before the README tells anyone to make NIM the lead.

### LOW-1: a malformed NIM_KEY is silently ignored when the gate already has one
`setup-models.sh:126,140`. A key that fails the regex (a typo, 91+ chars, a different NVIDIA key format, an
`NGC_API_KEY` that is not `nvapi-`) yields an empty `$nim`; `keep nim` then prints "nim: keeping the key the gate
has" and the run succeeds. Pranab believes the key was rotated. The same is true of the older keys (pre-existing).
Fix: if a `NIM_KEY=` line is present in the input but the regex yields nothing, exit with an error naming the
provider (never the value).

### LOW-2: pick-keys.py alias `NGC_API_KEY` can pick a key for a broader credential
`pick-keys.py:21`. NGC personal keys can carry scopes beyond the NIM API (registry, org). If the file has
`NGC_API_KEY` of `nvapi-` shape and no `NIM_KEY`/`NVIDIA_API_KEY`, that key is installed on the gate. It stays on
the gate, but a broader key is a larger loss if the gate is ever compromised.
Fix: drop `NGC_API_KEY` from the aliases and require a key created for build.nvidia.com / the NIM API only.

### LOW-3: `NVIDIA_API_KEY` is a well-known env var name in 561's Mind settings
`mind-env.sh:185`. Other SDKs or tools on 561 that read `NVIDIA_API_KEY` by convention (LangChain NVIDIA, the
OpenAI-compatible NIM clients) would send the instance key to `api.nvidia.com` if pointed there by a prompt-injected
Mind. The instance key is only good on the gate (LAN-only to 561), so impact is low, but it is a credential leaving
the box. Consider a dedicated name if the Mind allows one.

### LOW-4: pre-existing, carried over by the new file
- `setup-models.sh:153`: the backup dir (including the previous provider keys) is removed and rebuilt per run and
  is never cleaned afterwards, so a rotated-out key (a leaked one, say) stays in `/etc/live-gate/before-cloud/`
  (700, root) after rotation. Delete it once nginx accepts the config, or on a later success.
- `setup-models.sh:164-167,171-176`: a failure between the `.auth` writes and the nginx test (guest agent
  timeout, awk failure) exits via `set -e` without the rollback, leaving a new key with an old route. Wrap the
  rollback in a `trap` that runs on any non-zero exit after the backup.
- `setup-models.sh:203` does `reload nginx` with no check that it succeeded or that `nginx -t` passed after the
  final write; a failed reload leaves the file state changed.
- `mind-env.sh:170`: `cp "$f" "$f.before-cloud"` overwrites the backup on every run, so a second run destroys the
  pre-cloud settings the README says to restore from. Only copy if the backup does not exist.
- `live_models.js:62`: the daily count is incremented before upstream success, so a failing upstream or an
  allowed-model flood from 561 burns the 100/day (a self-DoS of its own NIM lane only; accepted by design, noted).
- nginx `if ($http_authorization != "Bearer ...")` is not constant-time; irrelevant while the only holder of the
  instance key is 561.

## Summary of fixes before merge
1. Make the NIM key optional in setup-models.sh (MEDIUM-1).
2. Confirm the Mind's provider id and env names for NIM, and correct mind-env.sh and the README (MEDIUM-2).
3. Fail loudly on a malformed key line (LOW-1) and drop the `NGC_API_KEY` alias (LOW-2).
