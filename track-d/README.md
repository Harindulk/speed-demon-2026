# Track D: Traverse

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/d-traverse.md).

## Approach

Zero-crate Rust, one file (`src/main.rs`).

1. **Load.** `graph.txt` is mapped with `mmap` (one small `extern "C"` declaration, no `libc` crate). The box pre-warms `/data` into the page cache, so the mapping costs no memory of ours, where reading the file into a `Vec` would cost 92 MB. The edge lines are split into one chunk per core at newline boundaries and parsed in parallel with a hand-written decimal loop.
2. **Build.** Forward and reverse CSR graphs (`first` offsets plus `(neighbour, weight)` pairs), counted and scattered from the parsed edges.
3. **Relabel.** Node ids are a random shuffle, so a search step in input ids is a cache miss on almost every edge. A breadth-first pass over the forward graph gives nodes that are close in the graph ids that are close in memory, and both CSRs are rewritten in that order (row copies, parallel blocks). The BFS runs while the reverse graph is still being built, and the row copies run in parallel blocks. This made the query phase about 3x faster locally.
4. **Query.** Every core takes queries from an atomic counter and runs a bidirectional Dijkstra (binary heap, forward search on the graph, backward search on the reversed graph), stopping when the two frontier minima add up to the best meeting distance found. The side with the *smaller heap* expands next, not the side with the smaller distance: from an island whose only exits weigh about 2^32, distance balancing lets the other side settle all 1.7M nodes before the island side runs dry (four such queries took 0.7 to 0.9 s each); size balancing ends them in a dozen steps.
5. **Memory.** A search's tentative distances live in a small open-addressing hash map per side per thread (linear probing, the low id bits as the hash, which suits BFS-ordered ids), not in a dense array of all 1.7M nodes. Dense arrays would cost 12 threads x 2 sides x 14 MB = 336 MB on the box, more than the whole graph; a search touches about 10^4 nodes. Locally this cut peak memory from 415 MB to 297 MB for about 15% more time, a net gain under the scoring formula (time weighs 0.5, memory 0.2, and the box runs 12 threads, not 8).
6. **Output.** Answers go into one string, written once, in query order. Distances are `u64` (answers can exceed 2^32 - 1).

## Dependencies

None.

## With another 24 hours

- A radix heap or a 4-ary heap keyed on `u64` instead of `BinaryHeap<Reverse<(u64, u32)>>`.
- ALT landmarks or a light contraction hierarchy: with 2,000 queries the preprocessing has to pay back in under ~100 ms, so it needs measuring on the box first.
- A parallel BFS for the relabelling, which is now the longest serial step.
- A parallel CSR build with atomic counters was tried and measured no faster locally (0.30-0.33 s either way), so the simpler serial build stayed; worth re-measuring on the box's 12 threads.
