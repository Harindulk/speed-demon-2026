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
| 10 | Multiplicative hash instead of the low id bits | run-23a4b3d65854 (with 14) | queries 165-185 → 125-140 ms | | |
| 11 | Check meetings only when a label improves; never push a node at >= best | run-23a4b3d65854 (with 14) | queries best-of-3 130 → 122 ms | | |
| 12 | BFS relabelling from one root per thread (atomic claims), regions laid out in turn | run-23a4b3d65854 (with 14) | build+BFS ~140 → ~75 ms; total best 330 → 251 ms | | |
| 13 | Tried: CSR build split by tail-id range across threads (no atomics). Reverted | | build+BFS 102-131 vs 133-247 ms, no win | | |
| 14 | Radix heap instead of `BinaryHeap` (committed 9df7c6d with steps 10-12) | run-23a4b3d65854 (box, steps 10-14 together: T 0.507 → 0.405 s, M 210.5, S 402, 53.7 → 55.4 pts) | queries best-of-6 115 → 79 ms; total 244 → 215 ms | | |
| 15 | Tried: range-partitioned build for the forward CSR only, reverse build kept serial. Reverted | | build+BFS best 93 → 142 ms, slower | | |
| 16 | Tried: `MAP_POPULATE` on the input mapping. Reverted | | parse 20-26 → 33-37 ms, total no better | | |
| 17 | Tried: parallel degree gather in the relabel. Reverted | | relabel 38-45 vs 43-54 ms, no win under load | | |
| 18 | Tried: no reverse CSR in input ids; reverse = transpose of the relabelled forward CSR. Reverted | | harness T 0.238 → 0.268 s, M 303 → 258 MB | | |
| 19 | Owner-bucketed parallel CSR build: parse threads bucket edges by tail owner, each owner builds its rows; reverse CSR = bucketed transpose of the relabelled forward CSR | run-c0e7c2523de5 | forward build 65 → 23 ms; harness T 0.208 → 0.178 s, M 303 → 372 MB | | |
| 20 | Owners write straight into one shared CSR (split slices) instead of per-owner blocks concatenated after | run-c0e7c2523de5 | harness T 0.208 → 0.173 s (best of 2x5 vs 2x5), M 303 → 326 MB | | |
| 21 | Tried: SWAR parse of up to 7 digits per 8-byte word. Reverted | | parse 23-28 ms either way | | |
| 22 | Tried: parallel inversion of the BFS order into labels. Reverted | | BFS 23-26 → 19-22 ms, total best 166 vs 178 ms, not a clear win | | |
| 23 | Tried: dense per-thread distance arrays again, now with the radix heap. Reverted | | queries 75-87 vs 76-90 ms, no faster | | |

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
15. Same idea as 13 for the forward graph alone, so the BFS could start sooner. Every thread still reads all 60 MB of parsed edges, and it was slower.
16. Asking the kernel to map all 23k pages up front moved that work onto one thread inside `mmap`, where the parse threads had been taking the faults in parallel.
17. Computing the new row lengths reads `first[]` at random, so it looked worth splitting across threads. No difference showed through the noise, so the shorter loop stayed.
18. Building the reverse graph from the relabelled forward graph drops one 47 MB CSR and frees the parsed edges sooner (local M -15%), but the transpose lands on the critical path instead of overlapping the BFS (T +12%). Under the K formula (time ^0.5, memory ^0.2) that is a wash locally, and the box's slower serial speed tips it to a loss, so it was not kept.
19. The input edges are shuffled, so no thread can own a range of tails without first grouping the edges. Each parse thread now drops each edge into one bucket per owner as it parses, and each owner counts and places only its own edges into rows that fit in its cache: no atomics, no thread reads edges it does not own (unlike steps 13 and 15). The forward build fell from 65 to 23 ms. The reverse graph is the same build fed from the relabelled forward graph, so the separate reverse build in input ids and its relabel are gone.
20. Step 19 built each owner's rows into its own vectors and then copied them into one CSR, an extra 40 MB alive at the peak. Counting first and then splitting one shared edge array between the owners removes the copy: memory back down to +8% over step 14 for 17% less time, a net gain under the K formula (time ^0.5, memory ^0.2).
21. Parsing 92 MB took 23-28 ms on 8 threads, so the per-byte digit loop looked like the limit. A SWAR version (find the separator with one mask, combine digit pairs, quads and halves with three multiplies) passed every seed but did not change the parse time. The phase is bound by page faults and bucket writes, not digit conversion, so the simple loop stayed.
22. Writing `label[order[k]] = k` is 1.7M random writes on one thread. Splitting it across threads with atomic stores saved about 3 ms of BFS time, too small to show in the total, so it was not worth 12 more lines.
23. A check that the hash map from step 8 is not costing time now that the heap is faster: dense arrays with a touched-list reset gave the same query time, so the map keeps its ~170 MB memory saving at no time cost.
| 21b | Board check of steps 19-20: run-c0e7c2523de5 at f885f9b, 0.2767 s / 244.8 MB / S 441 / 58.5 pts, against 0.4046 s / 210.5 MB / 55.4 pts for fc1e03f | run-c0e7c2523de5 | 0.2767 | 244.8 | kept: 32% faster on the box for +34 MB and +39 lines, +3.2 points |
| 21 | Drop the input-id CSR and the BFS order before the transpose (shadowing kept them alive); size the transpose buckets exactly from a count pass | | harness best-of-5 181 → 174 ms (same) | harness 311 → 252 MB (heap ~219 → ~160) | |
