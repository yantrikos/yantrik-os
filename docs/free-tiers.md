# Free LLM tiers the pool uses

This is the source for `crates/yantrik-ml/src/provider/pool/tiers.rs`. Everything here was read from each provider's own documentation on **2026-09-30**. A limit marked *unpublished* is not invented: the pool learns it from the provider's refusals.

Re-check before trusting any number. Free tiers change often: in 2026 alone GitHub Models was retired, Chutes ended its free tier and Cerebras moved to a trial.

## In the pool

| Provider | Free models (coding) | Limits | Trains on prompts | Headers | Source |
|---|---|---|---|---|---|
| Groq | openai/gpt-oss-120b, qwen/qwen3.8-27b, openai/gpt-oss-20b | per model: 30 RPM, 1,000 RPD, 8K TPM, 200K TPD; org-wide | No (Services Agreement §4.2) | `x-ratelimit-*`, `retry-after` | console.groq.com/docs/rate-limits |
| OpenRouter | nemotron-3-ultra-550b-a55b:free, qwen3.8-27b:free, north-mini-code:free, nemotron-3-super-120b-a12b:free, laguna-s-2.1:free | 20 RPM; 50 RPD, or 1,000 RPD once $10 of credit has been bought (shared across free models, per account) | Depends on each free model's host and the account's privacy toggle | only on 429; remaining quota at `GET /api/v1/key` | openrouter.ai/docs/api-reference/limits |
| Google Gemini | gemini-3.8-flash (1M context) | per project, shown only in AI Studio; RPD resets at midnight Pacific | **Yes**, and humans may read it | none | ai.google.dev/gemini-api/docs/rate-limits, /terms |
| Cloudflare Workers AI | @cf/openai/gpt-oss-120b, @cf/nvidia/nemotron-3-120b-a12b | 10,000 neurons/day (about 150K gpt-oss-120b output tokens), 300 RPM; resets 00:00 UTC | No | none (429 codes 3036/3040) | developers.cloudflare.com/workers-ai/platform/pricing |
| Z.ai | glm-4.7-flash (200K), glm-4.5-flash | concurrency per user tier, unpublished | **Yes** for individuals (terms of use: content may be used to develop and improve its models; only the Team Plan is excluded) | none (body codes 1302/1308) | docs.z.ai/guides/overview/pricing, docs.z.ai/legal-agreement/terms-of-use |
| Mistral | mistral-medium (256K) | Free mode: shown only in the console | **Yes, unless you opt out** | `x-ratelimit-remaining-req-minute` | help.mistral.ai/en/articles/698531 |
| Kilo Gateway | nemotron-3-ultra-550b:free, qwen3.8-27b:free, north-mini-code:free | no key: 200 requests/hour per IP | **Yes** | none | kilo.ai/docs/gateway/usage-and-billing |
| OVHcloud AI Endpoints | gpt-oss-120b | no key: 2 RPM per IP per model | No | `ratelimit-*`, `retry-after` | docs.ovhcloud.com (AI Endpoints capabilities) |

**Model ids to confirm against each provider's `/models` before first use:** Mistral's `mistral-medium-latest`, and the Kilo and OVH ids, were taken from model lists and pages, not from an id table.

## Not in the pool, and why

| Provider | Reason |
|---|---|
| GitHub Models | Retired 2026-07-30. |
| Together, DeepInfra, Chutes | No free tier (prepaid only; Chutes ended free use 2026-03-15). |
| Cerebras | $5 for 30 days after adding a card; no renewing free tier. |
| Cohere | Trial keys are "not permitted to be used for production"; one account; no sharing. |
| Moonshot/Kimi | Paid only ($1 minimum top-up). |
| NVIDIA NIM | API Trial terms: "internal testing and evaluation purposes, not in production". |
| Hugging Face | $0.10 a month of credit. |
| Fireworks | $1 of credit at 10 RPM. |
| SambaNova | 20 RPD per model; its own pages disagree on whether free use still exists. |
| LLM7, Pollinations | Terms forbid proxying; quotas too small or credit-based. |
| OpenCode Zen | Its free tier only works inside OpenCode. |

## One-time grants, not yet in the pool

- **Alibaba Model Studio:** about 1M tokens per model for 90 days, including qwen3-coder-next. Never trains on data.
- **Scaleway:** 1M tokens; needs a card.
- **Vercel AI Gateway:** $5 of credit every 30 days; needs a card; Hobby is non-commercial.

## The rules the pool keeps

- **One account per provider, inside its free limits.** Groq, OpenRouter, Cloudflare, Kilo and others forbid extra accounts for the purpose of stretching a free tier.
- **Never a private turn to a provider that may train on prompts.**
- **Only providers whose terms allow it answer the public.** On this list that is Groq; unclear terms count as no.
- **Your keys, for your own use.** Several providers forbid reselling access or running it as a service for others.
