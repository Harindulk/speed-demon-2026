# Track B: Index

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/b-index.md).

## Approach

Rust, standard library only, one file (`src/main.rs`).

1. **Queries first.** `queries.txt` is parsed before the corpus. Every distinct term (about 76,000 at full scale, out of 610,000 corpus tokens) gets an id in a small open-addressing table, rebuilt at its final size so it stays in L2 while the corpus is scanned.
2. **Index only what is asked.** `docs.txt` is memory-mapped (the box pre-warms it into the page cache, so it costs no heap) and cut into one chunk of whole lines per thread. Each thread tokenizes its chunk with a 256-entry byte table (ASCII lowercase, everything else a separator), hashes each token while scanning it, and records the document id only if the token is a query term.
3. **Two posting shapes.** Inside a chunk a document is its 16-bit offset from the chunk's first id (chunks are capped at 32 MB, ~21k documents each on 12 threads). A term's documents in a chunk are a sorted `Vec<u16>` while it is rare. Once it appears in more than 1/32 of the chunk's documents, the list is converted to a bitset, which is then the smaller of the two, and later documents just set bits. Words like `the` never grow a 250,000-entry list.
4. **No merge.** Chunks hold consecutive id ranges, so a query's answer is just the chunks' answers in order: counts and sums add, and the ten smallest ids are the first ten found walking the chunks. Posting lists are never concatenated or shared between threads.
5. **Queries in parallel.** Workers take blocks of 256 queries from an atomic counter and answer each over every chunk.
   - `BOOL` runs strictly left to right on a running set that is a sorted list or, once a frequent term is OR-ed in, a bitset. AND/NOT against a frequent term test bits; against a rare term they merge, or binary-search when one side is 32x smaller; bitset-with-bitset is word-wise `&`, `&!`, `|`.
   - `PHRASE`: while indexing, every adjacent pair of query terms that occurs in some phrase (about 45k pairs) is indexed exactly like a term. A phrase's candidates are the documents holding all of its pairs; for two-term phrases that is the answer. Longer phrases re-tokenize each candidate straight from the mapped bytes through an 8-slot ring and stop at the first match. No positions are stored.
6. Results are written with one `write_all`.

The only `unsafe` is the `mmap` call, declared with `extern "C"` so no crate is needed.

## Dependencies

None.

## With another 24 hours

- Delta/varint-compress the rare-term lists and allocate per-term lists only for terms a chunk contains, to get from ~94 MB toward the 44 MB at which the memory factor caps.
- Profile on x86 with `perf`: local timing here was on an arm64 Mac under Colima, where virtiofs page faults dominate the indexing phase and runs varied 2x between repeats.
- Tokenize with AVX2 class masks (32 bytes at a time) instead of a byte loop.
