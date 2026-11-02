# Track A: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 32.05 | 228 | 0 |
| 1 | Rust rewrite: mmap, 12-thread parse, packed records, counting sort into flat latency array, `select_nth_unstable` percentiles, parallel formatting | | | | |
| 2 | Thread-major cursor build, FNV-style hasher instead of SipHash | | | | |
| 3 | Shared atomic 5xx counters instead of one array per thread; write buffers straight to the file instead of concatenating | | | | |
| 4 | Hash endpoints by first and last 8 bytes plus length | | | | |
| 5 | SWAR (8 bytes at a time) search for the space that ends the endpoint | | | | |
| 6 | Percentiles: select p50 first, then p95 in the top half, then p99 in the top 5% | | | | |

Local notes (Apple M5 under Colima, 8 vCPUs, shared with three other builds, so noisy; only board numbers count):

- Step 1: official harness `T_s` not taken; in-container wall 0.17 to 0.47 s. A per-phase timing build showed parse ~50 ms, cursor build ~35 ms, scatter ~4 ms, report ~6 ms. All four seeds match the reference hashes.
- Step 2: the cursor loop walked eight count arrays per group (strided); thread-major passes made it sequential. SipHash's DoS protection is wasted on trusted ASCII. In-container best 0.17 s. Four seeds pass.
- Step 3: saves one 0.5 MB array per thread and one 10 MB copy of the output. 5xx lines are a few percent of traffic, so the atomic adds rarely contend. Four seeds pass.
- Step 4: hashing every byte of a ~30-byte endpoint per line is wasted work; two 8-byte loads separate the 1,842 endpoints well, and the map still compares full bytes. Official harness `T_s` 0.18 to 0.26 s (noise); in-container best of 10: 0.100 s. Four seeds pass.
- Profiling method from step 5 on: a copy of the code with phase timestamps, run 10 times in the container, min and median per phase. A first parallel pass that only counts newlines showed that locally ~40 ms of the parse phase is page-faulting the mmap through Colima's virtiofs, not parsing: counting alone took 48 ms, the full parse 49 ms. With page tables warmed by that pass, the real parse cost shows: 25 ms at step 4.
- Tried and dropped: `MAP_POPULATE` (one thread faults everything in the `mmap` call: 150 to 260 ms instead of 48 ms), and `std::fs::read` (89 ms to copy 240 MB single-threaded, and the copy would count toward M). Per-thread `pread` into small buffers would skip the faults, but the board's input is in the native page cache, where mmap faults are cheap, so it isn't worth the extra code without a board run to show it.
- Step 5: warm parse 25 ms to 15 ms (min of 10). An endpoint averages ~30 bytes and was scanned one byte per iteration; XOR with spaces and the has-zero-byte test checks eight per iteration. The interleaved wall-clock A/B (best of 24) was flat at 0.080 s vs 0.081 s, because the local virtiofs fault cost hides the compute; on the board compute is a larger share. Four seeds pass.
- Step 6: three full `select_nth_unstable` calls cost about 3n; p50 first partitions the slice, so p95 searches the top half and p99 the top 5%: about 1.55n. Report phase 4.2 ms to 3.5 ms (min of 10, under load from other builds). Four seeds pass.
- Local M from the harness is 289 MB because Colima charges the mmap'd page cache to the run; on the board the input is pre-warmed, so expect ~50 MB (27 MB records, 12 MB latencies, 10 MB output).
- `check.py` and `tools/verify.sh` can fail with `NotFound` on sub-second runs: `check.py` deletes and recreates `.out/track-a` between runs and Colima's virtiofs serves a stale directory. A fresh output folder per run passes every time; this is a local artefact, not a bug in the solution.
- Stopped at 13:33 UTC: the host disk filled up (4 GB free on 460 GB) and Colima's containerd started returning I/O errors, so nothing more could be built or measured. Steps 5 and 6 were both verified on all four seeds before that.
