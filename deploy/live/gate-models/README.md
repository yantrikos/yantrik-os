# The live instance's cloud models

The live instance (VM 561) is a public machine: its screen is streamed, and its Mind can be talked
into showing anything it has. So it holds no provider key. Its only way to a model is the gate
(VM 560), which checks the instance's own key and adds the real one on the way out.

| Route on the gate (`10.99.0.1:8443`)      | Goes to                      | Rate          | Requests a day | Model allowed                    |
|-------------------------------------------|------------------------------|---------------|----------------|----------------------------------|
| `/api/chat`, `/v1/chat/completions`, …    | AIG (`aig.mycluster.cyou`)   | 2 a second    | no cap         | whatever AIG serves              |
| `/ollama-cloud/v1/chat/completions`       | `ollama.com`                 | 30 a minute   | 1,500          | `kimi-k3`, `deepseek-v4.1-flash` |
| `/nanogpt/api/v1/chat/completions`        | `nano-gpt.com`               | 10 a minute   | 100            | `deepseek/deepseek-v4-pro-cheaper` |
| `/nim/v1/chat/completions`                | `integrate.api.nvidia.com`   | 10 a minute   | 100            | `nvidia/nemotron-3-super-120b-a12b` |

Each cloud route also answers `GET <base>/models` (`/ollama-cloud/v1/models`, `/nanogpt/api/v1/models`,
`/nim/v1/models`). The Mind reads a provider's model list before it will use it, and the gate used to answer
404, so the Mind never used its cloud primary. The answer is a static JSON list of exactly the models
`live_models.js` allows, behind the instance key, with no upstream and no provider key. `models-list.conf` is
generated from `live_models.js` by `node gen-models-list.js`; `setup-models.sh` runs `--check` first and
refuses to install a list that differs. Change the allowlist, run the generator, commit both.

The AIG route is `gate-setup.sh`'s. The three cloud routes are this directory's. On those:
- The request isn't forwarded as sent. `live_models.js` refuses a model that isn't on its list,
  caps `max_tokens` at 8,192, counts the day's requests, and sends the provider a new body built
  from known fields only.
- No headers cross in either direction. The instance can't set a provider option, and it never
  sees the account's usage headers.
- When a provider refuses the gate's key, the instance gets a 502 and a one-line reason, never the
  provider's reply.
- The provider's certificate is verified. nginx doesn't do that by default.
- The rates and daily counts are per gate, not per caller address. Otherwise the instance could add
  addresses of its own on the segment and get a limit for each.

NanoGPT gets the lowest numbers on purpose. It is the subscription Pranab's own Mind calls first,
and a live machine stuck in a loop must not spend that week's tokens.

## Setting it up

On node2 as root, with this directory and `../guest.sh` copied there:

```sh
# 1. The routes, with the keys on stdin. They go into the gate as root-only files and are
#    never written on node2 or printed.
grep -E '^(OLLAMA_CLOUD_KEY|NANOGPT_KEY|NIM_KEY)=' keys.env | ssh root@node2 'cd /root/live-setup/gate-models && sh setup-models.sh'

# 2. The instance's Mind, pointed at them. This restarts the Mind service, not the desktop.
ssh root@node2 'cd /root/live-setup/gate-models && sh point-mind.sh'
```

`NIM_KEY` is the `nvapi-` key from build.nvidia.com: `nvapi-` and about 64 base64url characters.
`setup-models.sh` refuses a `NIM_KEY=` line of any other shape, loudly, and so for the other two keys. Every key is
optional on a run: a provider left out keeps the key the gate has, and one the gate never had gets a placeholder
(its route answers 502) so rotating one key never needs another. `NGC_API_KEY` is not read; use a key made for
build.nvidia.com only. As with the others it arrives on stdin, goes into
`/etc/live-gate/nim.auth` (root, 600) and is on no command line.

Run step 1 again to change a key, and again after any run of `gate-setup.sh`, which rewrites
the server block without this directory's include. If nginx refuses a run, step 1 puts back
everything it touched. Step 2 keeps the Mind's previous settings as
`yantrik-mind.env.before-cloud`. It edits them as the Mind's own account, never as root.

## What the Mind ends up with

1. Ollama Cloud, kimi-k3, with deepseek-v4.1-flash allowed on the same route as a fallback. (deepseek-v4.1-flash
   scored 8 of 9 on the desktop task battery at about 3 s a call.)
2. NanoGPT, when Ollama Cloud refuses or fails.
3. AIG, as the survival fallback.

NVIDIA NIM is routed and the Mind is given its address and key, but it is not in the chain until chosen.
These names were verified in the Mind's own code (`mind-inference`): the provider id is `nim` (alias `nvidia`),
the key env var is `NVIDIA_API_KEY`, the base-URL override is `YM_PROVIDER_BASE_URL_NIM`, and the default base is
`https://integrate.api.nvidia.com/v1`. The value written for `NVIDIA_API_KEY` is the instance key, good only on the
gate. `NVIDIA_API_KEY` is a name other NVIDIA tools on that machine also read; it cannot be renamed because it is
the one the Mind reads, and the value is worthless anywhere but the gate. To make NIM
the lead, change the one line `mind-env.sh` writes, `YM_PRIMARY_BRAIN`, to
`nim:nvidia/nemotron-3-super-120b-a12b` (it supports tools) and run step 2. That model is the
only one the gate lets through on this route.

Private turns still go to AIG only, and fail closed when it is down. They never fall through to a
cloud.

AIG's model (bonsai2-27b) no longer leads. On this machine it could not fill a tool's named
parameters.
