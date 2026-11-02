//! Track A (Ingest): per (UTC hour, endpoint) request count, nearest-rank p50/p95/p99
//! latency and 5xx error count/ppm, from /data/requests.log into /out/summary.txt.
//!
//! Four parallel phases over a memory-mapped input:
//! 1. parse: each thread parses one newline-aligned chunk into packed records,
//!    interning endpoints locally;
//! 2. rank: endpoints are sorted bytewise and hours chronologically, so a dense
//!    group id `hour_rank * endpoints + endpoint_rank` is already in output order;
//! 3. scatter: a counting sort places every latency in its group's slice;
//! 4. report: threads select percentiles and format disjoint ranges of groups.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::thread;

/// Hour keys are `(month * 32 + day) * 24 + hour`, which sort chronologically within a year.
const HOUR_KEYS: usize = 13 * 32 * 24;

/// A record packs hour key (14 bits), 5xx flag (1), local endpoint id (17) and latency (32).
fn pack(hour: usize, error: bool, endpoint: usize, latency: u32) -> u64 {
    (hour as u64) << 50 | (error as u64) << 49 | (endpoint as u64) << 32 | latency as u64
}

/// Hashes an endpoint by its first and last 8 bytes (plus the length, which `Hash` for
/// slices writes first): two loads per line instead of a pass over every byte. Endpoints
/// are trusted ASCII, so SipHash's DoS resistance buys nothing here.
#[derive(Default)]
struct FastHash(u64);

impl Hasher for FastHash {
    fn write(&mut self, bytes: &[u8]) {
        let word = |b: &[u8]| {
            let mut w = [0u8; 8];
            w[..b.len()].copy_from_slice(b);
            u64::from_le_bytes(w)
        };
        let n = bytes.len();
        let (head, tail) = if n >= 8 { (word(&bytes[..8]), word(&bytes[n - 8..])) } else { (word(bytes), 0) };
        for w in [head, tail] {
            self.0 = (self.0 ^ w).wrapping_mul(0x9e3779b97f4a7c15);
        }
    }
    fn finish(&self) -> u64 {
        self.0 ^ self.0 >> 29
    }
}

struct Chunk<'a> {
    endpoints: Vec<&'a [u8]>,
    records: Vec<u64>,
}

fn two_digits(b: &[u8]) -> usize {
    ((b[0] - b'0') * 10 + (b[1] - b'0')) as usize
}

/// Index of the first space at or after `from`, eight bytes at a time (SWAR): XOR with
/// spaces turns each space into a zero byte, and the classic has-zero-byte test finds it.
fn find_space(data: &[u8], mut from: usize) -> usize {
    const ONES: u64 = 0x0101_0101_0101_0101;
    while let Some(word) = data.get(from..from + 8) {
        let x = u64::from_le_bytes(word.try_into().unwrap()) ^ (ONES * b' ' as u64);
        let zeros = x.wrapping_sub(ONES) & !x & (ONES << 7);
        if zeros != 0 {
            return from + (zeros.trailing_zeros() / 8) as usize;
        }
        from += 8;
    }
    from + data[from..].iter().position(|&b| b == b' ').unwrap()
}

fn parse_chunk(data: &[u8]) -> Chunk<'_> {
    let mut ids: HashMap<&[u8], usize, BuildHasherDefault<FastHash>> = HashMap::default();
    let mut chunk = Chunk { endpoints: Vec::new(), records: Vec::with_capacity(data.len() / 70) };
    let mut pos = 0;
    while pos < data.len() {
        let ts = &data[pos..pos + 27];
        let hour = (two_digits(&ts[5..7]) * 32 + two_digits(&ts[8..10])) * 24 + two_digits(&ts[11..13]);
        // Skip "<ts> " and the method (3 to 7 bytes) to the endpoint.
        let mut start = pos + 31;
        while data[start] != b' ' {
            start += 1;
        }
        start += 1;
        let end = find_space(data, start + 1);
        let endpoint = &data[start..end];
        let error = data[end + 1] == b'5';
        let mut latency = 0u32;
        pos = end + 5;
        while data[pos] != b'\n' {
            latency = latency * 10 + (data[pos] - b'0') as u32;
            pos += 1;
        }
        pos += 1;
        let next_id = chunk.endpoints.len();
        let id = *ids.entry(endpoint).or_insert_with(|| {
            chunk.endpoints.push(endpoint);
            next_id
        });
        chunk.records.push(pack(hour, error, id, latency));
    }
    chunk
}

/// Splits `data` into about `parts` pieces, each ending just after a newline.
fn split_lines(data: &[u8], parts: usize) -> Vec<&[u8]> {
    let mut pieces = Vec::with_capacity(parts);
    let mut rest = data;
    for i in (1..=parts).rev() {
        if rest.is_empty() {
            break;
        }
        let mut cut = (rest.len() / i).max(1);
        while cut < rest.len() && rest[cut - 1] != b'\n' {
            cut += 1;
        }
        let (piece, tail) = rest.split_at(cut);
        pieces.push(piece);
        rest = tail;
    }
    pieces
}

/// Maps the input read-only so its (pre-warmed) page cache is shared, not copied.
fn map_input(path: &str) -> std::io::Result<&'static [u8]> {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    }
    const PROT_READ: i32 = 1;
    const MAP_PRIVATE: i32 = 2;
    let file = File::open(path)?;
    let len = file.metadata()?.len() as usize;
    if len == 0 {
        return Ok(&[]);
    }
    // SAFETY: a private read-only mapping of a file nobody writes during the run; it is
    // never unmapped, so the 'static slice stays valid. The fd may close after mmap.
    let ptr = unsafe { mmap(std::ptr::null_mut(), len, PROT_READ, MAP_PRIVATE, file.as_raw_fd(), 0) };
    if ptr as isize == -1 {
        return Ok(std::fs::read(path)?.leak());
    }
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

fn push_num(out: &mut Vec<u8>, mut n: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i..]);
}

/// Nearest-rank p50, p95 and p99: the values at 1-indexed rank ceil(p * n / 100).
/// Selecting p50 first leaves everything above it at the back, so p95 only searches
/// the top half and p99 only the top 5%.
fn percentiles(values: &mut [u32]) -> [u32; 3] {
    let rank = |p: usize| (p * values.len() + 99) / 100 - 1;
    let (r50, r95, r99) = (rank(50), rank(95), rank(99));
    let p50 = *values.select_nth_unstable(r50).1;
    let p95 = *values[r50..].select_nth_unstable(r95 - r50).1;
    let p99 = *values[r95..].select_nth_unstable(r99 - r95).1;
    [p50, p95, p99]
}

fn main() -> std::io::Result<()> {
    let data = map_input("/data/requests.log")?;
    let threads = thread::available_parallelism().map_or(8, |n| n.get());
    let year = data.get(..4).unwrap_or(b"0000");

    // Phase 1: parse.
    let mut chunks: Vec<Chunk> = thread::scope(|s| {
        let handles: Vec<_> =
            split_lines(data, threads).into_iter().map(|p| s.spawn(move || parse_chunk(p))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Phase 2: rank endpoints bytewise and hours chronologically.
    let mut names: Vec<&[u8]> = chunks.iter().flat_map(|c| c.endpoints.iter().copied()).collect();
    names.sort_unstable();
    names.dedup();
    let mut hour_rank = vec![u32::MAX; HOUR_KEYS];
    for c in &chunks {
        for r in &c.records {
            hour_rank[(r >> 50) as usize] = 0;
        }
    }
    let mut hours = Vec::new();
    for (key, rank) in hour_rank.iter_mut().enumerate() {
        if *rank == 0 {
            *rank = hours.len() as u32;
            hours.push(key);
        }
    }
    let groups = hours.len() * names.len();

    // Each thread rewrites its records as (group id, latency) and counts its groups.
    let errors: Vec<AtomicU32> = (0..groups).map(|_| AtomicU32::new(0)).collect();
    let mut cursors: Vec<Vec<u32>> = thread::scope(|s| {
        let handles: Vec<_> = chunks
            .iter_mut()
            .map(|c| {
                let (names, hour_rank, errors) = (&names, &hour_rank, &errors);
                s.spawn(move || {
                    let rank: Vec<usize> =
                        c.endpoints.iter().map(|e| names.binary_search(e).unwrap()).collect();
                    let mut counts = vec![0u32; groups];
                    for r in c.records.iter_mut() {
                        let hour = hour_rank[(*r >> 50) as usize] as usize;
                        let g = hour * names.len() + rank[(*r >> 32 & 0x1_ffff) as usize];
                        counts[g] += 1;
                        if *r >> 49 & 1 == 1 {
                            errors[g].fetch_add(1, Relaxed); // 5xx lines are rare
                        }
                        *r = (g as u64) << 32 | (*r & 0xffff_ffff);
                    }
                    counts
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Phase 3: counting sort. Each thread's counts become its write cursors.
    let mut start = vec![0usize; groups + 1];
    for counts in &cursors {
        for g in 0..groups {
            start[g + 1] += counts[g] as usize;
        }
    }
    for g in 0..groups {
        start[g + 1] += start[g];
    }
    let mut next: Vec<usize> = start[..groups].to_vec();
    for counts in cursors.iter_mut() {
        for (count, at) in counts.iter_mut().zip(next.iter_mut()) {
            (*count, *at) = (*at as u32, *at + *count as usize);
        }
    }
    let slots: Vec<AtomicU32> = (0..start[groups]).map(|_| AtomicU32::new(0)).collect();
    thread::scope(|s| {
        for (c, mut cursor) in chunks.iter().zip(cursors) {
            let slots = &slots;
            s.spawn(move || {
                for r in &c.records {
                    let g = (r >> 32) as usize;
                    slots[cursor[g] as usize].store(*r as u32, Relaxed);
                    cursor[g] += 1;
                }
            });
        }
    });
    drop(chunks);
    let mut latencies: Vec<u32> = slots.into_iter().map(AtomicU32::into_inner).collect();

    // Phase 4: report. Split groups into ranges of about equal record counts.
    let per_part = latencies.len() / threads + 1;
    let mut bounds = vec![0];
    for g in 0..groups {
        if start[g + 1] - start[bounds[bounds.len() - 1]] >= per_part {
            bounds.push(g + 1);
        }
    }
    bounds.push(groups);
    bounds.dedup();
    let outputs: Vec<Vec<u8>> = thread::scope(|s| {
        let mut rest = latencies.as_mut_slice();
        let mut handles = Vec::new();
        for w in bounds.windows(2) {
            let (first, last) = (w[0], w[1]);
            let (mine, tail) = rest.split_at_mut(start[last] - start[first]);
            rest = tail;
            let (start, errors, names, hours) = (&start, &errors, &names, &hours);
            handles.push(s.spawn(move || {
                let mut out = Vec::with_capacity((last - first) * 96);
                for g in first..last {
                    let values = &mut mine[start[g] - start[first]..start[g + 1] - start[first]];
                    if values.is_empty() {
                        continue;
                    }
                    let key = hours[g / names.len()];
                    let (month, day, hour) = (key / 24 / 32, key / 24 % 32, key % 24);
                    out.extend_from_slice(year);
                    for (sep, n) in [(b'-', month), (b'-', day), (b'T', hour)] {
                        out.extend_from_slice(&[sep, b'0' + (n / 10) as u8, b'0' + (n % 10) as u8]);
                    }
                    out.extend_from_slice(b":00:00Z ");
                    out.extend_from_slice(names[g % names.len()]);
                    let count = values.len() as u64;
                    let errs = errors[g].load(Relaxed) as u64;
                    let [p50, p95, p99] = percentiles(values).map(u64::from);
                    for n in [count, p50, p95, p99, errs, errs * 1_000_000 / count]
                    {
                        out.push(b' ');
                        push_num(&mut out, n);
                    }
                    out.push(b'\n');
                }
                out
            }));
        }
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut file = File::create("/out/summary.txt")?;
    outputs.iter().try_for_each(|out| file.write_all(out))
}
