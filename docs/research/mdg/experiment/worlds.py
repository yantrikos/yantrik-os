"""Procedural world generator for the MDG R1 experiment.

Every world is generated as a ground-truth event graph (the MDG). The three
text/code renderings are produced FROM that graph, so all conditions carry
exactly the same facts; no parser is involved and no answer is precomputed
into any rendering.

World content: people move between places, pass objects to each other,
devices switch off/on, and some device-off events have an explicit causal
chain (person -> intermediate event -> device off). Questions:

  where_now   Where is P now?                       -> place
  where_at    Where was P at time t?                -> place
  holder_now  Who has object O now?                 -> person
  when_moved  When did P last go to L?              -> time
  root_cause  Who ultimately caused D to go off?    -> person   (2-hop causal chain)
  dev_now     Is device D on now?                   -> yes/no
"""
import random
import re

PEOPLE = [("alice", "f"), ("bob", "m"), ("carol", "f"), ("dave", "m"), ("erin", "f"),
          ("frank", "m"), ("grace", "f"), ("heidi", "f"), ("ivan", "m"), ("judy", "f"),
          ("ken", "m"), ("lena", "f"), ("mike", "m"), ("nina", "f"), ("oscar", "m"),
          ("paul", "m"), ("quinn", "f"), ("rosa", "f"), ("sam", "m"), ("tara", "f")]
PLACES = ["kitchen", "garden", "office", "garage", "library", "lab", "hall", "attic",
          "cellar", "studio", "porch", "gym"]
OBJECTS = ["key", "laptop", "ledger", "torch", "badge", "camera", "map", "radio", "wallet", "drill"]
DEVICES = ["pump", "server", "heater", "fridge", "router", "lift"]
# intermediate cause events (actor does X -> X causes device off)
MECHS = ["breaker", "flood", "fire", "outage"]   # breaker trip, flood, fire, power outage
QTYPES = ["where_now", "where_at", "holder_now", "when_moved", "root_cause", "dev_now"]
N_TIMES = 24

def answer_vocab():
    return ([p for p, _ in PEOPLE] + PLACES + [f"t{i}" for i in range(N_TIMES + 1)] + ["yes", "no"])

class World:
    """Ground-truth MDG: typed facts with time index and explicit cause links."""
    def __init__(self, rng, n_people=6, n_places=5, n_objects=3, n_devices=2, n_events=16):
        self.people = rng.sample(PEOPLE, n_people)
        self.places = rng.sample(PLACES, n_places)
        self.objects = rng.sample(OBJECTS, n_objects)
        self.devices = rng.sample(DEVICES, n_devices)
        self.facts = []  # dicts: {type, args..., t, cause(optional t)}
        loc = {}; hold = {}; dev = {}
        # initial state at t0
        for p, _ in self.people:
            loc[p] = rng.choice(self.places)
            self.facts.append(dict(type="AT", p=p, l=loc[p], t=0))
        for o in self.objects:
            hold[o] = rng.choice(self.people)[0]
            self.facts.append(dict(type="HAS", p=hold[o], o=o, t=0))
        for d in self.devices:
            dev[d] = "on"
            self.facts.append(dict(type="DEV_ON", d=d, t=0))
        self.loc_hist = {p: [(0, l)] for p, l in loc.items()}
        self.root = {}  # device -> root-cause person of its most recent OFF
        t = 0
        pending = []  # scheduled caused events: (t, fact)
        times = sorted(rng.sample(range(1, N_TIMES + 1), n_events))
        for t in times:
            # emit a pending caused event if one is due (keeps causal links forward in time)
            if pending and pending[0][0] <= t:
                _, f = pending.pop(0); f["t"] = t
                self._apply(f, loc, hold, dev); self.facts.append(f); continue
            r = rng.random()
            if r < 0.40:
                p = rng.choice(self.people)[0]
                l = rng.choice([x for x in self.places if x != loc[p]])
                f = dict(type="MOVE", p=p, l=l, t=t)
            elif r < 0.65:
                o = rng.choice(self.objects); giver = hold[o]
                recv = rng.choice([q for q, _ in self.people if q != giver])
                f = dict(type="GIVE", p=giver, o=o, q=recv, t=t)
            elif r < 0.85 and not pending:
                # causal chain: actor triggers mechanism at t, device goes off later because of it
                d = rng.choice(self.devices); actor = rng.choice(self.people)[0]
                m = rng.choice(MECHS)
                f = dict(type="TRIGGER", p=actor, m=m, t=t)
                pending.append((t + 1, dict(type="DEV_OFF", d=d, cause=t, by=actor)))
            else:
                d = rng.choice(self.devices)
                if dev[d] == "off":
                    f = dict(type="DEV_ON", d=d, t=t)
                else:
                    actor = rng.choice(self.people)[0]
                    f = dict(type="DEV_OFF", d=d, t=t, cause=None, by=actor)  # switched off directly
            self._apply(f, loc, hold, dev); self.facts.append(f)
        self.loc, self.hold, self.dev = loc, hold, dev
        self.t_end = t

    def _apply(self, f, loc, hold, dev):
        if f["type"] == "MOVE":
            loc[f["p"]] = f["l"]; self.loc_hist[f["p"]].append((f["t"], f["l"]))
        elif f["type"] == "GIVE":
            hold[f["o"]] = f["q"]
        elif f["type"] == "DEV_OFF":
            dev[f["d"]] = "off"; self.root[f["d"]] = f["by"]
        elif f["type"] == "DEV_ON":
            dev[f["d"]] = "on"

    def questions(self, rng):
        """Return list of (qtype, args, answer). One per qtype where answerable."""
        qs = []
        p = rng.choice(self.people)[0]
        qs.append(("where_now", dict(p=p), self.loc[p]))
        movers = [p for p, h in self.loc_hist.items() if len(h) > 1]
        p = rng.choice(movers) if movers else rng.choice(self.people)[0]
        h = self.loc_hist[p]
        t = rng.randint(0, self.t_end)
        ans = [l for (tt, l) in h if tt <= t][-1]
        qs.append(("where_at", dict(p=p, t=t), ans))
        o = rng.choice(self.objects)
        qs.append(("holder_now", dict(o=o), self.hold[o]))
        if movers:
            p = rng.choice(movers); last = {}
            for tt, l in self.loc_hist[p][1:]:
                last[l] = tt
            l = rng.choice(sorted(last)); qs.append(("when_moved", dict(p=p, l=l), f"t{last[l]}"))
        offd = [d for d in self.devices if d in self.root]
        if offd:
            d = rng.choice(offd); qs.append(("root_cause", dict(d=d), self.root[d]))
        d = rng.choice(self.devices)
        qs.append(("dev_now", dict(d=d), "yes" if self.dev[d] == "on" else "no"))
        return qs

# ---------------------------------------------------------------- renderings
CLOCK = lambda t: f"{8 + t // 4}:{(t % 4) * 15:02d}"   # t -> wall-clock string for plain NL
MECH_NL = {"breaker": ("tripped the breaker", "the tripped breaker"),
           "flood": ("left a tap running and flooded the floor", "the flood"),
           "fire": ("started a small fire", "the fire"),
           "outage": ("cut the main power line", "the power outage")}

def _ev_by_time(w):
    return {f["t"]: f for f in w.facts if f["t"] > 0}

def render_plain(w, rng):
    """Verbose, varied English: pronouns, paraphrase, clock times, filler, indirect cause refs."""
    sex = dict(w.people); out = []; last_subj = None
    def name(p, obj=False):
        nonlocal last_subj
        if p == last_subj and rng.random() < 0.7:
            return ("her" if obj else "she") if sex[p] == "f" else ("him" if obj else "he")
        last_subj = p; return p
    out.append("This is the story of one day.")
    for f in w.facts:
        if f["t"] != 0: break
        if f["type"] == "AT":
            n = name(f["p"])
            out.append(rng.choice([f"At the start of the day {n} was in the {f['l']}.",
                                   f"{n.capitalize()} began the morning in the {f['l']}."]))
        elif f["type"] == "HAS":
            if rng.random() < 0.5:
                out.append(f"{name(f['p']).capitalize()} was carrying the {f['o']}.")
            else:
                out.append(f"The {f['o']} was with {name(f['p'], obj=True)}.")
        elif f["type"] == "DEV_ON":
            out.append(f"The {f['d']} was running.")
    byt = _ev_by_time(w)
    for f in w.facts:
        if f["t"] == 0: continue
        c = CLOCK(f["t"])
        when = rng.choice([f"At {c},", f"Around {c},", f"At {c} exactly,"])
        if f["type"] == "MOVE":
            n = name(f["p"])
            s = rng.choice([f"{when} {n} walked over to the {f['l']}.",
                            f"{when} {n} went to the {f['l']}.",
                            f"{when} {n} headed into the {f['l']}."])
        elif f["type"] == "GIVE":
            g = name(f["p"])
            s = rng.choice([f"{when} {g} handed the {f['o']} to {f['q']}.",
                            f"{when} {g} gave {f['q']} the {f['o']}."])
        elif f["type"] == "TRIGGER":
            s = f"{when} {name(f['p'])} {MECH_NL[f['m']][0]}."
        elif f["type"] == "DEV_OFF":
            if f.get("cause") is not None:
                s = rng.choice([f"{when} the {f['d']} stopped working because of {MECH_NL[byt[f['cause']]['m']][1]}.",
                                f"{when} {MECH_NL[byt[f['cause']]['m']][1]} knocked out the {f['d']}."])
            else:
                s = f"{when} {name(f['by'])} switched off the {f['d']}."
        elif f["type"] == "DEV_ON":
            s = rng.choice([f"{when} someone switched the {f['d']} back on.", f"{when} the {f['d']} came back on."])
        out.append(s[0].upper() + s[1:])
        if rng.random() < 0.25:
            out.append(rng.choice(["Nothing else happened for a while.", "The weather stayed mild.",
                                   "It was a quiet stretch.", "Somebody made tea."]))
    return " ".join(out)

def q_plain(q):
    t, a = q[0], q[1]
    return {"where_now": lambda: f"Where is {a.get('p')} now?",
            "where_at": lambda: f"Where was {a.get('p')} at {CLOCK(a.get('t', 0))}?",
            "holder_now": lambda: f"Who has the {a.get('o')} now?",
            "when_moved": lambda: f"At what time did {a.get('p')} last go to the {a.get('l')}?",
            "root_cause": lambda: f"Who is ultimately responsible for the {a.get('d')} going off most recently?",
            "dev_now": lambda: f"Is the {a.get('d')} on now?"}[t]()

def compact_sents(w):
    """Compact English: resolved references, canonical names, explicit time ids, one fact per clause.
    Causes are named by the time id of the causing event and the responsible person is NOT
    copied onto the effect (that would be precomputing the answer)."""
    S = []
    for f in w.facts:
        t = f"t{f['t']}"
        if f["type"] == "AT": S.append([t, f["p"], "is in", f["l"]])
        elif f["type"] == "HAS": S.append([t, f["p"], "has", f["o"]])
        elif f["type"] == "MOVE": S.append([t, f["p"], "goes to", f["l"]])
        elif f["type"] == "GIVE": S.append([t, f["p"], "gives", f["o"], "to", f["q"]])
        elif f["type"] == "TRIGGER": S.append([t, f["p"], "causes", f["m"]])
        elif f["type"] == "DEV_OFF":
            if f.get("cause") is not None: S.append([t, f["d"], "goes off because of", f"t{f['cause']}"])
            else: S.append([t, f["p"] if "p" in f else f["by"], "switches off", f["d"]])
        elif f["type"] == "DEV_ON": S.append([t, f["d"], "is on"])
    # split multiword phrases into words
    return [" ".join(s).split() + ["."] for s in S]

def q_compact(q):
    t, a = q[0], q[1]
    return {"where_now": lambda: ["where", "is", a["p"], "now", "?"],
            "where_at": lambda: ["where", "is", a["p"], "at", f"t{a['t']}", "?"],
            "holder_now": lambda: ["who", "has", a["o"], "now", "?"],
            "when_moved": lambda: ["when", "does", a["p"], "last", "go", "to", a["l"], "?"],
            "root_cause": lambda: ["who", "caused", a["d"], "last", "off", "?"],
            "dev_now": lambda: ["is", a["d"], "on", "now", "?"]}[t]()

def mdg_facts(w):
    """MDG serialisation: each fact is a typed tuple of atomic codes (one code per slot).
    Slots: [REL, arg1, arg2, arg3, TIME, CAUSE]; absent slots omitted in the sequence form."""
    F = []
    for f in w.facts:
        t = f"T{f['t']}"
        if f["type"] == "AT": F.append(["R_AT", "E_" + f["p"], "E_" + f["l"], t])
        elif f["type"] == "HAS": F.append(["R_HAS", "E_" + f["p"], "E_" + f["o"], t])
        elif f["type"] == "MOVE": F.append(["V_MOVE", "E_" + f["p"], "E_" + f["l"], t])
        elif f["type"] == "GIVE": F.append(["V_GIVE", "E_" + f["p"], "E_" + f["o"], "E_" + f["q"], t])
        elif f["type"] == "TRIGGER": F.append(["V_TRIG", "E_" + f["p"], "E_" + f["m"], t])
        elif f["type"] == "DEV_OFF":
            if f.get("cause") is not None: F.append(["V_OFF", "E_" + f["d"], t, "C_T%d" % f["cause"]])
            else: F.append(["V_OFF", "E_" + f["d"], "E_" + f["by"], t])
        elif f["type"] == "DEV_ON": F.append(["V_ON", "E_" + f["d"], t])
    return F

def q_mdg(q):
    t, a = q[0], q[1]
    return {"where_now": lambda: ["Q_WHERE_NOW", "E_" + a["p"]],
            "where_at": lambda: ["Q_WHERE_AT", "E_" + a["p"], f"T{a['t']}"],
            "holder_now": lambda: ["Q_HOLDER_NOW", "E_" + a["o"]],
            "when_moved": lambda: ["Q_WHEN_MOVED", "E_" + a["p"], "E_" + a["l"]],
            "root_cause": lambda: ["Q_ROOT_CAUSE", "E_" + a["d"]],
            "dev_now": lambda: ["Q_DEV_NOW", "E_" + a["d"]]}[t]()

def check_same_information(w):
    """Bijection check: compact NL and MDG carry the same fact list (same count, same entities, same times)."""
    C, M = compact_sents(w), mdg_facts(w)
    assert len(C) == len(M) == len(w.facts)
    for c, m in zip(C, M):
        ce = {x for x in c if not re.fullmatch(r"t\d+", x)} - {"is", "in", "has", "goes", "to", "gives", "causes",
                                                          "off", "because", "of", "switches", "on", "."}
        me = {x[2:] for x in m if x.startswith("E_")}
        assert ce == me, (c, m)
