# Track D: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

Local numbers come from an Apple M5 running the official harness in Colima (8 vCPUs, arm64) while three other tracks were building on the same machine, so they are noisy (about ±20%) and are only good for comparing one step with the next. Local M includes about 92 MB of `graph.txt` page cache that the box pre-warms and does not charge. Every step passed seeds 1 to 4 against the reference hashes before it was kept.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 72.69 | 893 | 0 |
| 1 | Rust, mmap input, parallel parse, forward CSR, one-way Dijkstra on every core | | local ~0.9 (parse 0.1, queries 0.8) | | |
| 2 | Bidirectional Dijkstra on a reverse CSR, balanced by distance | | local ~0.9 | | |
| 3 | Balance the two searches by heap size instead | | local queries 0.8 → ~0.35 | | |
| 4 | BFS relabelling of node ids, both CSRs rewritten in BFS order | | local 0.75 (harness, contended) | local 428 | |
| 5 | Distance arrays start as zero pages (bitwise-inverted storage) | | local 0.73 | local 412 | |
| 6 | Parallel row copies for the relabel; BFS overlapped with the reverse build | | local 0.67 contended, 0.29-0.31 quiet | local 415 | |
| 7 | Tried: CSR build with atomic counters on every core. Reverted | | local 0.30-0.33 vs 0.29-0.33 | | |
| 8 | Per-search hash map for distances instead of dense per-thread arrays (committed 3ce2b62) | run-825573299ad2 (box: T 0.507, M 204.6, S 332, 53.7 pts) | local 0.35-0.38 vs 0.31 | local 297 vs 415 | |
| 9 | Tried: key and distance interleaved in one hash slot. Reverted | | queries ~200 ms either way | | |
| 10 | Multiplicative hash instead of the low id bits | | queries 165-185 → 125-140 ms | | |
| 11 | Check meetings only when a label improves; never push a node at >= best | | queries best-of-3 130 → 122 ms | | |
| 12 | BFS relabelling from one root per thread (atomic claims), regions laid out in turn | | build+BFS ~140 → ~75 ms; total best 330 → 251 ms | | |
| 13 | Tried: CSR build split by tail-id range across threads (no atomics). Reverted | | build+BFS 102-131 vs 133-247 ms, no win | | |
| 14 | Radix heap instead of `BinaryHeap` | | queries best-of-6 115 → 79 ms; total 244 → 215 ms | | |

Why each step:

1. Parsing is cheap (92 MB over every core); the 2,000 searches are the cost, mean Dijkstra rank about 12,300 nodes. mmap keeps the input off our memory bill.
2. Two searches that meet in the middle settle far fewer nodes on a road network. The gain was hidden by step 3's problem.
3. Timing every query showed four queries at 0.7 to 0.9 s each, all island sources: the island's only exits weigh about 2^32, so with distance balancing the backward side settled all 1.74M nodes before the island side ran dry. Expanding the side with the smaller heap ends those in a dozen steps and is just as correct (the stop test does not depend on the alternation rule).
4. Input ids are a random shuffle, so almost every edge relaxation was a cache miss. BFS order puts graph neighbours near each other in memory; the query phase got about 3x faster (0.4 → 0.14 s) for about 0.15 s of extra build, which step 6 cut back.
5. `vec![u64::MAX; n]` writes every page up front; storing `!d` makes "unreached" 0, so untouched pages are never committed. Small win (16 MB), since searches touch most pages over a run.
6. The relabel copies whole rows (streaming, not scattering), so it splits cleanly into per-thread blocks of new ids; the BFS only needs the forward graph, so it runs while the reverse graph is built.
7. Atomic degree counts and an atomic cursor per row let every core build the CSRs, but locally it was no faster and the code was longer, so it was reverted (no evidence, no complexity).
8. Dense distance arrays cost 2 x 14 MB per thread (336 MB at 12 threads), more than the graph itself, while one search touches about 10^4 nodes. A linear-probing map sized to the search took local M from 415 to 297 MB for about 15% more time. With the scoring weights (time ^0.5, memory ^0.2) and 12 threads on the box, that is a net gain of roughly 1.5 to 2 points.
9. One (key, distance) array instead of two parallel arrays, hoping for one cache line per probe. No measurable change, so the simpler layout stayed.
10. With BFS ids, the low bits put the runs of nearby ids from different BFS rings into the same slots, and linear probing then walks long clusters. A multiplicative hash spreads them; the query phase dropped about 25%.
11. A node that has not improved needs no meeting check: whichever side labelled it last already compared against the other side's label. Nodes at or beyond the best meeting distance can never be on a shorter path, so they are not pushed.
12. The board's box runs serial code slower than this Mac (board 0.507 s vs 0.35 s locally), so the single-threaded BFS (~70 ms locally) was the biggest serial step. Each thread now grows its own BFS region from its own root and claims nodes with an atomic swap; regions are laid out one after another, and unreached nodes (islands, no-in nodes) go last. Locality is as good inside a region, and the step runs on every core.
13. Each thread scanned all edges and handled only rows in its id range, so writes stayed cache-local without atomics. Every thread reads all 60 MB of parsed edges, and with the reverse build running at the same time it was not faster locally.
14. Dijkstra keys only grow, so a radix heap needs no comparisons between entries: each entry sits in the bucket of the highest bit where it differs from the last popped key and moves down at most 64 times.
