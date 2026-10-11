# Where a mind may connect: egress for the mind account

29 September 2026. Status: **step 1 built** (`crates/yantrik-egress`: the proxy, audit and enforce, the control socket);
**step 3 built 5 October 2026** (the kernel holds the mind account to the proxy, in audit and enforce
alike: section 1); the rest for review by Pranab and yantrik-mind-72.
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

### Two doors

(5 Oct 2026.) The proxy listens on two loopback ports, each set in its unit the same way:

| door | address | variable | `lan` rules |
|---|---|---|---|
| **endpoint door** | `127.0.0.1:7450` | `EGRESS_LISTEN` | honoured, as above |
| **public door** | `127.0.0.1:7451` | `EGRESS_PUBLIC_LISTEN` | never |

The public door is **pinned**. `yantrik-egress` refuses to start with `EGRESS_PUBLIC_LISTEN` set to
anything but `127.0.0.1:7451` (`PUBLIC_DOOR` in `main.rs`), because the kernel table's loopback
set and the status file's `public_proxy` name that port (`EGRESS_PUBLIC_DOOR` in `yantrik-update`,
from which both are made). A drop-in that moved it would otherwise send untrusted fetches to a
port the proxy does not serve, and to whatever else bound it. The selftest checks that the
updater, the unit and the proxy's pin agree, and that the binary refuses another address.

**Why.** The proxy grants the local network on the *name asked for*: a `lan` rule for
`gpu.example.ts.net:11434` lets any request for that name through. Security reviews found
untrusted fetches riding those rules. yt-dlp and ffmpeg follow redirects and HLS segment URLs the
Mind never sees, through this proxy, so a public playlist that lists
`http://gpu.example.ts.net:11434/...` reached the LAN service with a blind GET. Parser
differentials in the Mind's own checks did the same twice. The Mind cannot guard hops it never
sees, so the fix is at the OS: only the Mind's own endpoint clients get the door that honours
`lan` rules.

**The public door.** Same code as the endpoint door: the head, `CONNECT` and absolute-form
`http://`, the limits (one shared count of open connections), Private mode, audit and enforce,
the ledger, the refusal of addresses that are never a destination. On top of that:

- a host that any `lan` rule names, exactly or by `*.domain`, is refused **on every port**, before
  it is looked up. A `lan` rule can never let a request through this door. The rule's host is
  read as the request's is (lowercase, no trailing dot), and an address rule is compared as an
  address, so `2001:470:0::1` names `[2001:470::1]` and `203.0.113.9` names
  `[::ffff:203.0.113.9]`. A new rule cannot be written with a trailing dot; a `policy.yaml` that
  already has one is read with the dot taken off (see section 3).
- what the name resolves to must be the internet. The local network, every range in
  `private_ranges.json` (CGNAT and Tailscale, ULA, NAT64, 6to4, Teredo, the documentation
  ranges, …), loopback, link-local, this machine's own addresses, and IPv4 written as IPv6
  (`::ffff:a.b.c.d`) are all refused, in audit too. It is the same classifier
  (`policy::place_of`), not a copy. A name that resolves to both kinds is reached at its internet
  addresses only, as on the endpoint door.

Every request's log line names the door that served it (`door=endpoint|public`).

**Which Mind callers use which.**

- **Public door (7451): anything fetched for someone else.** yt-dlp; ffmpeg; the browser and
  `net_guard`; the fetch tool; fetches of search results; image fetches; paper fetches. Set their
  `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY` (or the client's proxy option) to
  `http://127.0.0.1:7451`, and pass the same to every child process they start.
- **Endpoint door (7450): the Mind's own configured endpoints only.** Its model servers (Ollama,
  llama-server, a provider API), Home Assistant, its SearXNG instance's *query* (not the results'
  pages), and other endpoints from its own configuration. This is the unit's default
  `HTTPS_PROXY`, so a caller that is not moved keeps working as before.

When in doubt, use the public door: a request it refuses can be retried on the endpoint door only
by code that owns the endpoint. The Mind finds the public door in the status file
(`public_proxy`, section 5), set only when the running proxy prints `public-door` in
`yantrik-egress capabilities`. When it is `null` (an older proxy), there is no public door, and
untrusted fetches must keep the Mind's own `lan_hosts` checks.

The kernel table lets the mind account reach both ports on `127.0.0.1` (`MIND_LOOPBACK_PORTS` in
`yantrik-update`) and nothing else of the proxy's.

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

  A host is written one way: lowercase, with no trailing dot, as a request's host is made, so a
  rule and the request it names compare as text. A rule with a trailing dot is refused when it is
  written (and a seeded URL's host loses its dot first). A `policy.yaml` from before that check
  is **read with the dot taken off**, not refused whole: that is the name the status file's
  `lan_hosts` already published and the public door refuses, while refusing the policy would
  refuse everything and the next seed or allow would be saved over the person's other rules.

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

- **A third mode, between them (5 October 2026).** **guarded** — *Home network closed, internet
  open*, the one Settings marks Recommended — lets every destination on the internet through with
  no rule, counted as allowed (not as watching), and decides the local network, private ranges,
  loopback and this machine exactly as enforce does: only a `lan` rule opens a LAN host, on the
  endpoint door only. The kernel refuses the account DNS, as in enforce; the proxy resolves. So
  nothing public is ever refused in guarded, and nothing public becomes a proposal. Strictness:
  audit < guarded < enforce. The words the person chooses from (the control socket's `status`
  answers them as `modes`): *Watch only: everything allowed and recorded* (audit); *Home network
  closed, internet open* (guarded, Recommended); *Only places I approve* (enforce).

- **The home network is more than the private ranges.** A dual-stack home gives every device a
  global IPv6 address from the ISP's prefix (the NAS at `2a02:8070:abcd:1::20`), and some homes and
  most servers sit on a public IPv4 subnet. So the proxy reads, with this machine's own addresses
  (`getifaddrs`), what the kernel says the home network is (`local::Net::build`):
  - the prefix each interface is on (its netmask);
  - every prefix routed straight out of an interface with no router, in every routing table, IPv4
    and IPv6: one netlink dump (`RTM_GETROUTE`, `NLM_F_DUMP`, no table asked for; `netlink.rs`),
    so a policy-routed host's on-link prefix in `table 100`, or a VRF's, counts. Only unicast
    routes are routes; the `local` table's own addresses and broadcasts, and the refusals, are
    not. When netlink will not answer, `/proc/self/net/route` (IPv4's main table only) and
    `ipv6_route`, and that is logged. A DHCPv6-only network (M=1, A=0) gives a /128 address, and its /64 exists only
    as such a route. Such a route counts only when it is no wider than /48 (IPv6) or /16 (IPv4),
    is not the /0 default, and does not leave through a point-to-point or tun device
    (`IFF_POINTOPOINT`, or `ARPHRD_NONE` in `/sys/class/net/<dev>/type`). So a full-tunnel VPN's
    `0.0.0.0/1` and `128.0.0.0/1 dev tun0` never make the internet the home network. A wider
    public route is skipped, and logged once;
  - the /56 around each global IPv6 address of this machine on a shared network (not on a
    point-to-point or tun device: a VPN's address says nothing of who is near). When the ISP
    delegates a /56 and the router puts the cameras on `…:2::/64` while the desktop is on
    `…:1::/64`, the camera is the home network too. At worst this refuses a neighbour of the
    ISP's (a VPS, or a pool that hands each customer a /64 of a shared /56), which a `lan` rule
    then names. It is a guess, so it is said: the control socket's `status` lists these /56s as
    `assumed_links`, and the refusal of an address that is the home network only by one says so;
  - every router a route sends through, in any table.

  An address on one of those prefixes, or a router's, is the local network, whatever range it is
  in (`local::place`): in every mode and on both doors, only a `lan` rule reaches it, on the
  endpoint door only. An address carried inside another (mapped, NAT64, 6to4) is judged by the
  one it carries.

  The network is read after the name is resolved, and is at most a second old (`local::Watch`):
  one `getifaddrs` and one route dump a second at most, however many connections, and a network
  change is seen within a second. Within that second, a network that just came up brought this
  machine a new address: a connection that leaves from an address the read did not know
  (`getsockname`) has the network read again, and when its destination is now the local network
  or this machine, the verdict on that place counts — a refusal closes it before a byte goes
  through, and a network that cannot be read then closes it too. When a read fails — the
  addresses, or the routes (any error but a missing IPv6 table, and netlink failing with no
  `/proc` to fall back on) — the last one that did not is used. When no
  read ever has, nothing is reached: `503`, saying the network could not be read, counted as
  refused (a rule might answer it once it reads), except an address that its range alone says is
  never a destination.

  **A wide public prefix on an interface.** All of a prefix an interface is on is the local
  network, however wide: a server put on `44.1.2.3/8` has all of 44/8 as its neighbours. That
  fails closed, but a rule without `lan` for a host in it, which reached it before, now refuses
  it ("its rule does not allow"), and is never proposed, because a rule covers it. Such a rule
  now needs `lan: true`. The proxy logs each such prefix once, and the control socket's `status`
  lists them as `wide_links`: public prefixes wider than /16 (IPv4) or /32 (IPv6).

  **Known limit: the router's public address.** A request to the home's own public WAN IPv4
  address hairpins back through the router (often to its admin page). That address is on no
  interface of this machine, and finding it means asking something outside ("what is my IP?"),
  which the proxy never does. To the proxy it is the internet: allowed in guarded and audit,
  reached in enforce only by a rule. A router that answers its admin page on the WAN side is
  exposed to the whole internet anyway.

- **A tunnel-only rule is stricter for plain http (guarded).** In guarded, a host no rule names
  is reached over plain `http://` and tunnels alike. A rule for it decides instead: a rule with
  `http: false` (say `api.x.ai:443`, tunnels only) refuses a plain-http request to that host on
  that port, which with no rule would have been allowed. That is the rule doing what it says; to
  allow plain http too, the rule says `http: true`.

- **The kernel follows a switch a moment later.** The proxy decides by the new mode the moment
  the control socket's `mode` returns. The kernel's table (DNS above all) and the status file
  follow when `yantrik-mind-egress.path` sees `policy.yaml` change and runs `apply`, normally
  well under a second; until then they are still the old mode's, which is never wider than the
  mode being left. The `mode` answer, and every `status` answer, carry `kernel_current`: whether
  the status file names this mode and Private mode and was made from this policy file or a later
  one (`null` when there is no status file). `apply` takes `policy.yaml`'s modification time
  (`stat -c %.9Y`) before it reads the policy, and once the table is loaded sets the status
  file's to it (`touch -d`); the proxy compares the two to the nanosecond, status no older than
  policy. When the policy changed while it loaded (its inode or time is not the one taken), `apply`
  loads again, three times more at most — the path unit may not run for a change made while it
  was running — and a policy still changing then leaves the status file older than it, so
  `kernel_current` stays `false` until the next change is loaded. The status file's `loaded_at`
  is whole seconds and the time of the load, so it is not used. A desktop can ask again until it
  is `true`.

### 4. Asking, instead of silently failing

When the mind connects somewhere no rule allows, the proxy refuses at once with `403` and a body
saying why. It also records a proposal: host, port, count, first and last time, and the program in
version 2. The shell shows the proposal as an ordinary approval card:

> The Mind wants to reach `api.example.com` (port 443) · Allow once · Always · No

A phone can answer the card like any other, since cards reach channels. Only the person's answer
writes a rule. This is OpenShell's Policy Advisor, done through the approvals we already have.
Repeated attempts to one host make one card, not a stream.

A refusal no rule could answer is counted (as `refused`, and as `never` in the ledger) but is
never a proposal, in any mode: a name that did not resolve (a typo, a dead link), a name that
resolves only to an address that is never a destination (a Pi-hole's `0.0.0.0` for a blocked
tracker, `127.0.0.1`), and a literal one (`127.0.0.1:7450`). Asking the person about those would
be a question whose Yes changes nothing. In enforce, a name with no rule is still refused before
it is looked up, so it is a proposal as before: a rule could answer it.

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
 "public_proxy": "http://127.0.0.1:7451", "proxy_refuses_private": true, "mode": "audit", "private": false, "dns_allowed": true,
 "lan_hosts": [{"host": "192.168.4.42", "ports": [8888]}, {"host": "homeassistant.local", "ports": [8123]}],
 "loaded_at": 1759600000, "version": 3}
```

- `proxy`: the endpoint door, as a URL. It honours `lan` rules (see "Two doors").
- `public_proxy`: the public door, as a URL like `proxy` (`http://127.0.0.1:7451`); use it as it
  is. It is set only when the installed `yantrik-egress capabilities`
  prints `public-door`, read from the binary at each apply like `proxy_refuses_private`. It is
  **`null`** from a proxy without one, and then every untrusted fetch keeps the `lan_hosts` checks
  below.

- `mode`: `audit`, `guarded` or `enforce`, the policy's; `fallback` when the policy could not be read or its
  answer was refused and the loaded table holds loopback only (`private` and `dns_allowed` false).
- `private`: Private mode is on; the table has no entries and refuses DNS.
- `dns_allowed`: the account may send DNS (audit, not Private; never in guarded). Otherwise every lookup is refused.
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
- `loaded_at`: unix seconds of the load. `version`: this layout (2 adds `lan_hosts`; 3 adds
  `public_proxy`). A reader refuses one it does not know.
- **guarded is version 3.** Every mode is written as version 3, `guarded` included; guarded
  changes no field's name or type, only adds a value of `mode`, with `dns_allowed` false. The
  Mind's reader never reads `mode` (its trust rests on the checks below, and it honours
  `dns_allowed` as written), so it reads a guarded file correctly with no change. A reader that
  does check `mode` must accept `guarded`.
- **Version 3 and version 2 readers.** Version 3 only adds `public_proxy`. Every version 2 field
  is still there, with the same name, type and meaning; the selftest checks that. A version 2
  reader keeps working by ignoring the new field, **once it accepts `version` 3 as well as 2**. A
  reader that still refuses every version it does not know will see 3 and fail closed (egress
  taken as not enforced) until it adds 3.

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
