# Baseline measurements

Measured on 1 November 2026, before any optimisation. Every track folder still holds the unchanged Python reference. Each `track-*/PERF.md` logs the steps from here, with the live-board run ID for every one.

## Board runs (scoring box, seed 1)

Every push to `speed-demon` is measured once by the live board. Per-step detail and reasoning live in each `track-*/PERF.md`.

| Commit | Track | Run ID | T (s) | M (MB) | S | Points | What changed |
|---|---|---|---|---|---|---|---|
| 3ce2b62 | A | run-df1f0f914040 | 0.0618 | 53.1 | 297 | 68.8 | First Rust version: mmap, 12-thread chunks, counting scatter, select_nth percentiles |
| 3ce2b62 | C | run-d53c8620b124 | 0.0681 | 20.1 | 309 | 67.3 | First Rust version: mmap, quote-aware parallel split, dense regions, name bitsets |
| 3ce2b62 | D | run-825573299ad2 | 0.5065 | 204.6 | 332 | 53.7 | First Rust version: CSR, BFS relabel, size-balanced bidirectional Dijkstra |
| 3ce2b62 | B | run-802df1e33e90 | | | | | Not run: folder still held the reference (the board skips it) |
| bc14fde | B | run-9a95e9f79a61 | 0.6952 | 114.4 | 448 | 55.4 | First Rust version: query-term-only postings, per-chunk hybrid bitsets, parallel queries |
| bc14fde | A | run-08e2d5bb50f8 | 0.0616 | 52.9 | 297 | 68.8 | Unchanged: re-run shows board noise under 1% on T |
| bc14fde | C | run-716b1ac5363a | 0.0677 | 24.4 | 309 | 66.6 | Unchanged: M moved 20.1 → 24.4 MB between identical runs, so M differences under ~5 MB are noise |
| bc14fde | D | run-dba010fa61f0 | 0.5297 | 205.8 | 332 | 53.2 | Unchanged: T within 5% of the previous run |
| 5d4ccc6 | A | run-addbbb812832 | 0.0760 | 53.9 | 281 | 66.6 | Seen-hours flag moved into the parse, line cuts: 23% slower on the box (shared-cache-line writes from 12 threads, invisible locally); fix follows |
| 5d4ccc6 | C | run-73ce90a4f176 | 0.0674 | 28.6 | 312 | 65.9 | 256 KB worker stacks: no effect, M is noisy (20-29 MB on identical code), reverted in 2b636c9 |
| 5d4ccc6 | B | run-6f6434c2ca40 | 0.7095 | 112.7 | 448 | 55.2 | Unchanged |
| 5d4ccc6 | D | run-4c67f524c848 | 0.5171 | 205.2 | 332 | 53.4 | Unchanged |
| fc1e03f | A | run-fe1b1d19685e | 0.0588 | 60.3 | 284 | 68.9 | calloc latency array, still with steps 7-8: fastest A so far, so 5d4ccc6's 0.076 s was board noise, not steps 7-8 |
| fc1e03f | B | run-86258154b497 | 0.6295 | 133.8 | 519 | 55.3 | Phrase-pair index, growable term table: 10% faster, but +71 lines and +20 MB cancel it in K |
| fc1e03f | C | run-e5f0de48672e | 0.0836 | 22.1 | 320 | 64.6 | Name lookups batched 32 at a time: 24% slower, reverted |
| fc1e03f | D | run-23a4b3d65854 | 0.4046 | 210.5 | 402 | 55.4 | Multiplicative hash, parallel multi-root BFS relabel, radix heap: 20% faster, +2 points |
| f885f9b | A | run-32c6cb071a4e | 0.0590 | 64.5 | 288 | 68.5 | Undo the in-parse hour flag and slice hash: same T as fc1e03f, so those two were never the problem; 5d4ccc6's 0.076 s was a noisy run |
| f885f9b | B | run-0f71a590def1 | 0.6845 | 133.3 | 519 | 54.4 | Unchanged code: 0.630 → 0.685 s between identical runs, B's board noise is ~8% |
| f885f9b | C | run-115fa9606cd3 | 0.0617 | 18.1 | 317 | 68.8 | Still the batched-lookup code: 0.084 then 0.062 s on identical builds; the three unbatched runs (0.067-0.068) are the steadier choice, revert stands |
| f885f9b | D | run-c0e7c2523de5 | 0.2767 | 244.8 | 441 | 58.5 | Owner-bucketed parallel CSR build and bucketed reverse transpose: 0.405 → 0.277 s, +3.2 points, the biggest single step of the day |
| dbff237 | A | run-6465cef9f442 | 0.0631 | 61.2 | 288 | 68.0 | Unchanged code: 0.059 → 0.063 s on identical builds, A's board noise is ~7% |
| dbff237 | B | run-15d9f1cc2d14 | 0.6498 | 133.0 | 519 | 55.0 | Unchanged code: 0.630, 0.685, 0.650 s across three identical runs |
| dbff237 | C | run-6c63bcf3ed77 | 0.0680 | 22.3 | 309 | 66.9 | Unbatched lookups restored: 0.068 s, inside the 0.067-0.068 band of the earlier unbatched runs |
| dbff237 | D | run-8334de25364f | 0.2694 | 173.7 | 449 | 60.2 | Old CSR freed before the transpose, exact-size transpose buckets: M 244.8 → 173.7 MB, T flat, +1.7 points, best D |
| bd92f70 | A | run-fc143cdf6cd3 | 0.0579 | 61.0 | 304 | 68.8 | SWAR latency parse and open-addressing intern table: fastest A run yet but +16 lines, so points flat; final code commit |
| bd92f70 | B | run-a3309fb2f924 | 0.5929 | 134.0 | 519 | 55.9 | Unchanged code: 0.593 s, the best of four identical runs (0.630-0.685), so the pair index did pay after all |
| bd92f70 | C | run-d35980de4a5e | 0.0678 | 17.9 | 309 | 67.9 | Unchanged code: 0.068 s again, M 22 → 18 MB is the usual noise |
| bd92f70 | D | run-fa90cf38b4d7 | 0.3686 | 174.6 | 449 | 56.8 | Unchanged code: 0.269 → 0.369 s between identical runs, D's board noise reaches 35% |

## Final board (code freeze, 2 November 15:00 UTC)

Best scored run per track on the live board at the freeze. Final scoring reruns the frozen code on a holdout seed, so these are the board's numbers, not the final ones.

| Track | Commit | Run ID | T (s) | M (MB) | S | Points |
|---|---|---|---|---|---|---|
| A | fc1e03f | run-fe1b1d19685e | 0.0588 | 60.3 | 284 | 68.9 |
| B | bd92f70 | run-a3309fb2f924 | 0.5929 | 134.0 | 519 | 55.9 |
| C | f885f9b | run-115fa9606cd3 | 0.0617 | 18.1 | 317 | 68.8 |
| D | dbff237 | run-8334de25364f | 0.2694 | 173.7 | 449 | 60.2 |

Frozen code is bd92f70 (Track A SWAR parse and intern table; B, C and D unchanged since dbff237), scored on all four tracks before the freeze. Best three (A, C, D) give a geometric mean of about 65.8 points on the machine part.

Lesson from the first runs: local best-of-N times in Colima were close for A and C (0.08 and 0.07 s) but D ran 1.5x slower on the board (0.35 s local, 0.51 s board), and local memory, measured as container peak minus mapped input, missed 15-50 MB of heap the board counts. The leaders are at 0.014 s (A, C) and 0.072 s (D), so the gap is real, not a measuring error. From here every decision uses board numbers.

## Machines

| | Scoring box (counts) | Local (for iteration only) |
|---|---|---|
| CPU | Intel Core i5-13500, 6 P-cores / 12 threads, turbo off, AVX2, no AVX-512 | Apple M5, 4 P + 6 E cores, arm64 |
| Memory | 8 GB cap, no swap | 16 GB host; Colima VM with 8 vCPUs, 10 GB (vz) |
| OS / runtime | Linux, Docker, cgroup v2 | macOS 27.2, Colima, Docker 29.5.2 (aarch64, cgroupfs, cgroup v2) |
| Toolchain | per Dockerfile, from the allowlist | Python 3.13.16 (uv), Rust 1.99.1, `python:3.13-slim` pinned image |

The local machine is a different architecture from the box (arm64, not x86-64). Local times are only good for comparing one version with another on this Mac. Any x86 SIMD path also needs a portable fallback before it can be tested here.

## Reference on the scoring box (seed 1, full scale)

From `harness/calibration-seed1.json` in the public specs repo (organiser calibration, 24 October). These are the T_ref and M_ref every submission is divided into.

| Track | Task | T_ref (s) | M_ref (MB) | S_ref (lines) | Final timeout (10x) | Live-board cap (3x) |
|---|---|---|---|---|---|---|
| A | Ingest: 3.1M log lines → hourly percentiles | 32.05 | 228 | 56 | 320.5 s | 96.1 s |
| B | Index: 250k docs, 50k boolean / phrase queries | 56.35 | 2,200 | 91 | 563.5 s | 169.1 s |
| C | Join: hostile 202 MB CSV ⨝ regions → per-country aggregates | 14.44 | 367 | 108 | 144.4 s | 43.3 s |
| D | Traverse: 1.74M nodes, 5M edges, 2,000 shortest paths | 72.69 | 893 | 43 | 726.9 s | 218.1 s |

## Reference on the local machine (seed 1, full scale)

The official harness (`harness/run_submission.py`) ran with the arm64 `sd-timer`, `--cgroups colima --cpuset 0-7 --runs 3`. All three gates passed on every track: correctness, timeout and sandbox.

| Track | T local (s) | Walls (s) | M local (MB) | Box T ÷ local T |
|---|---|---|---|---|
| A | 11.07 | 11.65, 10.66, 11.07 | 467 | 2.90 |
| B | 32.93 | 34.77, 32.64, 32.93 | 2,456 | 1.71 |
| C | 6.13 | 6.79, 6.13, 5.97 | 572 | 2.36 |
| D | 43.49 | 43.77, 43.19, 43.49 | 986 | 1.67 |

- The box/local ratio ranges from 1.7 to 2.9 depending on the track, so a local time can't be turned into a box time with one factor. Only board runs count.
- Local M is higher than on the box. The box pre-warms `/data` into the page cache before the run's cgroup exists. Colima reads it through virtiofs during the run, so the cache pages are charged to the run. Treat local M as an upper bound.
- Native macOS runs from a background agent session are about 10x slower (A 133 s, C 111 s). The shell there runs at background priority (PRI 4), so macOS keeps it on the efficiency cores. Measure inside Colima.

## Correctness seeds

The final score is measured on a holdout seed nobody has seen. Three more full-scale seeds per track were made with the published generators (`tools/gen.sh`, in the reference's pinned image). The reference's output hash for each is in `tools/expected.txt`. Seed-1 inputs and outputs match the hashes published at `s3.amazonaws.com/raptors.dev/speeddemon.dev/2026/data/SHA256SUMS.json`.

Reference run through `check.py` (board sandbox settings; time includes container start-up):

| Track | Seed 1 | Seed 2 | Seed 3 | Seed 4 |
|---|---|---|---|---|
| A | 11.79 s `495ca096…` | 12.82 s `fe3469ff…` | 12.40 s `5207056c…` | 12.70 s `a3122e5c…` |
| B | 33.05 s `f86973c2…` | 31.21 s `7f224ef6…` | 33.70 s `7e004661…` | 32.80 s `54cc921b…` |
| C | 7.76 s `d8088b1c…` | 7.29 s `f99e6f91…` | 7.24 s `4fb4eeb2…` | 7.36 s `6acf40b6…` |
| D | 46.52 s `a23dbdf0…` | 47.18 s `23d60c06…` | 46.10 s `6b5c6836…` | 42.67 s `c70bc5ff…` |

Seed 1 is the live board's dataset. Seed sizes vary slightly (A's `requests.log` is 231.6 to 242.1 MB across seeds 1 to 4), so no code may assume seed-1 byte counts.

## Reproduce

```
git clone https://github.com/hackathon-raptors/speed-demon-2026 _spec   # specs, generators, harness
colima start --cpu 8 --memory 10 --vm-type vz
tools/gen.sh a 1                     # .data/a/full-seed1 (any track, any seed)
tools/verify.sh a                    # build track-a once, check every seed in tools/expected.txt
python3 _spec/harness/run_submission.py --context ./track-a --track a \
  --data ./.data/a/full-seed1 --output summary.txt \
  --expected-sha256 495ca09682b9673ef836a506496177680bb4a3af54255174befddb1f83481cd3 \
  --timeout-s 320.5 --timer _spec/harness/timer/sd-timer-arm64 \
  --cpuset 0-7 --runs 3 --cgroups colima --work ~/sd-work --results a.json
```

`tools/ref.sh <track> <data>` runs the reference natively and prints its hash, for data made outside Docker.
