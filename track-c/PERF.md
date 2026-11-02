# Track C: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

Local numbers are from this Mac's Colima VM (8 arm64 vCPUs), timing `/solution` inside the container on seed 1 while three other tracks were building and running, so they are noisy and only good for before/after. "M local" is the cgroup's peak minus the 204 MB of mapped input that the VM charges to the run (the board pre-warms `/data`, so it does not), and includes the shell that timed it. Every step passed `tools/verify.sh c` on seeds 1 to 4.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 14.44 | 367 | 0 |
| 1 | Rust rewrite: mmap, 12-thread split at record starts, byte state machine, per-country `Vec` of name hashes sorted at the end | | local 0.11 | local ~46 | |
| 2 | Sort and dedupe each country's name hashes in parallel | | local: final sort 25 → ~10 ms | | |
| 3 | Shared name-hash set plus one bit per (country, name) instead of a hash per record (removes step 2's sort) | | local ~0.11 (CPU up 2x) | local ~9 | |
| 4 | Plain load before compare-and-swap in the name set | | local CPU 0.45 → 0.41 s | | |
| 5 | SWAR search for the closing quote, eight bytes per step | | local 0.09, CPU 0.41 → 0.33 s | | |
| 6 | Region loader: two byte scans with a base-26 code table instead of split + binary search | | local: regions 21 → 4 ms | | |
| 1-6 | All of the above, as committed (board, commit 3ce2b62) | run-d53c8620b124 | 0.0681 | 20.1 | 67.3 |

Why each step:

1. Correct first. Section 2.2 of the spec lets threads find record starts without a sequential quote pass (only notes hold LFs, and note lines never start with a digit or a BOM), which is what makes the parse parallel.
2. After step 1 the only serial work left was sorting 3.3M hashes; splitting it across threads cut it to the cost of one country's share.
3. Memory weighs 0.2 in K and is capped at M_ref / 50 = 7.3 MB. Step 1's ~46 MB of hashes put us at ratio 8 (0.2 power: 1.52); about 7-9 MB is near the cap (2.19), worth roughly 8 points, for more CPU but the same wall time on 12 threads.
4. After the first few thousand records almost every name is already in the set, so most locked compare-and-swaps were wasted.
5. Notes average about 30 bytes; the byte-at-a-time scan for `"` was the hottest loop left in the parser.
6. Region loading runs before any thread starts, so its time adds straight to the total.

Tried and dropped: `MAP_POPULATE` on the records mapping. Locally it was slower (the kernel faults pages in one thread before the parse starts, through the VM's virtiofs), and the box result is unknown.

After the first commit:

| Step | Change | Run ID | T (s) | M (MB) | Kept? |
|---|---|---|---|---|---|
| 7 | Measured memory: the cgroup peak minus mapped input minus the idle shell's peak leaves ~4.5 MB for `/solution` (3 runs: 4.40, 4.46, 4.57 MB) | | | local ~4.5 | no code change. The board then said 20.1 MB (run-d53c8620b124), so the local number misses something on the box |
| 8 | Test the bitset word with a plain load and skip `fetch_or` when the bit is already set | | best of 30: 0.068 → 0.070 | | reverted: no measurable gain (only ~7% of marks are repeats) and two more lines |
| 9 | Re-measured with seed 1 copied into a VM-local volume and read once first, so the input sits in the native page cache as on the box (virtiofs had misled steps 1-8). Phases: regions 2.5 ms, threads 22-48 ms each, output 0.3 ms. Removing the name tracking cut the parse from 38 to 22 ms (3 rounds, best of 7 each); hashing alone is ~2 ms of that, the rest is two random memory accesses per record | | 0.038 (local, warm) | local ~5 | measurement only |
| 10 | Worker threads with 256 KB stacks instead of 2 MB. With 12 threads and transparent huge pages set to `always`, each 2 MB stack can be backed by a whole 2 MB page; that is the one thing that grows with the box's thread count and could explain 20 MB vs ~5 MB local. Local THP is `madvise`, so no local change is expected | | no change | local 5.0 → 3.9-4.7 | kept, needs a board run to confirm |
| 11 | One slot per name holding its hash and a 256-bit group mask, with `fetch_or`'s old value giving the distinct count: one random access per record instead of two, and no final popcount | | best of 7, 3 rounds: 49-52 vs 49-52 ms | local ~4.5 | reverted: no gain, likely because every lookup now reads lines other cores keep writing |
| 12 | `MAP_POPULATE` again, now on native page cache | | 55-73 vs 57-67 ms | | reverted: inside the noise |
