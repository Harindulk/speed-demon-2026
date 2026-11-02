# StarFun: Speed Demon 2026

Our entry for [Speed Demon 2026](https://speeddemon.dev), all four tracks. Each `track-*` folder is one standalone Rust program with no crates (D = 1 on every track), built offline on the allowlisted `rust:1-slim-trixie` image and copied into `debian:13-slim`.

| Track | Task | Idea in one line | Details |
|---|---|---|---|
| A | 3.1M log lines → hourly p50/p95/p99 | mmap, one chunk per thread, every line packed into one `u64`, counting-sort scatter, `select_nth_unstable` | [track-a](track-a/README.md) |
| B | 250k docs, 50k boolean and phrase queries | index only the terms (and adjacent term pairs) the queries use, per-chunk sorted lists that turn into bitsets, no posting merge | [track-b](track-b/README.md) |
| C | Hostile 202 MB CSV joined to regions | split on record starts without a quote pass, SWAR quote scan, dense region array, one bit per (country, name) | [track-c](track-c/README.md) |
| D | 2,000 shortest paths on 1.74M nodes / 5M edges | owner-bucketed parallel CSR, BFS relabel for locality, size-balanced bidirectional Dijkstra with a radix heap | [track-d](track-d/README.md) |

<!-- RESULTS -->

Three choices hold across all four tracks:

- **Inputs are memory-mapped.** The box pre-warms `/data` into the page cache before the run starts, so mapped input costs no memory, while reading it into a buffer would cost its full size. The `mmap` call is declared with `extern "C"` and is the only `unsafe` block in each program.
- **One thread per hardware thread, no locks in hot loops.** Work is split into ranges that threads own outright; results are merged once after `join`.
- **Correct first.** Every change was checked against the reference's sha256 on four full-scale seeds (the board's seed 1 and seeds 2-4 made with the public generators) before it was pushed. `PERF.md` (root) lists every board run; each `track-*/PERF.md` lists every step, including the ones we reverted and why.

## Reproduce

```
git clone https://github.com/hackathon-raptors/speed-demon-2026 _spec       # specs, generators, harness
tools/gen.sh a 1                                                           # full-scale data for any track and seed
tools/verify.sh a                                                          # build once, check every seed in tools/expected.txt
```

`check.py` is the organisers' single-run check: `python3 check.py a --data .data/a/full-seed1 --expected <sha256>`.

## The rules each program runs under

- It reads its inputs from `/data`, which is read-only, and writes exactly one file to `/out`, then exits 0. The file is `summary.txt` for A, `results.txt` for B, `aggregates.txt` for C and `distances.txt` for D.
- `TRACK` is set to `a`, `b`, `c` or `d`.
- There is no network, at build time or at run time (we have no dependencies to vendor).
- It runs as an unprivileged user on a read-only filesystem. `/tmp` is 1 GB of scratch space.
- It gets 12 hardware threads (six P-cores, turbo off), 8 GB of memory with no swap, and 512 processes.
- The live board stops a run at 3x the reference's time. Final scoring stops it at 10x.
- The output must match the reference's byte for byte (sha256) on every run.

## Scoring, briefly

Faster than the reference earns points. Using less memory helps, up to a cap. Code beyond the reference's size, and every dependency, cost a little. Each track folder is built and measured on its own: your other tracks never count toward it, and code shared between tracks is copied into each folder that uses it and counts in each. A run that fails correctness, the timeout or the sandbox scores nothing. The full formula and the judging criteria are on speeddemon.dev.

## Licence

MIT, see `LICENSE`. The starter and reference code it grew from are copyright Hackathon Raptors CIC under the same licence.
