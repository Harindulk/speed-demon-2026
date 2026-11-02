//! Track C (Join): parse the hostile records.csv, join each record to its
//! country through regions.csv, and write one line of aggregates per country.
//! specs/c-join.md is the contract.

use std::fmt::Write;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::thread;

const BOM: &[u8] = b"\xEF\xBB\xBF";
/// Group slot for records whose region_id is not in regions.csv.
const UNMATCHED: u8 = u8::MAX;

/// Maps a whole input file read-only. /data is already in the page cache,
/// so mapped pages cost the run no memory, unlike reading into a buffer.
fn map(path: &str) -> &'static [u8] {
    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    }
    const PROT_READ: i32 = 1;
    const MAP_PRIVATE: i32 = 2;
    let file = File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let len = file.metadata().unwrap().len() as usize;
    if len == 0 {
        return &[];
    }
    // SAFETY: a private read-only mapping of an input nobody writes during the
    // run. It is never unmapped, so the slice really is 'static.
    unsafe {
        let p = mmap(std::ptr::null_mut(), len, PROT_READ, MAP_PRIVATE, file.as_raw_fd(), 0);
        assert!(p as isize != -1, "mmap {path} failed");
        std::slice::from_raw_parts(p, len)
    }
}

/// Running totals for one output line.
#[derive(Clone, Copy)]
struct Agg {
    count: u64,
    id_sum: u64,
    amount_sum: i64,
    amount_min: i64,
    amount_max: i64,
    name_bytes: u64,
    note_bytes: u64,
}

impl Agg {
    fn new() -> Self {
        Agg {
            count: 0,
            id_sum: 0,
            amount_sum: 0,
            amount_min: i64::MAX,
            amount_max: i64::MIN,
            name_bytes: 0,
            note_bytes: 0,
        }
    }

    fn merge(&mut self, o: Agg) {
        self.count += o.count;
        self.id_sum += o.id_sum;
        self.amount_sum += o.amount_sum;
        self.amount_min = self.amount_min.min(o.amount_min);
        self.amount_max = self.amount_max.max(o.amount_max);
        self.name_bytes += o.name_bytes;
        self.note_bytes += o.note_bytes;
    }
}

/// Room for distinct names. The generator draws names from a fixed pool of
/// 100,000 (plus a few edge-case names), so this keeps the table under 80% full.
const SLOTS: usize = 1 << 17;

/// Distinct names per group in about 5 MB, shared by all threads: an
/// open-addressing set of name hashes gives every name a slot, and each group
/// keeps one bit per slot. Holding hashes per record instead cost ~50 MB.
struct Names {
    slots: Vec<AtomicU64>,
    seen: Vec<AtomicU64>,
}

impl Names {
    fn new() -> Self {
        let zeros = |n| (0..n).map(|_| AtomicU64::new(0)).collect();
        Names { slots: zeros(SLOTS), seen: zeros(256 * SLOTS / 64) }
    }

    /// Records the name with hash `h` (from `hash`) in `group`.
    fn add(&self, group: u8, h: u64) {
        let h = h | 1; // 0 marks an empty slot
        let mut i = (h >> 40) as usize % SLOTS;
        for _ in 0..SLOTS {
            // A plain load first: after warm-up almost every name is already in.
            let v = match self.slots[i].load(Relaxed) {
                0 => self.slots[i].compare_exchange(0, h, Relaxed, Relaxed).unwrap_or_else(|v| v),
                v => v,
            };
            if v == 0 || v == h {
                break;
            }
            i = (i + 1) % SLOTS;
        }
        assert_eq!(self.slots[i].load(Relaxed), h, "more than {SLOTS} distinct names");
        let bit = group as usize * SLOTS + i;
        self.seen[bit / 64].fetch_or(1 << (bit % 64), Relaxed);
    }

    fn distinct(&self, group: usize) -> u32 {
        let row = &self.seen[group * SLOTS / 64..(group + 1) * SLOTS / 64];
        row.iter().map(|w| w.load(Relaxed).count_ones()).sum()
    }
}

/// 64-bit hash of a name, for counting distinct names.
/// shortcut: distinct counts assume no 64-bit collision among ~100k names
/// (odds ~1e-9); compare the bytes if that ever matters.
fn hash(s: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut h = s.len() as u64 ^ K;
    for chunk in s.chunks(8) {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(K).rotate_left(31);
    }
    (h ^ (h >> 29)).wrapping_mul(K)
}

/// Reads `[0-9]+` and the byte after it (a comma).
fn digits(b: &[u8], i: &mut usize) -> u64 {
    let mut v = 0;
    while b[*i].is_ascii_digit() {
        v = v * 10 + (b[*i] - b'0') as u64;
        *i += 1;
    }
    *i += 1;
    v
}

/// Reads a name or note field and the comma after it. Returns the raw value
/// (still escaped, which maps one to one onto the unescaped value) and the
/// unescaped byte length.
fn text<'a>(b: &'a [u8], i: &mut usize) -> (&'a [u8], u64) {
    let start = *i;
    if b[start] != b'"' {
        while b[*i] != b',' {
            *i += 1;
        }
        *i += 1;
        return (&b[start..*i - 1], (*i - 1 - start) as u64);
    }
    let mut j = start + 1;
    let mut escapes = 0;
    loop {
        j = next_quote(b, j);
        if b.get(j + 1) == Some(&b'"') {
            escapes += 1;
            j += 2;
        } else {
            break;
        }
    }
    *i = j + 2;
    let raw = &b[start + 1..j];
    (raw, raw.len() as u64 - escapes)
}

/// Index of the next `"` at or after `j`, testing eight bytes per step.
fn next_quote(b: &[u8], mut j: usize) -> usize {
    const ONES: u64 = 0x0101_0101_0101_0101;
    while j + 8 <= b.len() {
        let w = u64::from_le_bytes(b[j..j + 8].try_into().unwrap()) ^ (ONES * b'"' as u64);
        // Nonzero high bit at each zero byte of w; the lowest one is exact.
        let zero = w.wrapping_sub(ONES) & !w & (ONES << 7);
        if zero != 0 {
            return j + zero.trailing_zeros() as usize / 8;
        }
        j += 8;
    }
    j + b[j..].iter().position(|&c| c == b'"').expect("unclosed quote")
}

/// Reads `-?[0-9]+\.[0-9]{2}` as integer cents.
fn cents(b: &[u8], i: &mut usize) -> i64 {
    let negative = b[*i] == b'-';
    *i += negative as usize;
    let mut v = 0i64;
    while b[*i] != b'.' {
        v = v * 10 + (b[*i] - b'0') as i64;
        *i += 1;
    }
    v = v * 100 + ((b[*i + 1] - b'0') * 10 + (b[*i + 2] - b'0')) as i64;
    *i += 3;
    if negative {
        -v
    } else {
        v
    }
}

/// Aggregates the records in `b[start..end]`; both ends are record starts.
fn parse(b: &[u8], mut i: usize, end: usize, group: &[u8], names: &Names) -> [Agg; 256] {
    let mut aggs = [Agg::new(); 256];
    // Name lookups are random memory accesses; doing them in batches lets the
    // CPU overlap their cache misses instead of waiting on each in turn.
    let mut batch = [(0u8, 0u64); 32];
    let mut pending = 0;
    while i < end {
        if b[i..].starts_with(BOM) {
            i += 3;
        }
        let id = digits(b, &mut i);
        let (name, name_len) = text(b, &mut i);
        let (_, note_len) = text(b, &mut i);
        let region = digits(b, &mut i) as usize;
        let amount = cents(b, &mut i);
        i += (b.get(i) == Some(&b'\r')) as usize + 1; // CRLF or LF (or end of file)

        let g = *group.get(region).unwrap_or(&UNMATCHED);
        batch[pending] = (g, hash(name));
        pending += 1;
        if pending == batch.len() {
            batch.iter().for_each(|&(g, h)| names.add(g, h));
            pending = 0;
        }
        let a = &mut aggs[g as usize];
        a.count += 1;
        a.id_sum += id;
        a.amount_sum += amount;
        a.amount_min = a.amount_min.min(amount);
        a.amount_max = a.amount_max.max(amount);
        a.name_bytes += name_len;
        a.note_bytes += note_len;
    }
    batch[..pending].iter().for_each(|&(g, h)| names.add(g, h));
    aggs
}

/// Next record start at or after `p`. Only notes hold LFs, and a note line
/// never starts with a digit or a BOM (spec 2.2), while every record does.
fn record_start(b: &[u8], mut p: usize) -> usize {
    while let Some(lf) = b[p..].iter().position(|&c| c == b'\n') {
        p += lf + 1;
        if p >= b.len() || b[p].is_ascii_digit() || b[p] == BOM[0] {
            return p;
        }
    }
    b.len()
}

/// regions.csv rows as (region_id, country code read as a base-26 number,
/// whose numeric order is the codes' bytewise order).
fn regions(b: &[u8]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut i = b.iter().position(|&c| c == b'\n').unwrap() + 1;
    std::iter::from_fn(move || {
        (i < b.len()).then(|| {
            let id = digits(b, &mut i) as usize;
            let code = b[i..i + 3].iter().fold(0, |v, &c| v * 26 + (c - b'A') as usize);
            i += 4;
            (id, code)
        })
    })
}

/// regions.csv -> (group index per region_id, sorted country codes).
fn load_regions(b: &[u8]) -> (Vec<u8>, Vec<[u8; 3]>) {
    let mut index = vec![UNMATCHED; 26 * 26 * 26];
    for (_, code) in regions(b) {
        index[code] = 0;
    }
    let mut codes = Vec::new();
    for (code, slot) in index.iter_mut().enumerate().filter(|(_, s)| **s == 0) {
        *slot = codes.len() as u8;
        let letter = |k: usize| b'A' + (code / k % 26) as u8;
        codes.push([letter(676), letter(26), letter(1)]);
    }
    assert!(codes.len() < UNMATCHED as usize, "too many countries");
    let mut group = Vec::new();
    for (id, code) in regions(b) {
        if group.len() <= id {
            group.resize(id + 1, UNMATCHED);
        }
        group[id] = index[code];
    }
    (group, codes)
}

fn money(out: &mut String, c: i64) {
    let sign = if c < 0 { "-" } else { "" };
    let a = c.unsigned_abs();
    write!(out, " {sign}{}.{:02}", a / 100, a % 100).unwrap();
}

fn main() {
    let (group, codes) = load_regions(map("/data/regions.csv"));
    let b = map("/data/records.csv");

    let mut start = if b.starts_with(BOM) { 3 } else { 0 };
    start += b[start..].iter().position(|&c| c == b'\n').unwrap() + 1; // header
    let threads = thread::available_parallelism().map_or(1, |n| n.get());
    let mut cuts = vec![start];
    for k in 1..threads {
        let prev = *cuts.last().unwrap();
        cuts.push(record_start(b, prev.max(b.len() * k / threads)).max(prev));
    }
    cuts.push(b.len());

    let names = Names::new();
    let mut total = [Agg::new(); 256];
    thread::scope(|s| {
        let parts: Vec<_> = cuts
            .windows(2)
            .map(|w| {
                let (lo, hi, group, names) = (w[0], w[1], &group, &names);
                s.spawn(move || parse(b, lo, hi, group, names))
            })
            .collect();
        for part in parts {
            for (t, a) in total.iter_mut().zip(part.join().unwrap()) {
                t.merge(a);
            }
        }
    });

    let mut out = String::new();
    let keys = codes.iter().map(|c| std::str::from_utf8(c).unwrap());
    let groups = (0..codes.len()).chain([UNMATCHED as usize]);
    for (key, g) in keys.chain(["_UNMATCHED"]).zip(groups) {
        let a = &total[g];
        if a.count == 0 {
            continue;
        }
        write!(out, "{key} {} {}", a.count, a.id_sum).unwrap();
        money(&mut out, a.amount_sum);
        money(&mut out, a.amount_min);
        money(&mut out, a.amount_max);
        writeln!(out, " {} {} {}", names.distinct(g), a.name_bytes, a.note_bytes).unwrap();
    }
    std::fs::write("/out/aggregates.txt", out).expect("write /out/aggregates.txt");
}
