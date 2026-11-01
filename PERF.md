# Baseline measurements

Measured on 1 November 2026, before any optimisation. Every track folder still holds the unchanged Python reference. Each `track-*/PERF.md` logs the steps from here, with the live-board run ID for every one.

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
