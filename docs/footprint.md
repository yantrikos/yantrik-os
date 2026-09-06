# What Yantrik costs to run

Measured on 2026-09-06, WSL2 with 31 GB, software renderer, shell plus three autostarted services.

PSS is the number that adds up. RSS charges every process for the full size of pages it shares
with others, so summing RSS over a process tree double-counts; PSS splits each shared page between
its users.

## The total

```
process                     RSS MB    PSS MB
yantrik-ui                   181.2     177.9
a11y-service                   4.8       2.9
weather-service                4.4       2.5
network-service                4.0       2.1
                             -----     -----
TOTAL                        194.4     185.5
```

**About 186 MB** for the whole desktop. For scale, that is in the same range as GNOME Shell alone,
and less than a browser with a few tabs open.

The services are 2–3 MB each. That part is already as small as it is going to get, and it is worth
noticing why: they are separate processes, which usually costs memory, and here it costs almost
nothing because each one does one job and links almost nothing.

## Where the shell's 181 MB goes

| | |
| --- | --- |
| the embedder's weights | ~87 MB |
| the binary's own code, mapped in | ~54 MB |
| everything else | ~21 MB |

"Everything else" is yantrikdb's connection pool, 195 registered tools, 50 skill manifests, the
Slint software renderer's buffers, and three tokio runtimes. Twenty-one megabytes for all of that
is not where the remaining wins are.

## Build profile matters more than anything we wrote

The iteration profile (`--profile fast`) is `opt-level = 0` with debug info, and it is not a fair
measure of the system:

```
                    on disk    .text mapped    shell resident
fast                 322 MB        125.2 MB          230.1 MB
release              110 MB         65.6 MB          181.2 MB
```

Optimising halves the code and takes 49 MB off the running shell — 21% of the whole desktop, for
no change to a line of source. `[profile.release]` carries `lto = "thin"`, `strip = true`, and
`opt-level = 1` for the generated `yantrik-ui-slint` crate, which is glue rather than hot path and
would otherwise cost ~14 GB of RAM in a single rustc invocation.

**Never quote a memory figure from a `fast` build.** It is about a third too high.

## The one remaining lever, and why it is not pulled yet

The embedder is `sentence-transformers/all-MiniLM-L6-v2`, loaded from
`models/embedder/model.safetensors` — 86.7 MB of **f32** weights, hard-coded at
`crates/yantrik-ml/src/embedder.rs:82`:

```rust
VarBuilder::from_mmaped_safetensors(&[&files.weights], DType::F32, &device)
```

At f16 that is ~43 MB; quantised to int8, ~22 MB. Halving it would take another 24% off the whole
desktop, and it is a one-line change.

It has not been made because the change is not really about precision, it is about the vectors
already in the database. Every embedding yantrikdb holds was computed in f32, and vectors computed
at a different precision differ by roughly 1e-3. Cosine similarity almost certainly survives that,
but "almost certainly" is not a thing to discover after a memory search starts returning slightly
different neighbours than it used to. The change wants a re-embed of the existing corpus and a
before-and-after on recall quality, not a drive-by.

Making it *lazy* is the obvious-looking alternative and it does not help: the morning brief runs a
recall five seconds after boot, so the model loads anyway. It would help a headless deployment that
never searches memory, and it would get the desktop painting sooner — but it saves nothing on a
normal session.

## A thing that was quietly costing more than any of this

Measuring turned up 46 orphaned `network-service` processes, the oldest 41 hours old — one per
shell start over two days of testing, all reparented to init and never collected. `stop_all` covers
a clean shutdown; a crash or a `pkill` left them running forever.

Fixed with `PR_SET_PDEATHSIG` in `service_manager.rs`. Verified by killing the shell outright:

```
before start:                        0
while the shell runs:                5
after the shell is KILL-ed outright: 0
```

Worth stating as a general point: the leak was invisible in any single measurement and obvious the
moment the process list was read carefully. Footprint work is mostly reading the process list
carefully.

## Reproducing this

```sh
cargo build --release -p yantrik-ui -p weather-service -p network-service -p a11y-service
# start the shell, wait past the startup brief, then sum PSS from /proc/<pid>/smaps_rollup
```

Two traps in the measurement itself. `pgrep -f <path>` matches the `sudo` or shell that launched a
process as readily as the process, so read the pid from the service rather than searching for it.
And on a machine that has been used for testing, filter to the newest process per name — the rest
are debris from before the fix above.
