# Track D: Traverse

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/d-traverse.md).

## Approach

Zero-crate Rust, one file (`src/main.rs`).

1. **Load.** `graph.txt` is mapped with `mmap` (one small `extern "C"` declaration, no `libc` crate). The box pre-warms `/data` into the page cache, so the mapping costs no memory of ours, where reading the file into a `Vec` would cost 92 MB. The edge lines are split into one chunk per core at newline boundaries and parsed in parallel with a hand-written decimal loop.
2. **Build.** Rows are owned in blocks, one block per thread. While parsing, each thread drops every edge into a bucket for the owner of its tail. Each owner then counts and places only its own edges into its slice of one forward CSR (`first` offsets plus `(neighbour, weight)` pairs), so the build needs no atomics and each thread's random writes stay in a cache-sized slice. The input edges are shuffled, so this bucketing is what makes an owner-per-range build possible.
3. **Relabel.** Node ids are a random shuffle, so a search step in input ids is a cache miss on almost every edge. Each thread grows a breadth-first region from its own root (nodes claimed with an atomic swap), and the regions are laid out one after another, so nodes that are close in the graph get ids that are close in memory. The forward CSR is rewritten in that order with parallel row copies, and the reverse CSR is built from it with the same owner-bucket scheme.
4. **Query.** Every core takes queries from an atomic counter and runs a bidirectional Dijkstra (radix heap, forward search on the graph, backward search on the reversed graph), stopping when the two frontier minima add up to the best meeting distance found. The side with the *smaller heap* expands next, not the side with the smaller distance: from an island whose only exits weigh about 2^32, distance balancing lets the other side settle all 1.7M nodes before the island side runs dry (four such queries took 0.7 to 0.9 s each); size balancing ends them in a dozen steps.
5. **Memory.** A search's tentative distances live in a small open-addressing hash map per side per thread (linear probing, multiplicative hash), not in a dense array of all 1.7M nodes. Dense arrays would cost 12 threads x 2 sides x 14 MB = 336 MB on the box, more than the whole graph; a search touches about 10^4 nodes. Locally this cut peak memory from 415 MB to 297 MB for about 15% more time, a net gain under the scoring formula (time weighs 0.5, memory 0.2, and the box runs 12 threads, not 8).
6. **Output.** Answers go into one string, written once, in query order. Distances are `u64` (answers can exceed 2^32 - 1).

## Dependencies

None.

## With another 24 hours

- ALT landmarks or a light contraction hierarchy: with 2,000 queries the preprocessing has to pay back in under ~100 ms, so it needs measuring on the box first.
- Order the queries so the longest ones start first, to shorten the tail of the query phase (single queries take up to ~25 ms).
- Try a cheaper locality order than BFS regions, or drop the relabel for the reverse graph if the box's caches make it pay less.
