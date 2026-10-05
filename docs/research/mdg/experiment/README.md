# MDG R1 experiment

This experiment compares a text serialisation and an MDG serialisation of the same procedurally generated worlds. Every condition uses the same small Transformer and is trained on the same number of examples. See `PREREGISTRATION.md` for the kill criteria, which were fixed before the main runs, and `../R1_report.md` for the results.

| file | what it is |
|---|---|
| `worlds.py` | World generator. It produces the ground-truth event graph (the MDG), the question types and the renderings (plain English, compact English, MDG tuples). `check_same_information` asserts that the compact and MDG renderings carry the same facts. |
| `bpe.py` | A tiny domain BPE over word units. It is used for the `*_bpe` conditions. |
| `data.py` | Turns a world into the position lists for each condition. |
| `train.py` | Trains and evaluates one (condition, seed) pair and writes `results/<cond>_s<seed>.json`. |
| `run_all.sh` | Runs all the main runs, 4 parallel CPU jobs by default. |
| `analyze.py` | Builds the results table and evaluates kill criteria K1–K3. |

## Reproduce on CPU (what was run in the sandbox)
```sh
pip install torch numpy
./run_all.sh          # about 2.5 h on 4 CPU cores
python3 analyze.py
```

## Single GPU (e.g. one RTX 3090 Ti)
The CPU budget leaves the model under-trained, with some question types near chance (see the report). On a GPU, use more steps and a bigger model, and run all 3 seeds for `plain`:
```sh
for c in plain compact compact_bpe mdg_seq mdg_seq_bpe mdg_packed compact_packed; do
  for s in 0 1 2 3 4; do
    python3 train.py --cond $c --seed $s --d 256 --layers 6 --heads 8 --bs 128 --steps 30000 \
      --lr 5e-4 --device cuda --out results_gpu
  done
done
```
Data generation runs in Python on the CPU. It is the bottleneck for the short conditions, and two GPUs can run two seeds at once (`CUDA_VISIBLE_DEVICES=0` / `1`). After the runs, point `analyze.py` at `results_gpu`: change its `glob`, or `mv results_gpu results`. Keep the kill criteria in `PREREGISTRATION.md` unchanged.
