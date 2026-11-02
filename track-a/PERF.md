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
| 6 | Percentiles: select p50 first, then p95 in the top half, then p99 in the top 5% | run-df1f0f914040 | 0.0618 | 53.1 | 68.8 |
| 7 | Mark the hours seen while parsing instead of a serial pass over all 3.1M records | | | | |
| 8 | Line cuts: reuse the SWAR scan to skip the method, one-line `mmap` call, simpler hash (288 to 274 counted lines) | | | | |
| 9 | Latency array zeroed via calloc (`vec![0u32; n]` collected in place into atomics) | | | | |
| - | Tried, reverted: count-then-rescan (two passes, no per-line records) | | | | |
| - | Tried, reverted: `madvise(MADV_POPULATE_READ)` per thread chunk | | | | |
| - | Tried, reverted: work-stealing parse over 8x more pieces than threads | | | | |

Steps 1 to 6 were first measured together on the board as run-df1f0f914040 (commit 3ce2b62): 0.0618 s, 53.1 MB, 297 lines, 68.8 points. The leader on A was at 0.014 s, 14 MB, 383 lines.

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
- Step 7: the rank phase walked every record once on one thread just to learn which hours occur. Each parse thread now sets a flag per hour as it goes, so the rank phase only merges 12 small arrays. Rank phase 2.0 ms to 0.9 ms (min of 10). The A/B wall time (best of 24) was flat: 0.060 s vs 0.059 s. Four seeds pass.
- Step 8: S counts lines after rustfmt, so the file is now rustfmt-clean. Skipping the method with `find_space` replaces a byte loop. Binding the fd first keeps the `mmap` call on one line. The hash loads `bytes[..min(n, 8)]` and `bytes[n - min(n, 8)..]` with no branch. 288 to 274 counted lines. A/B wall times over three rounds: 0.059/0.061, 0.057/0.056, 0.055/0.056 s (old/new), so no change. Four seeds pass.
- Board memory (53 MB) breaks down as 25 MB of packed per-line records, 12.4 MB latency array, 6.3 MB of per-thread group counts and ~10 MB of output buffers, with the 10 MB output file's page cache on top.
- Reverted, count-then-rescan: pass 1 interns endpoints and counts lines per (endpoint, hour) cell; pass 2 parses the chunk again and writes each latency straight to its slot, so the 25 MB of records go away. Local harness M (after subtracting the 239 MB input that Colima charges) fell from 49 MB to 40 MB, because the peak moves to the report phase (latencies + output buffers + output page cache). But the wall time rose from 0.075 s to 0.093 s. With the score's weights (time ^0.5, memory ^0.2), that is about -2.3 points for time against +0.8 for memory, a net loss. The code is kept out of the repo.
- Reverted, `MADV_POPULATE_READ` per chunk: an attempt to fault the mapping in bulk on each thread. The A/B was 0.054 s (old) vs 0.063 s (new), slower.
- Reverted, work stealing: 8x more pieces than threads, claimed from an atomic counter, so faster cores take more. The A/B was flat (0.082/0.079, 0.068/0.070 s). The box's 12 threads are 6 identical P-cores with hyperthreading, so there is no slow core to balance against. Not worth 8 more lines.
- To reach the leader's 14 MB, the per-line latencies can't be stored at all. That needs a multi-pass histogram selection that rescans the page-cached input, which is a redesign and not a tweak.
- Step 9: building 3.1M `AtomicU32`s one at a time zeroes and faults the 12.4 MB array serially before the scatter. Starting from `vec![0u32; n]` gives calloc'd pages, and the in-place collect into atomics reuses that allocation. The A/B over three rounds was 0.062/0.059, 0.062/0.061 and 0.063/0.062 s (old/new): small, but faster every time. Four seeds pass.
- Why records stay 8 bytes: latency spans the full `u32` range (0 to 4294967295 per the spec), so a line needs 32 bits of latency plus hour, endpoint and the 5xx flag. One `u64` per line is already the minimum for a single pass.
