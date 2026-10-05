| condition | seeds | accuracy % (mean ± sd) | per-seed | positions | fwd MFLOPs/ex | KV floats/ex |
|---|---|---|---|---|---|---|
| plain | 1 | 25.1 ± nan | 25.1 | 290.2 | 150.3 | 111442 |
| compact | 3 | 29.5 ± 1.2 | 28.5, 29.2, 30.8 | 169.5 | 72.1 | 65090 |
| compact_bpe | 3 | 30.0 ± 0.6 | 30.6, 29.6, 29.7 | 76.9 | 27.2 | 29512 |
| mdg_seq | 3 | 30.9 ± 2.0 | 29.6, 29.9, 33.2 | 112.7 | 43.0 | 43260 |
| mdg_seq_bpe | 3 | 31.8 ± 0.5 | 31.2, 32.2, 32.1 | 72.6 | 25.4 | 27861 |
| mdg_packed | 3 | 66.9 ± 1.3 | 65.6, 68.3, 66.6 | 30.0 | 9.5 | 11520 |
| compact_packed | 3 | 64.0 ± 2.5 | 61.1, 65.8, 65.0 | 30.0 | 9.5 | 11520 |

| condition | where_now | where_at | holder_now | when_moved | root_cause | dev_now |
|---|---|---|---|---|---|---|
| plain | 21.9 | 20.2 | 17.2 | 5.3 | 18.5 | 67.4 |
| compact | 29.5 | 27.3 | 21.0 | 5.6 | 23.5 | 70.0 |
| compact_bpe | 30.6 | 30.9 | 21.0 | 5.1 | 24.7 | 67.4 |
| mdg_seq | 30.9 | 30.0 | 22.3 | 5.2 | 26.5 | 70.5 |
| mdg_seq_bpe | 31.3 | 28.1 | 32.9 | 5.7 | 25.6 | 67.4 |
| mdg_packed | 95.8 | 85.4 | 54.3 | 4.9 | 65.7 | 94.8 |
| compact_packed | 93.8 | 81.6 | 48.1 | 5.2 | 62.7 | 92.4 |

T* = compact_bpe (30.0%, 76.9 pos);  M* = mdg_packed (66.9%, 30.0 pos)
K1: acc(M*) - acc(T*) = +36.9 pt; pos ratio = 0.39 -> PASS
K2: compact_packed 64.0 vs mdg_packed 66.9 -> literal 1-pt rule: MDG packed ahead; gap +2.9 pt vs 2*sd noise 5.0 -> NOT detectable (noise rule)
   Welch t = 1.77
K3/compare mdg_seq vs compact_bpe: Δacc +1.0 pt (2·sd = 4.0), positions 112.7 vs 76.9 -> no detectable difference
K3/compare mdg_seq_bpe vs compact_bpe: Δacc +1.9 pt (2·sd = 1.1), positions 72.6 vs 76.9 -> higher
K3/compare mdg_seq vs compact: Δacc +1.4 pt (2·sd = 4.0), positions 112.7 vs 169.5 -> no detectable difference
K3/compare mdg_packed vs compact_packed: Δacc +2.9 pt (2·sd = 5.0), positions 30.0 vs 30.0 -> no detectable difference
