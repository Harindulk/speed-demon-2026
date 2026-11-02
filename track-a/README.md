# Track A: Ingest

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/a-ingest.md).

## Approach

A single Rust binary with no crates (`src/main.rs`), in four parallel phases:

1. **Parse.** `/data/requests.log` is memory-mapped read-only, so the page cache the board pre-warms is shared instead of copied into the process. The file is cut into one newline-aligned chunk per hardware thread. Each thread walks its chunk with byte offsets: the hour key comes from fixed timestamp positions (`(month * 32 + day) * 24 + hour`, chronological within the fixed 72-hour span and correct across the October to November rollover), the method is skipped, the endpoint is interned in a thread-local hash map, and only the first status digit is read (`5` means 5xx). Each line becomes one packed `u64`: hour key, 5xx flag, local endpoint id, latency.
2. **Rank.** The distinct endpoints are sorted bytewise and the hours present are ranked chronologically, so the dense group id `hour_rank * endpoints + endpoint_rank` is already in output order. No sort of the output is needed.
3. **Scatter.** Each thread rewrites its records as (group id, latency) and counts its groups; 5xx lines bump a shared atomic counter. Per-thread counts become write cursors (a counting sort), and every thread places its latencies into one flat array, so each group's latencies end up contiguous.
4. **Report.** Groups are split into ranges of about equal record counts. Each thread runs `select_nth_unstable` for the nearest-rank p50, p95 and p99 (rank `(p * n + 99) / 100`) and formats its lines; the buffers are written to `/out/summary.txt` in order.

The only `unsafe` is the `mmap` call (declared with `extern "C"`, no `libc` crate), with a fallback to `std::fs::read`.

## Dependencies

None.

## With another 24 hours

- Profile on the x86 box: the parse loop scans the endpoint byte by byte; an AVX2 or SWAR space search would cut it.
- Drop the 27 MB record buffer by parsing twice (count, then scatter straight into the latency array), and measure whether the extra parse costs more time than the memory saves in score.
- Look at huge pages and `MAP_POPULATE` for the input mapping.
