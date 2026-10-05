"""Train one (condition, seed) run and write a JSON result.

Model: identical small Transformer encoder for every condition (same d_model,
layers, heads, vocab table size, answer head). A position's input embedding is
sum_k(tok_emb[id_k] + slot_emb[k]) + pos_emb[i]; for ordinary token sequences
k=1. Answer = classification over the shared answer vocabulary, read at <read>.
Training data are freshly generated worlds (one random question per world);
every condition sees the same number of training examples (matched data),
FLOPs differ and are reported.
"""
import argparse, json, math, random, time, os
import numpy as np, torch, torch.nn as nn
import worlds as W, data as D

p = argparse.ArgumentParser()
p.add_argument("--cond", required=True); p.add_argument("--seed", type=int, default=0)
p.add_argument("--steps", type=int, default=3000); p.add_argument("--bs", type=int, default=64)
p.add_argument("--d", type=int, default=128); p.add_argument("--layers", type=int, default=4)
p.add_argument("--heads", type=int, default=4); p.add_argument("--lr", type=float, default=1e-3)
p.add_argument("--n_test", type=int, default=2000); p.add_argument("--out", default="results")
p.add_argument("--threads", type=int, default=1); p.add_argument("--device", default="cpu")
args = p.parse_args()
torch.set_num_threads(args.threads); torch.manual_seed(args.seed)
V, A = D.build_vocab()
MAXLEN, MAXK = 400, 8

def batch(rng, n):
    ex = []
    for _ in range(n):
        w = W.World(rng, **D.world_kwargs(rng)); q = rng.choice(w.questions(rng))
        pos, ans = D.example(w, q, args.cond, rng); ex.append((pos, ans, q[0]))
    return ex

def tensorize(ex):
    L = max(len(e[0]) for e in ex); assert L <= MAXLEN
    X = torch.zeros(len(ex), L, MAXK, dtype=torch.long); M = torch.zeros(len(ex), L, dtype=torch.bool)
    R = torch.zeros(len(ex), dtype=torch.long); Y = torch.tensor([A[e[1]] for e in ex])
    for i, (pos, _, _) in enumerate(ex):
        for j, toks in enumerate(pos):
            for k, t in enumerate(toks):
                X[i, j, k] = V.setdefault(t, len(V)) if len(V) < VOCAB_CAP else V.get(t, 0)
            M[i, j] = True
        R[i] = len(pos) - 1
    return X, M, R, Y

VOCAB_CAP = 2048
class Model(nn.Module):
    def __init__(s):
        super().__init__()
        s.tok = nn.Embedding(VOCAB_CAP, args.d, padding_idx=0); s.slot = nn.Embedding(MAXK, args.d)
        s.pos = nn.Embedding(MAXLEN, args.d)
        layer = nn.TransformerEncoderLayer(args.d, args.heads, 4 * args.d, dropout=0.0, batch_first=True, norm_first=True)
        s.enc = nn.TransformerEncoder(layer, args.layers); s.norm = nn.LayerNorm(args.d)
        s.head = nn.Linear(args.d, len(A))
    def forward(s, X, M, R):
        present = (X != 0).unsqueeze(-1).float()
        e = ((s.tok(X) + s.slot.weight[None, None, :, :]) * present).sum(2)
        e = e + s.pos.weight[None, :X.shape[1]]
        h = s.enc(e, src_key_padding_mask=~M)
        return s.head(s.norm(h[torch.arange(len(R)), R]))

dev = torch.device(args.device)
model = Model().to(dev)
n_params = sum(p.numel() for p in model.parameters())
opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.01)
sched = torch.optim.lr_scheduler.LambdaLR(opt, lambda s: min(1, (s + 1) / 200) * 0.5 * (1 + math.cos(math.pi * min(s, args.steps) / args.steps)))
rng = random.Random(1000 + args.seed)
test_rng = random.Random(10**6)          # identical test worlds/questions for all conditions
test = []
for _ in range(args.n_test):
    w = W.World(test_rng, **D.world_kwargs(test_rng))
    for q in w.questions(test_rng):
        test.append((w, q))

t0 = time.time(); tok_seen = 0
for step in range(args.steps):
    ex = batch(rng, args.bs); X, M, R, Y = (t.to(dev) for t in tensorize(ex))
    tok_seen += int(M.sum())
    loss = nn.functional.cross_entropy(model(X, M, R), Y)
    opt.zero_grad(); loss.backward(); nn.utils.clip_grad_norm_(model.parameters(), 1.0); opt.step(); sched.step()
    if step % 500 == 0: print(f"{args.cond} s{args.seed} step {step} loss {loss.item():.3f} {time.time()-t0:.0f}s", flush=True)
train_time = time.time() - t0

model.eval(); correct = {}; total = {}; lens = []
erng = random.Random(4242)   # plain NL surface randomness at test time, fixed
with torch.no_grad():
    for i in range(0, len(test), 256):
        ex = []
        for w, q in test[i:i + 256]:
            pos, ans = D.example(w, q, args.cond, erng); ex.append((pos, ans, q[0])); lens.append(len(pos))
        X, M, R, Y = (t.to(dev) for t in tensorize(ex))
        pred = model(X, M, R).argmax(-1).cpu()
        for (pos, ans, qt), pr, y in zip(ex, pred, Y.cpu()):
            correct[qt] = correct.get(qt, 0) + int(pr == y); total[qt] = total.get(qt, 0) + 1
acc = sum(correct.values()) / sum(total.values())
L = float(np.mean(lens)); d = args.d; nl = args.layers
# forward FLOPs per example (2*MACs): per layer 24*L*d^2 (QKV,O,FFN 4x) + 4*L^2*d (scores, AV)
flops_fwd = nl * (24 * L * d * d + 4 * L * L * d)
res = dict(cond=args.cond, seed=args.seed, acc=acc, per_type={k: correct[k] / total[k] for k in total},
           n_test=sum(total.values()), mean_positions=L, kv_floats_per_example=2 * nl * L * d,
           fwd_flops_per_example=flops_fwd, train_flops_approx=3 * flops_fwd * args.steps * args.bs,
           params=n_params, steps=args.steps, bs=args.bs, train_time_s=train_time, positions_seen=tok_seen,
           vocab_used=len(V), args=vars(args))
os.makedirs(args.out, exist_ok=True)
json.dump(res, open(f"{args.out}/{args.cond}_s{args.seed}.json", "w"), indent=1)
print(json.dumps({k: res[k] for k in ["cond", "seed", "acc", "mean_positions", "train_time_s"]}), res["per_type"])
