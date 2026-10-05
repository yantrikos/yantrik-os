| condition | seeds | accuracy % (mean ± sd) | per-seed | positions | fwd MFLOPs/ex | KV floats/ex |
|---|---|---|---|---|---|---|
| compact_bpe | 1 | 32.3 ± nan | 32.3 | 76.9 | 27.2 | 29512 |
| mdg_seq_bpe | 1 | 37.6 ± nan | 37.6 | 72.6 | 25.4 | 27861 |
| mdg_packed | 3 | 98.0 ± 0.2 | 97.8, 98.1, 98.2 | 30.0 | 9.5 | 11520 |
| compact_packed | 3 | 88.1 ± 6.5 | 86.0, 82.8, 95.4 | 30.0 | 9.5 | 11520 |

| condition | where_now | where_at | holder_now | when_moved | root_cause | dev_now |
|---|---|---|---|---|---|---|
| compact_bpe | 33.9 | 33.1 | 23.9 | 6.4 | 29.1 | 67.4 |
| mdg_seq_bpe | 33.0 | 31.6 | 39.9 | 5.3 | 48.0 | 67.4 |
| mdg_packed | 99.9 | 98.7 | 89.8 | 99.9 | 99.8 | 99.9 |
| compact_packed | 99.7 | 97.2 | 90.3 | 46.6 | 94.9 | 99.7 |

