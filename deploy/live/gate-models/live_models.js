// live-gate: what the live instance may ask a cloud provider for (deploy/live/gate-models).
//
// The instance is public and its Mind can be talked into anything, so the gate does not forward
// what it is sent. It reads the request, refuses what is not on this list, and sends the
// provider a new request built from the fields below and nothing else. That is how the
// instance cannot pick a model billed outside the subscription, ask for an unbounded answer, or
// reach a provider-specific header or option; a per-minute rate is not a budget, so each
// provider also has a daily count of requests.

const RULES = {
    // kimi-k3 leads on the live machine: deepseek-v4.1-flash lost its way on a long build there
    // (2 Oct 2026). Flash stays allowed as the chain's fallback on the same provider.
    'ollama-cloud': { models: ['kimi-k3', 'deepseek-v4.1-flash'], daily: 1500 },
    // The subscription Pranab's own Mind calls first: the instance's fallback, and kept small.
    'nanogpt': { models: ['deepseek/deepseek-v4-pro-cheaper'], daily: 100 },
    // NVIDIA NIM: one model, the one the Mind's catalogue defaults to, and a count as small as
    // NanoGPT's: a loop on the live machine must not run down a key that is Pranab's.
    'nim': { models: ['nvidia/nemotron-3-super-120b-a12b'], daily: 100 },
};

// What the Mind's OpenAI-compatible backend sends (yantrik-ml generic_openai.rs). Anything else
// in a request is dropped, not forwarded.
const PASSED = ['messages', 'temperature', 'top_p', 'stop', 'frequency_penalty', 'think',
    'reasoning_effort', 'tools', 'tool_choice'];
const MAX_TOKENS = 8192;
// The local lane's models (AIG, Ollama-native /api/chat): the one the live Mind is set to
// (YM_LOCAL_OLLAMA_MODEL on 561).
const LOCAL_MODELS = ['qwen3.8:27b'];
// AIG also serves Pranab's own Mind's private turns, from the same loaded model. On Ollama any
// load-time option that differs from the loaded runner's (num_ctx, num_gpu, main_gpu, num_batch,
// num_thread, use_mmap, ...) reloads it, and every request's keep_alive resets when it unloads. So
// the instance chooses none of them: the gate sends AIG a request built from these fields only,
// with the context pinned to what the Mind sends (reviews of #850). No keep_alive either: the
// Mind's Ollama backend (yantrik-ml provider/generic_openai.rs, num_ctx 32,768) sends none, so
// AIG's own default decides when the model unloads, as it does for Pranab's Mind. A -1 here
// pinned the model in AIG's memory for good, crowding out the other models AIG serves.
// No `format`: the Mind never sends one, and Ollama compiles a JSON schema into a grammar that
// constrains every token, so a deep or wide one could stall or crash the shared server.
const LOCAL_PASSED = ['messages', 'tools', 'think'];
// `stop` is not here: it is checked after the loop, because every stop string is matched against
// each token for every caller sharing the runner.
const LOCAL_OPTIONS = ['temperature', 'top_p', 'top_k', 'min_p', 'repeat_penalty', 'repeat_last_n',
    'presence_penalty', 'frequency_penalty', 'seed'];
const LOCAL_MAX_STOPS = 8;
const LOCAL_MAX_STOP_LENGTH = 64;
const LOCAL_CONTEXT = 32768;

function refuse(r, status, text) {
    r.headersOut['Content-Type'] = 'text/plain; charset=utf-8';
    r.return(status, text + '\n');
}

async function chat(r) {
    const provider = r.variables.live_provider;
    const rule = RULES[provider];
    if (!rule) return refuse(r, 404, 'no such provider on this gate');

    // In memory, because the location sets the body buffer to its size limit; a body that did not
    // fit would have been refused with 413 before this ran.
    let asked;
    try {
        asked = JSON.parse(r.requestText || '');
    } catch (e) {
        return refuse(r, 400, 'the request body is not JSON');
    }
    if (typeof asked !== 'object' || asked === null || Array.isArray(asked)) {
        return refuse(r, 400, 'the request body is not a JSON object');
    }
    if (!rule.models.includes(asked.model)) {
        return refuse(r, 403, 'this gate serves ' + rule.models.join(', ') + ' on ' + provider);
    }
    if (!Array.isArray(asked.messages) || asked.messages.length === 0) {
        return refuse(r, 400, 'messages must be a non-empty list');
    }
    // Functions the Mind defines, and nothing else: a provider-side tool (a web search, a code
    // runner) is one the provider bills for on top.
    if (asked.tools !== undefined && (!Array.isArray(asked.tools)
        || !asked.tools.every((t) => t && typeof t === 'object' && t.type === 'function'))) {
        return refuse(r, 400, 'tools must be a list of functions');
    }

    const day = new Date().toISOString().slice(0, 10);
    const used = ngx.shared.live_budget.incr(provider + ':' + day, 1, 0);
    if (used > rule.daily) {
        return refuse(r, 429, "today's " + provider + ' requests for the live instance are spent ('
            + rule.daily + ' a day, reset at 00:00 UTC)');
    }

    const wanted = Math.floor(Number(asked.max_tokens));
    const sent = {
        model: asked.model,
        stream: asked.stream === true,
        max_tokens: wanted > 0 ? Math.min(wanted, MAX_TOKENS) : MAX_TOKENS,
    };
    if (sent.stream) sent.stream_options = { include_usage: true };
    // An index loop: the njs engine on the gate does not take for...of.
    for (let i = 0; i < PASSED.length; i++) {
        const field = PASSED[i];
        if (Object.prototype.hasOwnProperty.call(asked, field)) sent[field] = asked[field];
    }

    const reply = await r.subrequest('/_live_upstream/' + provider,
        { method: 'POST', body: JSON.stringify(sent) });

    // A refusal of the gate's key is the gate's problem, not the instance's to read about: the
    // provider's own words stay here.
    if (reply.status === 401 || reply.status === 403) {
        return refuse(r, 502, provider + ' refused the gate (status ' + reply.status + ')');
    }
    if (reply.status === 429 || reply.status >= 500) {
        return refuse(r, reply.status, provider + ' answered ' + reply.status);
    }
    // Anything else that is not an answer (a redirect, a 4xx about the body) is the gate's 502:
    // r.return with a 3xx would read the text as a Location.
    if (reply.status < 200 || reply.status > 299) {
        return refuse(r, 502, provider + ' answered ' + reply.status);
    }
    const type = String(reply.headersOut['Content-Type'] || '');
    r.headersOut['Content-Type'] = type.startsWith('text/event-stream')
        ? 'text/event-stream' : 'application/json';
    r.return(reply.status, reply.responseText);
}

// The local lane (AIG's Ollama-native /api/chat). The Mind always streams, and AIG's stream ends
// with an empty message where the answer was a tool call (seen 17 Oct 2026: the same request
// streamed came back empty, unstreamed it called os_describe), so every agent turn on this lane
// chose no tool. The gate asks for the whole answer and hands it back as a one-line stream, which
// the Mind's reader takes like any other: it reads tool calls from whichever line carries them.
// The request AIG gets is rebuilt from LOCAL_PASSED and LOCAL_OPTIONS, as `chat` rebuilds its own.
async function local(r) {
    let asked;
    try {
        asked = JSON.parse(r.requestText || '');
    } catch (e) {
        return refuse(r, 400, 'the request body is not JSON');
    }
    if (typeof asked !== 'object' || asked === null || Array.isArray(asked)) {
        return refuse(r, 400, 'the request body is not a JSON object');
    }
    if (!LOCAL_MODELS.includes(asked.model)) {
        return refuse(r, 403, 'this gate serves ' + LOCAL_MODELS.join(', ') + ' on the local lane');
    }
    if (asked.tools !== undefined && (!Array.isArray(asked.tools)
        || !asked.tools.every((t) => t && typeof t === 'object' && t.type === 'function'))) {
        return refuse(r, 400, 'tools must be a list of functions');
    }
    const given = (typeof asked.options === 'object' && asked.options !== null && !Array.isArray(asked.options))
        ? asked.options : {};
    // Unstreamed, a long answer holds a gate connection until it is whole, so the answer's length is
    // capped as `chat` caps max_tokens. A shorter answer only shortens the instance's own reply.
    const wanted = Math.floor(Number(given.num_predict));
    const options = {
        num_predict: wanted > 0 ? Math.min(wanted, MAX_TOKENS) : MAX_TOKENS,
        num_ctx: LOCAL_CONTEXT,
    };
    // An index loop: the njs engine on the gate does not take for...of.
    for (let i = 0; i < LOCAL_OPTIONS.length; i++) {
        const key = LOCAL_OPTIONS[i];
        if (Object.prototype.hasOwnProperty.call(given, key)) options[key] = given[key];
    }
    // A few short stop strings, as the Mind sends, or none.
    const stops = typeof given.stop === 'string' ? [given.stop] : given.stop;
    if (Array.isArray(stops) && stops.length > 0 && stops.length <= LOCAL_MAX_STOPS
        && stops.every((s) => typeof s === 'string' && s.length > 0 && s.length <= LOCAL_MAX_STOP_LENGTH)) {
        options.stop = stops;
    }
    const sent = { model: asked.model, stream: false, options: options };
    for (let i = 0; i < LOCAL_PASSED.length; i++) {
        const field = LOCAL_PASSED[i];
        if (Object.prototype.hasOwnProperty.call(asked, field)) sent[field] = asked[field];
    }
    const streamed = asked.stream !== false; // Ollama streams unless told not to

    const reply = await r.subrequest('/_live_upstream/aig', { method: 'POST', body: JSON.stringify(sent) });
    if (reply.status < 200 || reply.status > 299) {
        // AIG's words go to the gate's own error log, never to the instance.
        // Control characters become spaces, so AIG's text can't start a line of its own in the log.
        r.error('local lane: AIG answered ' + reply.status + ': '
            + String(reply.responseText || '').slice(0, 200).replace(/[\x00-\x1f\x7f]/g, ' '));
        return refuse(r, reply.status === 429 || reply.status >= 500 ? reply.status : 502,
            'the local model answered ' + reply.status);
    }
    let answer;
    try {
        answer = JSON.parse(reply.responseText);
    } catch (e) {
        return refuse(r, 502, 'the local model did not answer with JSON');
    }
    r.headersOut['Content-Type'] = streamed ? 'application/x-ndjson' : 'application/json';
    r.return(200, JSON.stringify(answer) + '\n');
}

export default { chat, local };
