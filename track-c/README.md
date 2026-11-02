# Track C: Join

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/c-join.md).

## Approach

One Rust file, no crates.

- **Inputs are memory-mapped.** `/data` is already in the page cache, so mapped pages cost the run no memory, where reading 200 MB into a buffer would. The mapping is the only `unsafe` block.
- **Regions become a dense array.** `regions.csv` is read twice as plain bytes: once to find which country codes exist (a code read as a base-26 number sorts the same way as its bytes), once to fill a `Vec<u8>` from `region_id` to the country's sorted index. Ids past the end, or holes, map to `_UNMATCHED`. The array is about 1 MB.
- **Records are split across all threads without a quote pre-pass.** Section 2.2 of the spec says only notes contain LFs, and a note line never starts with a digit or a BOM, while every record does. So a thread's start is the first LF followed by a digit or `EF`. Each thread parses its range with a hand-written state machine: optional BOM, `id`, two text fields, `region_id`, `amount` in integer cents, then CRLF or LF.
- **Text fields are never copied.** A quoted field is scanned for its closing quote eight bytes at a time (SWAR), `""` pairs are counted, and the unescaped length is the raw length minus the pairs. The still-escaped bytes map one to one onto the unescaped value, so they serve as the name's identity.
- **Distinct names cost about 5 MB.** All threads share one open-addressing set of 64-bit name hashes (131,072 slots, loads before any compare-and-swap); a name's slot is its id, and each country keeps one bit per slot. The generator draws names from a fixed pool of 100,000, so the set stays under 80% full; it panics rather than loop if that ever stops being true.
- **Per-thread totals** (count, id sum, amount sum/min/max, byte counts) live in a 256-entry array indexed by country and are summed after the threads join. The 241 output lines go out in one write.

## Dependencies

None.

## With another 24 hours

- Count distinct names without atomics: the shared bitset's `fetch_or` and the hash probe are about half the CPU time. Per-thread bitsets would be faster but cost 12 times the memory, so the next try is batching marks per country.
- Parse with AVX2 masks (find every quote, comma and LF in 32-byte blocks) instead of byte loops for the numeric fields.
- Measure `MAP_POPULATE` and huge pages on the scoring box; locally the VM's file sharing made the numbers meaningless.
