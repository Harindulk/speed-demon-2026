# Track B: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

Local numbers: Apple M5 (arm64), Colima 8 vCPUs, seed 1 full. "check.py" is wall time including container start-up; "in-container" is the binary's wall time, best of several runs, with three other tracks building and running on the same VM, so expect ±50% noise. Run IDs come from the live board.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 56.35 | 2,200 | 0 |
| 1 | Rust, mmap, query terms only, 1 chunk per thread, lists concatenated, queries on a shared counter | | local check.py 0.88-3.07 | local anon ~200 | |
| 2 | Each thread answers every query over its own chunk; no list merge | | local check.py 1.13-1.71 | | |
| 3 | Term table rebuilt at its final size (8 MB -> 1 MB) | | local in-container 537 ms vs 751 ms before | | |
| 4 | Bitsets for terms in > 1/16 of a chunk; BOOL on list-or-bitset running set | | local, within noise | | |
| 5 | Phrase check streams tokens through an 8-slot ring, no `Vec` per document, early exit | | local, within noise | | |
| 6 | Lists turn into bitsets during indexing at 1/32 of the chunk; per-query partials are fixed arrays | | local in-container 437-701 ms | local anon peak ~155 | |
| 7 | Index phase, then queries in 256-query blocks over all chunks (no 50k x threads partials kept) | | local in-container 438-486 ms | local anon peak 137 | |
| 8 | Bitsets kept in one side vector, each term holds a 4-byte index instead of a 24-byte empty `Vec` | | local in-container 409-428 ms | local anon peak 125 | |
| 9 | Chunk-local document ids as `u16` (chunks capped at 32 MB, asserted <= 65,536 docs) | | local in-container 435-439 ms | local anon peak 94 | |

Why each step:

1. The query terms are known up front, so indexing only them (76k terms) skips most of the corpus' 610k-term vocabulary; mmap keeps the 254 MB input off the heap.
2. Chunks are consecutive id ranges, so the per-chunk answers combine in order; this removed the 22M-entry concatenation (60-130 ms locally) and its second copy.
3. With 36M tokens each doing one lookup, the table is the hot random access; at 0.29 load it fits in L2.
4. About 3% of BOOL terms are in > 10% of documents; walking their lists dominated set operations.
5. Profiling by query type showed PHRASE at ~70% of query CPU; per-candidate token `Vec`s were the overhead.
6. Converting during indexing means frequent terms never hold a long list at all, cutting the peak heap.
7. Storing 50,000 answers per thread cost ~34 MB on 12 threads; answering block by block also balances load across threads.
8. 76k terms x 12 chunks of empty `Vec<u64>` headers were ~22 MB on the box for the ~370 terms per chunk that ever need a bitset.
9. A chunk holds ~21k documents on 12 threads, so a 16-bit offset from the chunk's first id is enough; it halves the posting lists, the largest remaining allocation. Memory scores as min(M_ref/M, 50)^0.2, so 125 -> 94 MB is worth about +1.2 points at no time cost.
