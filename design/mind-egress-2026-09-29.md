# Where a mind may connect: egress for the mind account

29 September 2026. Status: **step 1 built** (`crates/yantrik-egress`: the proxy, audit and enforce, the control socket);
**step 3 built 5 October 2026** (the kernel holds the mind account to the proxy, in audit and enforce
alike: section 1); **search grants built 5 October 2026** (section 6); the rest for review by Pranab and yantrik-mind-72.
Prompted by NVIDIA OpenShell 0.1.0 (28 Sep 2026): its default-deny egress is the one thing it does
that we don't.

## The gap

`yantrik-mind` cannot read the person's files: `ProtectHome`, `InaccessiblePaths`, its own
account. It reaches the desktop only at the mind door, where every act is graded. But its unit
allows `AF_INET`/`AF_INET6` to anywhere. So a mind, or a plugin or MCP server it was talked into
loading, can send whatever it holds to any host:

- its memory (`mind.db`);
- what the desktop showed it;
- what a person typed to it;

and it can fetch and run anything. The door governs what it can **do** on the desktop. Nothing
governs where it can **send**.

## What we build

### 1. Default deny, by account

nftables, in the same shape as the DevTools guard (`yantrik-update`, `table inet yantrik_devtools`),
matched on the socket's owner. Until this, the proxy was advice: the mind's unit points
`HTTPS_PROXY` at it, and anything running as `yantrik-mind` that ignored the variable (a plugin, a
tool it spawned, a library that connects by itself) reached anywhere directly, past the policy,
enforce and Private mode. Now the kernel enforces it, in audit and in enforce alike (audit is the
proxy's policy, not a hole in the kernel's). As built (`/etc/yantrik/mind-egress.nft`):

```
table inet yantrik_mind_egress {
  counter refused {}  counter dns {}  counter direct {}
  set resolvers4 { type ipv4_addr; … }             set resolvers6 { type ipv6_addr; … }      # audit only
  set loopback4 { type inet_service; 7440, 7450, 8341, … } set loopback6 { type inet_service; … }
  set direct4 { type ipv4_addr . inet_service; … }  set direct6 { type ipv6_addr . inet_service; … }
  chain output {
    type filter hook output priority filter; policy accept;
    meta skuid <yantrik-mind> jump mind
  }
  chain mind {
    oifname "lo" ct direction reply accept                 # answers from the mind's own servers
    # audit:
    meta l4proto { tcp, udp } th dport 53 oifname "lo" counter name "dns" accept
    meta l4proto { tcp, udp } th dport 53 ip daddr @resolvers4 counter name "dns" accept
    meta l4proto { tcp, udp } th dport 53 ip6 daddr @resolvers6 counter name "dns" accept
    # enforce, Private mode, or no readable policy, instead:
    #   meta l4proto tcp th dport 53 counter name "dns" reject with tcp reset
    #   meta l4proto udp th dport 53 counter name "dns" reject with icmpx admin-prohibited
    ip daddr 127.0.0.1 tcp dport @loopback4 accept
    ip6 daddr ::1 tcp dport @loopback6 accept
    fib daddr type local meta l4proto tcp counter name "refused" reject with tcp reset
    fib daddr type local counter name "refused" reject with icmpx admin-prohibited
    ip daddr . tcp dport @direct4 counter name "direct" accept
    ip6 daddr . tcp dport @direct6 counter name "direct" accept
    limit rate 6/minute burst 12 packets log prefix "yantrik-mind-egress refused: " level info
    meta l4proto tcp counter name "refused" reject with tcp reset
    counter name "refused" reject with icmpx admin-prohibited
  }
}
```

- **A positive match and a jump.** A packet with no socket (a kernel reply, IPv6 neighbour
  discovery) has no owner; `meta skuid != N return` would let such a packet fall into the
  refusals for every account on the machine. Only packets the mind account owns enter `mind`.
- **Loopback is a list, not a hole.** On 127.0.0.1 the mind reaches the proxy (7450), the memory
  server (7440) and the bundled llama-server (8341), and the ports the person opened to it with a
  rule `{host: 127.0.0.1 (or ::1), ports: [11434], lan: true}`: a **loopback entry**, a local
  Ollama the usual one. Everything else on this machine is refused (`fib daddr type local`, which
  is loopback and also the machine's own LAN address, routed over `lo`). A local model server is a
  real way out otherwise: Ollama pulls and pushes models by name, so `/api/pull` with
  `evil.tld/<data>/m` carries data past enforce and Private mode, and anything bound to 0.0.0.0
  answers on the LAN address. The Mind's other loopback listeners are off in its unit (`YM_CTL`
  8077, `YM_WEB`, `YM_WEBUI`), its code sandboxes run in network namespaces of their own, and the
  desktop and harnesses are unix sockets. Replies from a server of the mind's own back to whoever
  connected to it on loopback are let through (`ct direction reply`).
- **DNS only in audit.** In audit the mind may ask the resolvers `/etc/resolv.conf` names when the
  table is made (the first three, a zone dropped) and a stub on loopback (systemd-resolved's
  127.0.0.53, dnsmasq), counted. In enforce, in Private mode and when the policy does not read, it
  may not ask at all: the proxy resolves for the mind, and classifies what a name resolves to
  (section 2), so a query has nothing to do but carry data out.
- **The direct set** is the only way past the proxy off this machine, and it comes only from the
  person's policy (section 3), never from anything the mind account can write. An entry is a rule
  with `lan: true` whose host is a **literal address on the local network** (10/8, 172.16/12,
  192.168/16, 100.64/10 which is CGNAT and Tailscale, fc00::/7), one entry per port, TCP only. A
  host name never makes an entry: it keeps going through the proxy, which decides on the address it
  resolves to. A public address never does either: the internet is reached through the proxy,
  where it is counted. Private mode empties the direct set and the loopback entries, and refuses
  DNS, so Private mode leaves the mind its proxy (which refuses everything) and its memory.
- **Who loads it.** Not the proxy, which stays unprivileged (no `CAP_NET_ADMIN`). `yantrik-update
  mind-egress apply`, as root: it asks the proxy's own code for the entries and the mode
  (`yantrik-egress direct`, run as `yantrik-egress` with `setpriv`, so the policy is parsed by the
  code that enforces it and never as root, and root never opens a file in a directory another
  account owns), checks every line again (literal LAN or loopback addresses, no scope ids, ports
  1–65535, at most 64 entries), writes the file whole (beside it, renamed over it) and loads it
  with `nft -f`, one transaction. Three units, written by the updater: `yantrik-mind-egress.service`
  at boot (after nftables, before the network, the proxy and the mind; `PartOf=nftables.service`),
  `yantrik-mind-egress.path`, which runs `yantrik-mind-egress-refresh.service` when `policy.yaml`,
  the `private` marker, `/etc/resolv.conf` or the file it links to changes, and every
  `yantrik-update reconcile`/`apply`.
- **No mind runs unguarded.** `yantrik-mind.service` `Requires=` the boot unit, so a failed load
  keeps the mind from starting and stopping or restarting nftables stops or restarts the mind with
  it; and its `ExecStartPre` checks the table is loaded, so a hand `nft flush ruleset` keeps it
  from starting again. A machine without nft has no mind: that is intended, it fails closed.
- **Fail closed.** A policy that does not read, an answer that does not check, or more than 64
  entries loads the table with **no** entries and no DNS, never the previous ones; the file's
  comment and the journal say why. A table that does not load is replaced by one with no entries
  and no DNS.

Why `skuid` rather than `IPAddressDeny=` in the unit: it covers every process of the account,
including one a harness starts outside the unit's cgroup, and it filters by port as well as
address. It is also written by the updater as root and put back on every update, like the DevTools
guard. Nothing the mind can change. (`RestrictNamespaces=` is not set on the mind's unit: its code
sandboxes are `unshare --user --net`. A network namespace of its own has no way out, and anything
that gives one a way out from the host, like pasta, runs as the account and meets this table.)

**What it does not close:**
- **DNS in audit.** A query under a domain someone controls carries data out to whoever serves it
  (a DNS tunnel), slowly. In audit the mind may still ask; the `dns` counter shows how much.
- **The services it may reach on loopback.** The proxy, the memory server, the llama-server and
  what the person opened. One the person opens is theirs to judge: a local Ollama opened to the
  mind can pull and push models by name again.
- **Direct entries are not counted by the proxy.** What the person allowed by address is reached
  without it; the `direct` counter counts packets, not destinations.
- **A reload of the ruleset.** `systemctl reload nftables` runs `flush ruleset` and then this table's
  own reload: for the 100–300 ms between them the running mind is not held. (A restart or stop of
  nftables stops the mind with it; a hand `nft flush ruleset` leaves it running unheld until the
  next update, boot or nftables reload, and keeps it from starting again.) Including the table in
  `/etc/nftables.conf` would close the gap, at the cost of one broken file taking the whole
  firewall down with it; not done.

### 2. One way out: `yantrik-egress`

A small Rust forward proxy on `127.0.0.1:<proxy>`, running as its own account (`yantrik-egress`),
which the table lets out. The mind's unit sets `HTTPS_PROXY`, `HTTP_PROXY` and `ALL_PROXY` to it.

- **HTTP `CONNECT host:port` only.** No TLS interception: the proxy sees the host name and the
  port, never the traffic. This is a deliberate difference from OpenShell, which terminates TLS to
  inspect requests. Holding every mind's TLS keys would make this machine the thing that reads
  everything; host-level control is enough to stop exfiltration to an unknown host. Plain
  `http://` gets forwarded only for rules that say so (a LAN Ollama).
- **Who is asking.** The proxy asks the kernel through `sock_diag`, the netlink call `ss -p` uses,
  which uid owns the connecting socket. Only `yantrik-mind` is served. Version 2 pins the program:
  the socket's inode leads to a pid and `/proc/<pid>/exe`, and a rule can be limited to a binary
  and its digest, as OpenShell pins on first use.
- **DNS.** The mind need resolve nothing itself; the proxy resolves the name in each `CONNECT`,
  and only once the name may be reached. With Private mode on, or in enforce without a rule, the
  name is refused unresolved: a lookup is itself a message to whoever serves the name. (The kernel
  table lets the account ask the system's resolvers itself in audit only; in enforce and Private
  mode it may not ask at all. See section 1.) Programs that honour proxy variables need no change: curl, Python
  requests/httpx, Node's undici with a proxy agent, and Go all send `CONNECT` by name. A program
  that ignores them fails, which is default deny working.
- **Never a destination.** Loopback, link-local (the cloud metadata address), unspecified,
  multicast, IPv4 hidden in IPv6 forms that lead to those, and every address this machine has.
  A service bound to 0.0.0.0 answers on the machine's LAN address too, and a name that resolves
  to either must not be a way past the loopback guards.
- **The local network, and every address that is not the internet.** Only a rule with
  `lan: true` reaches it, **in every mode, audit included** (5 Oct 2026; audit used to let it
  through, marked). "Not the internet" is the list in `crates/yantrik-egress/private_ranges.json`,
  a copy of the Mind's own `deploy/private_ranges.json`: RFC 1918, CGNAT and Tailscale
  (100.64/10), loopback, link-local, multicast, the documentation and benchmark ranges, NAT64,
  6to4 and Teredo. A name is classified by what it resolves to. So the Mind need not resolve a name
  itself to guard against reaching the LAN when it is behind this proxy, which is what lets the
  kernel refuse it DNS in enforce and Private mode. An audit refusal is a proposal like any other.
- **What it cannot see.** Because it opens no TLS, it cannot see a request whose TLS names a
  different site from the `CONNECT` (domain fronting) on a shared CDN. Host-level policy stops a
  new destination; it does not make an allowed CDN a single site.
- **Plain http** is forwarded one request per connection: strict CRLF, no folded headers, `Host`
  replaced with the decided authority, only a `Content-Length` body (chunked is refused), and
  nothing after it.
- **Private mode.** The shell tells the proxy "private"; every `CONNECT` is then refused. This
  sits beside the freeze and the closed door.

### 3. The policy is the person's

- **Where it lives.** `/var/lib/yantrik-egress/policy.yaml`, owned by the proxy. It is changed only
  through the proxy's control socket, `/run/yantrik-egress/control`. That socket admits the desktop
  owner's uid and root (`SO_PEERCRED`) and nobody else. So the mind cannot widen its own rules, and
  the shell is the only writer.
- **Rules.** Each rule is `host` (exact, or `*.domain`), `ports`, `why`, optionally `http` (plain
  http, not only tunnels) and `lan` (it may resolve to the local network), and in version 2
  `program`:

  ```yaml
  - host: dashscope-intl.aliyuncs.com
    ports: [443]
    why: the Mind's model
  - host: 192.168.4.20
    ports: [11434]
    http: true
    lan: true
    why: Ollama on the home GPU box
  ```

  The second rule is also a **direct entry** (section 1): a literal LAN address with `lan: true`,
  so the kernel lets the mind account reach `192.168.4.20:11434` without the proxy, in audit and
  in enforce. That is what a LAN service needs when a program that reaches it does not honour
  proxy variables. Written as a name (`gpu.lan`), the same rule stays proxy-only. A Settings
  button that lets minds reach a LAN service by address (web search, #656) should write exactly
  such a rule, in either mode. In the same way, `{host: 127.0.0.1, ports: [11434], lan: true}`
  opens a local Ollama to the mind (a **loopback entry**); without it the mind cannot reach one.

- **Seeded rules.** Until the person can write `lan` rules from the desktop, every
  `yantrik-update reconcile` (and `apply`) seeds them for the LAN services this machine is already
  set up to use, so an upgrade does not cut a model on the home GPU box, Home Assistant, a SearXNG
  or a Tailscale peer. Sources are only files the person or root owns, **never the mind's own
  settings** (a mind must not shape its own policy): `/opt/yantrik/config.yaml` (`api_base_url`,
  the providers' `base_url`, the fallback's), the desktop owner's `~/.config/yantrik/settings.yaml`
  (the web search URL) and `providers.yaml`, read as the owner, and root's
  `/etc/yantrik/mind-person.env` (`YM_SEARXNG_URL`, `YM_HA_URL`, `YM_LOCAL_OLLAMA_URL`,
  `YM_OLLAMA_LOCAL_URL`, `YM_NIM_BASE_URL`, `YM_FACE_ML_URL`, `YM_CRITIC_URL`, `YM_WEFT_URL`,
  `YM_IMMICH_URL`). Each URL whose host is not the internet (a literal private address, a name
  under `.local`, `.home.arpa`, `.internal` or `.lan`, or a name that resolves only to private
  addresses, Tailscale's included) becomes `{host, ports: [port], http: scheme == http, lan: true,
  why: "seeded from <source>"}`, marked `seeded`. The proxy's own code makes the plan
  (`yantrik-egress seed-plan`, as its account, with the one list of ranges); root checks it again
  and hands it to the control socket's `seed` op, which only root may use and which replaces the
  seeded rules and never the person's own. A loopback URL is never seeded (opening one is the
  person's call, section 1). The reconcile prints `seeded N lan rule(s): …`. A person who removes
  a seeded rule sees it come back while its source still names it; changing the source is what
  takes it away.
- **The ranges have a twin.** `crates/yantrik-egress/private_ranges.json` is a copy of the Mind's
  `deploy/private_ranges.json`; a test pins the sha256 of both arrays to the Mind's file at
  55842db, so a change on either side fails until both are changed. The direct-entry ranges are
  written twice too (`DIRECT_RANGES` in `direct.rs`, the updater's root check), and the updater's
  selftest holds them equal.

- **Two modes.** The whole policy starts in **audit**: everything is let through, and every
  destination is counted. After a week, Settings → Minds → *Where the Mind connects* shows what it
  reached, and how often, with Allow / Block beside each. Moving to **enforce** is a switch the
  person turns. (A rule has no mode of its own: in an allow-list a rule that only watched would
  let through exactly what one in force does.)

### 4. Asking, instead of silently failing

When the mind connects somewhere no rule allows, the proxy refuses at once with `403` and a body
saying why. It also records a proposal: host, port, count, first and last time, and the program in
version 2. The shell shows the proposal as an ordinary approval card:

> The Mind wants to reach `api.example.com` (port 443) · Allow once · Always · No

A phone can answer the card like any other, since cards reach channels. Only the person's answer
writes a rule. This is OpenShell's Policy Advisor, done through the approvals we already have.
Repeated attempts to one host make one card, not a stream.

### 5. What a mind may rely on

A mind cannot see the kernel's table, so it is told, in a file only root writes:
`/run/yantrik-mind-egress/mind-egress.json`. `yantrik-update mind-egress apply` writes it after a
table has loaded (and only then), at boot from `yantrik-mind-egress.service` (`/run` is tmpfs) and
again from the `.path` refresh when the policy, Private mode or resolv.conf changes. It is written
beside itself and renamed into place, so a link planted at the path is replaced, never followed.

The directory is its own, root:root 0755, made by the writer if missing. It is not `/run/yantrik`:
root services without `XDG_RUNTIME_DIR` keep their sockets there and harden it to 0700
(`yantrik-ipc-transport`'s `socket_dir`), and the mind's account could not traverse it. The writer
refuses a directory that is a link, not owned by uid 0 and group root, or group- or
world-writable, and then writes no file (an old one is removed). Stopping the boot unit removes
the file only; the directory stays.

```json
{"enforced": true, "table": "inet yantrik_mind_egress", "proxy": "http://127.0.0.1:7450",
 "proxy_refuses_private": true, "mode": "audit", "private": false, "dns_allowed": true,
 "lan_hosts": [{"host": "192.168.4.42", "ports": [8888]}, {"host": "homeassistant.local", "ports": [8123]}],
 "loaded_at": 1759600000, "version": 2}
```

- `mode`: `audit` or `enforce`, the policy's; `fallback` when the policy could not be read or its
  answer was refused and the loaded table holds loopback only (`private` and `dns_allowed` false).
- `private`: Private mode is on; the table has no entries and refuses DNS.
- `dns_allowed`: the account may send DNS (audit, not Private). Otherwise every lookup is refused.
- `proxy_refuses_private`: the installed `yantrik-egress capabilities` prints
  `refuses-private-all-modes`, so the proxy refuses private and special ranges without a `lan`
  rule in audit too. Read from the binary at each apply, never assumed; false from an older proxy.
- `lan_hosts`: every host a `lan` rule names, seeded or the person's — names, `*.domain`
  patterns and literal addresses — lowercased, without a trailing dot, sorted, each once with its
  ports. The proxy grants the local network on the **name asked for** (`Policy::decide` matches a
  `lan` rule by the requested host), so a name here reaches whatever it resolves to, a private
  address included. A mind fetching on behalf of anything untrusted (a URL from a web page, a
  tool's argument) must refuse these hosts itself, on any port (`*.domain` covers every name under
  it), and every literal address that is not the internet. Read by the proxy's own code as its
  account (`yantrik-egress snapshot`, which prints the table's entries and these hosts from one
  read of the policy, so the table and this list never describe two policies; `lan-hosts` from a
  proxy older than it), never parsed by root from the YAML; checked again as root.
  It lists the policy, so Private mode does not empty it (the proxy refuses everything then anyway).
  **`null`** when it could not be read (no policy reader, a policy that does not read, an answer
  that did not check) or there are more than 64 hosts: a reader must then take **every name it
  has not resolved itself as possibly the local network** and refuse it on the untrusted path —
  fail closed, never treat `null` as an empty list. `[]` means there are no `lan` rules.
- `loaded_at`: unix seconds of the load. `version`: this layout (2 adds `lan_hosts`); a reader
  refuses one it does not know.

Trust it only if all of these hold, else take egress as not enforced:
- `/run/yantrik-mind-egress` is owned by uid 0 and is not group- or world-writable;
- the file is opened with `O_NOFOLLOW`, and `fstat` on that descriptor (not a second `stat` of the
  path) shows a regular file owned by uid 0 with no group or other write bit;
- it parses, `enforced` is true and `version` is one it knows.

A missing file means **not enforced: fail closed**. It is removed when nothing could be loaded
(and then no mind starts anyway: its unit `Requires=` the boot unit), and when the boot unit
stops (stopping nftables flushes the ruleset, the table with it). When it holds, a mind sends
everything through the proxy and leaves resolving names to it; with `dns_allowed` false it must
not try to resolve anything itself.

### 6. Searching in its own words: the person's grant

5 October 2026. The Mind's egress planner let a web search carry only words from the person's own
messages or files they handed over, which made research impossible. Instead the person grants it,
as other agents are granted things: a **grant** lets one agent use one capability for one run, one
session of its harness, or always. Only `web_search_own_words` exists; anything else is refused by
the writer, the host and every reader.

**Where they live.** Two files, both root's, both written only with `write_file_atomic` (a
`mktemp` beside the file, then `mv -T` over it), so a reader sees the old file or the new one,
and a link planted at a path is replaced, never followed:

- `/run/yantrik-mind-egress/grants.json`: every grant in force, run, session and always. The
  directory is the status file's (section 5), root:root 0755. Gone at reboot.
- `/var/lib/yantrik/mind-grants.json`: the always-list, which outlives a reboot. Directory
  `/var/lib/yantrik`, root:root 0755, file 0644. The writer merges it into the `/run` file at every
  write, and at boot (`yantrik-update mind-egress apply`, from `yantrik-mind-egress.service`) and
  every reconcile.
- `/run/yantrik-mind-egress/run-secrets.json`: for each run grant in force, the SHA-256 of its
  one-time run secret (`{"version": 1, "written_at", "runs": [{"grant", "run", "sha256",
  "expires_at"}]}`), 0644, never the secret. Pruned at every write to the run grants in force. The
  grants file's schema is unchanged.

Each write drops grants that have expired or do not check. The writer refuses a directory that is
a link, not owned by its uid and group (root:root), or group- or world-writable; for `/run` it then
removes the grants file, so a bad directory leaves no grant in force.

```json
{"version": 1, "written_at": 1759650000, "grants": [
  {"id": "g-3fa29c07d1e4", "agent": "mind", "capability": "web_search_own_words",
   "scope": "session", "scope_id": "5f1c2a9e0b7d4c3e", "granted_at": 1759650000,
   "expires_at": 1759736400, "granted_by": "person"},
  {"id": "g-9b0d5e1a7c32", "agent": "mind", "capability": "web_search_own_words",
   "scope": "always", "scope_id": null, "granted_at": 1759600000, "expires_at": null,
   "granted_by": "person"}]}
```

- `id`: `g-` and 12 hex digits; what `revoke` takes.
- `agent`: the harness id, `mind` (the first-party Yantrik Mind) and nothing else for now.
- `scope`: `run`, `session` or `always`. `scope_id`: the run id for `run`; for `session`, the first
  16 hex digits of the SHA-256 of the session string `harness.attach` returned (never the session
  itself, which answers for the harness and this file is world-readable); `null` for `always`.
- `granted_at`: unix seconds. A reader drops a grant dated more than 300 s after its own clock (a
  writer whose clock was ahead would otherwise make it hold until that future date).
- `expires_at`: unix seconds, more than `granted_at` and at most 24 h after it, for `run` and
  `session`; `null` for `always`. A reader drops a grant at or past it.
- `granted_by`: `person` (a tap on the card) for `session` and `always`, `run-starter` for `run`.
- `version`: the integer 1; a reader refuses a file with any other (`true` and `1.0` included). A grant with another key, a capability or
  agent it does not know, or a field out of shape is dropped, never read as something else.

Trust it only if all of these hold, else read it as **no grants** (the Mind asks):
- `/run/yantrik-mind-egress` passes `lstat`: a directory, not a link, owned by uid 0 and gid 0,
  not group- or world-writable;
- the file is opened with `O_NOFOLLOW`, and `fstat` on that descriptor (not a second `stat` of the
  path) shows a regular file owned by uid 0 with no group or other write bit; at most 64 KiB;
- it parses, and `version` is 1.

`yantrik_harness::grants::read` is that reader, with tests.

**Writing both files.** Every rewrite (`add`, `revoke`, `publish`) holds `/run/yantrik-mind-grants.lock`
from its read to its last write, so a `publish` that read a grant cannot write it back after a
`revoke` took it out. A `revoke` writes the `/run` file (what readers believe) first and the
always-list second; an `add` and a `publish` write the always-list first and `/run` second. The
`/run` write is tried twice. The run secrets are written between the two either way: a secret with
no grant in force spends nothing, and an add whose secret could not be kept adds no run grant.

**Who writes.** Root, through `yantrik-update mind-grant add|revoke` (and `publish`). The shell
runs as the person, so it reaches root the way the shell already reaches the updater: the narrow
sudo rule for `$BIN_DIR/yantrik-update` (#397), which re-runs the same root-owned file as root.
sudo names the caller (`SUDO_UID`), and the writer accepts root and the desktop's owner only,
never the `yantrik-mind` or `yantrik-egress` account, whatever else is true. No new setuid binary
and no new socket. The proxy's control socket was not used: the proxy runs as `yantrik-egress`
and cannot write a root-owned file. The mind account has no path to the writer: it is not in the
sudo rule, and nothing it can call passes `mind-grant` to the updater (the control surface runs
the updater only with fixed arguments). **What this does not close:** anything running as the
person can run the updater as the person does, so a mind the person lets run commands as them
(`shell.agent_run`, a sensitive act with its own card) could ask for a grant that way; and every
grant written is in the journal.

**How the Mind asks.** A `grant_request` event on the turn it is answering (docs/harness.md):
`{"kind": "grant_request", "request_id", "capability": "web_search_own_words", "query"}`. It is
looked at only from a `mind` the kernel said, at attach, runs as the `yantrik-mind` account. The
query is shown exactly or refused, each with a reason the Mind can log. The screen is one
self-contained module, `crates/yantrik-harness/src/host/screen.rs`, using only `std`,
`unicode-normalization`, `unicode-properties` and `unicode-script` at pinned versions, so the Mind
can copy it as it is and screen a query before it asks. It refuses:

- characters that draw as nothing or as something else: control, format, separator, private-use,
  unassigned and surrogate characters, variation selectors, tag characters, other spaces and
  default-ignorables, U+2800, U+FFFC, U+FFFD, and the card's quote marks;
- a query that is not its own NFKC form (NFD accents, ligatures, fullwidth, mathematical letters,
  presentation forms);
- enclosing and overlay marks, a mark on nothing, more than 2 nonspacing marks on one letter, the
  same mark twice on one letter, and a dot above `i` or `j`;
- a word starting with `!`, `:` or `<`, which SearXNG and DuckDuckGo read as another engine, a
  language or a timeout: a grant to search the web would otherwise send the query elsewhere;
- a word mixing scripts, more than one script beside Latin in a query (Han with kana, Bopomofo or
  Hangul is one), and a Cyrillic or Greek word made only of Latin lookalikes (a fixed set, not
  the UTS #39 skeleton): each word choice would otherwise be a bit the person cannot see;
- right-to-left letters with digits or Latin letters, which bidi layout draws in another order.

The OS caps the cards, whatever the Mind holds itself to (`grant::may_raise`): one open at a time
per harness, at most 2 per turn, none for the rest of a turn after a No or a typed answer, and at
most 3 per harness per 10 minutes. The window is in memory, so a shell restart resets it. Over a
limit the reply is `refused` and the refusal is journalled. If a grant in
force covers it (this session's, the turn's run's, or always) the reply is `{"granted": {"id",
"scope", "expires_at"}}` and the use is journalled. Otherwise the shell shows the host's own card,
headed in the desktop's name with an accent band no agent's question has, the exact query, and four
answers: **Once** (this query only, nothing stored), **This session**, **Always**, **No**. A
harness `request` offering `Always` or `This session` (case-folded, trimmed, in any order or
company), or whose prompt starts with the card's first line, is refused. The answer comes back on a later
poll as `once`, `session` (with the grant's `scope_id`), `always` or `no`; a typed answer is `no`.
On *This session* and *Always* the shell asks root to write the grant, with the query on the
updater's stdin, never its command line. The model's words are only ever the query on the card:
nothing it sends makes a grant.

**For unattended runs**, the person or root starts the run with a grant that lasts the run:
`yantrik-update mind-grant add --scope run --run-id ID --ttl 4h` (`list`, `revoke ID|all`). That
prints a one-time run secret, `run_secret=<32 hex>` (128 random bits), once, to whoever ran it;
root keeps only its SHA-256 in `run-secrets.json`. Then they start the run: `yos act shell
send_message text=… run=ID run_secret=-`, with the secret on stdin (argv is readable in
`/proc/*/cmdline`), or the same `send_message` action over the control socket. The shell stamps
the run only when the secret hashes to the one kept for a run grant for that run in force
(`grants::run_secret_ok`); the run id alone, readable by every account, spends nothing. `run` is
also refused:

- from a call with an agent token, from the mind account, and from any process an attached
  harness started;
- from any process with a live agent-terminal job (`shell.agent_run`) in its ancestry;
- from a caller whose ancestry the shell cannot read whole. `mind_view::classify` now fails
  closed: an empty walk (the caller exited first), a walk that does not reach a session leader or
  pid 1, or a pid whose start time is not the one read at accept (`PeerCred::started`, so a pid
  reused since is nobody) is not the person.

Same-uid code outside all of these, such as a double fork with `setsid` from an agent's command,
is not told from the person by `/proc`; the secret is what stops it, and that code never saw the
secret unless the person handed it over. The host stamps the run on the turn it hands the harness,
as `turn["run"]`, and remembers it for that turn; a run grant covers a `grant_request` only on a
turn that carries its run. A `grant_request` naming a `run_id` itself is refused: run ids are in
the world-readable `/run` file, and one the Mind could name would make a run grant a Mind-wide
one.

**What binds the approved query to the search: the Mind's own planner, today.** The answer is a
word to the Mind; the egress proxy sees no grants and no queries, so nothing on this machine stops
a Mind that was allowed Q1 from searching Q2. Until the proxy enforces it (#669: a single-use
token bound to `sha256(query)`, spent by `yantrik-egress`), the shell journals every answer and
every use with the query's SHA-256 and the query, escaped, so an audit can compare what was
approved with what was searched.

**Audit.** Root journals every add and revoke (tag `yantrik-mind-grant`: id, agent, capability,
scope, scope id, expiry, who, caller uid, and the query that prompted it, as `sha256=… query="…"`
with every character outside printable ASCII escaped). The shell journals every answer (so a
*Once* is recorded) and every use of a grant in force, each with `sha256=… query="…"`, escaped.

**Settings → Harnesses** lists the grants in force, read with the reader's checks, each with
Revoke.

## What this is not

- **Not a sandbox for the person's own processes.** Agents' terminals run as the person, and
  harness user units (Hermes, Pi) run as the person too. Those need Landlock and seccomp at launch,
  which is a separate design.
- **Not content filtering.** An allowed host gets whatever the mind sends it. Choosing which hosts
  to allow is the control. That is why each rule has a `why`, and why the audit week comes first.
- **Not inference routing.** Which model a mind uses stays the mind's choice. This only decides
  whether it may reach it.

## Questions for yantrik-mind-72

1. **Hosts.** Which hosts does the Mind reach today? Its model provider(s), web search and fetch,
   plugin registries, MCP servers over HTTP, package installs by the coder. The audit week will
   find them, but a starting list means enforce breaks nothing on day one.
2. **Proxy variables.** Does everything in the Mind honour `HTTPS_PROXY`? Any websockets, gRPC or
   raw TCP (IMAP?) that would need `CONNECT` too, or a SOCKS5 front on the same proxy?
3. **127.0.0.1:7440** (the memory server): is it still the only loopback port the Mind needs?

## Order of work

1. **The proxy.** `yantrik-egress`, audit only: `CONNECT`, the `sock_diag` uid check, counting,
   the control socket. The unit gets the proxy variables. No nft yet, so nothing can break.
2. **Settings.** *Where the Mind connects*: the counts, and Allow / Block.
3. **The nft table** in `yantrik-update`: default deny with the proxy as the only way out, beside
   loopback, DNS to the resolvers and the person's direct set. Built 5 October 2026, and on in both
   modes rather than tied to enforce: in audit the proxy lets everything through and counts it,
   which is only true if everything goes through it.
4. **Cards** for proposals.
5. **Version 2:** rules per program, pinned by digest.

A test for each step, and the release gate on VM 520 checks both sides:
- a connect from the mind account to `1.1.1.1:443`, to a loopback port it was not given, and to
  UDP port 53 (anywhere in enforce; anywhere but the resolvers in audit) is refused; one to a
  direct entry is let out; another account is untouched;
- a `CONNECT` to an allowed host succeeds;
- a `CONNECT` to an unlisted host returns 403 and makes one card.
