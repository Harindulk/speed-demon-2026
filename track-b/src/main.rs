//! Speed Demon 2026, Track B (Index). Spec: specs/b-index.md.
//!
//! Reads the queries first and indexes only the terms they use, so the index holds a
//! fraction of the corpus' (term, document) pairs. The corpus is memory-mapped, split
//! into one chunk per thread at line boundaries, and each thread builds sorted posting
//! lists for its chunk; the lists are concatenated in chunk order. Queries then run in
//! parallel: BOOL with sorted-list set operations, PHRASE by intersecting the terms'
//! postings and re-tokenizing the few candidate documents.

use std::fs::File;
use std::fmt::Write as _;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Maps a file read-only. The page cache backs it, so the input costs no heap memory.
fn map(path: &str) -> &'static [u8] {
    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    }
    let file = File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let len = file.metadata().unwrap().len() as usize;
    if len == 0 {
        return &[];
    }
    // SAFETY: PROT_READ (1) + MAP_PRIVATE (2) of a file nobody writes during the run;
    // the mapping lives until the process exits, so the slice is valid for 'static.
    unsafe {
        let ptr = mmap(std::ptr::null_mut(), len, 1, 2, file.as_raw_fd(), 0);
        assert!(ptr as isize != -1, "mmap {path} failed");
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Byte -> lowercased token byte, or 0 for a separator (spec 2.2).
const LOWER: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut b = 0;
    while b < 256 {
        t[b] = match b as u8 {
            c @ (b'a'..=b'z' | b'0'..=b'9') => c,
            c @ b'A'..=b'Z' => c + 32,
            _ => 0,
        };
        b += 1;
    }
    t
};

fn hash_step(h: u64, c: u8) -> u64 {
    (h ^ c as u64).wrapping_mul(0x100000001b3)
}

/// Open-addressing set of the query terms; a lookup returns the term's id.
struct Terms {
    slots: Vec<u32>, // term id + 1, 0 = empty
    words: Vec<&'static [u8]>,
    shift: u32,
}

impl Terms {
    fn new(capacity: usize) -> Self {
        let bits = (capacity * 2).next_power_of_two().trailing_zeros().max(4);
        Terms { slots: vec![0; 1 << bits], words: Vec::new(), shift: 64 - bits }
    }

    fn slot(&self, hash: u64, word: &[u8]) -> usize {
        let mask = self.slots.len() - 1;
        let mut i = (hash.wrapping_mul(0x9E3779B97F4A7C15) >> self.shift) as usize;
        while self.slots[i] != 0 && self.words[self.slots[i] as usize - 1] != word {
            i = (i + 1) & mask;
        }
        i
    }

    fn get(&self, hash: u64, word: &[u8]) -> Option<u32> {
        self.slots[self.slot(hash, word)].checked_sub(1)
    }

    fn insert(&mut self, word: &'static [u8]) -> u32 {
        if self.words.len() * 2 >= self.slots.len() {
            *self = self.resized(self.slots.len());
        }
        let i = self.slot(word.iter().fold(0, |h, &c| hash_step(h, c)), word);
        if self.slots[i] == 0 {
            self.words.push(word);
            self.slots[i] = self.words.len() as u32;
        }
        self.slots[i] - 1
    }

    /// The same ids in a table with room for `capacity` terms at under half load.
    fn resized(&self, capacity: usize) -> Terms {
        let mut t = Terms::new(capacity);
        self.words.iter().for_each(|w| _ = t.insert(w));
        t
    }
}

/// The adjacent term pairs of every phrase. A pair is indexed like a term, with id
/// `first_id + n`, so a phrase's candidates are the documents holding all its pairs.
struct Pairs {
    slots: Vec<(u64, u32)>, // (key, id); key u64::MAX = empty
    role: Vec<u8>,          // per term: 1 = first of some pair, 2 = second of some pair
    first_id: u32,
    count: u32,
}

impl Pairs {
    fn new(queries: &[Query], first_id: u32) -> Pairs {
        let phrases = queries.iter().filter_map(|q| if let Query::Phrase(ts) = q { Some(ts) } else { None });
        let keys: Vec<u64> = phrases.flat_map(|ts| ts.windows(2).map(|w| (w[0] as u64) << 32 | w[1] as u64)).collect();
        let size = (keys.len() * 2).next_power_of_two().max(16);
        let mut p = Pairs { slots: vec![(u64::MAX, 0); size], role: vec![0; first_id as usize], first_id, count: 0 };
        for key in keys {
            p.role[(key >> 32) as usize] |= 1;
            p.role[key as u32 as usize] |= 2;
            let i = p.slot(key);
            if p.slots[i].0 == u64::MAX {
                p.slots[i] = (key, first_id + p.count);
                p.count += 1;
            }
        }
        p
    }

    fn slot(&self, key: u64) -> usize {
        let mask = self.slots.len() - 1;
        let mut i = (key.wrapping_mul(0x9E3779B97F4A7C15) >> 40) as usize & mask;
        while self.slots[i].0 != u64::MAX && self.slots[i].0 != key {
            i = (i + 1) & mask;
        }
        i
    }

    fn get(&self, a: u32, b: u32) -> Option<u32> {
        if self.role[a as usize] & 1 == 0 || self.role[b as usize] & 2 == 0 {
            return None;
        }
        let (key, id) = self.slots[self.slot((a as u64) << 32 | b as u64)];
        (key != u64::MAX).then_some(id)
    }
}

enum Query {
    Bool(Vec<(u8, u32)>), // (operator, term); the first operator is b'O' (OR into empty)
    Phrase(Vec<u32>),
}

fn parse_queries(text: &'static [u8], terms: &mut Terms) -> Vec<Query> {
    let lines = text.strip_suffix(b"\n").unwrap_or(text).split(|&b| b == b'\n');
    lines
        .map(|line| {
            if let Some(rest) = line.strip_prefix(b"BOOL ") {
                let mut words = rest.split(|&b| b == b' ');
                let mut ops = vec![(b'O', terms.insert(words.next().unwrap()))];
                while let (Some(op), Some(t)) = (words.next(), words.next()) {
                    ops.push((op[0], terms.insert(t)));
                }
                Query::Bool(ops)
            } else {
                let rest = line.strip_prefix(b"PHRASE \"").and_then(|r| r.strip_suffix(b"\""));
                let rest = rest.unwrap_or_else(|| panic!("bad query {:?}", String::from_utf8_lossy(line)));
                let phrase: Vec<u32> = rest.split(|&b| b == b' ').map(|t| terms.insert(t)).collect();
                assert!((2..=8).contains(&phrase.len()), "a phrase has 2 to 8 terms");
                Query::Phrase(phrase)
            }
        })
        .collect()
}
/// The documents of one chunk of whole lines and each query term's documents among them:
/// a sorted id list while the term is rare, a bitset once it is in more than 1/32 of the
/// chunk (the bitset is then the smaller of the two), and each document's text range.
/// Inside a chunk a document is its offset from `first_doc`, which fits in 16 bits.
struct Chunk {
    first_doc: u32,
    postings: Vec<Vec<u16>>, // empty once the term has a bitset
    bitset_of: Vec<u32>,     // index into `bitsets`, or NONE while the term is rare
    bitsets: Vec<Vec<u64>>,
    ranges: Vec<(u32, u32)>,
}

const NONE: u32 = u32::MAX;

impl Chunk {
    /// Records that document `local` contains term (or pair) `t`.
    fn add(&mut self, t: u32, local: u16, n_docs: usize) {
        let (list, b) = (&mut self.postings[t as usize], self.bitset_of[t as usize]);
        if b != NONE {
            set(&mut self.bitsets[b as usize], local, true);
        } else if list.last() != Some(&local) {
            list.push(local);
            if list.len() * 32 > n_docs {
                let mut bits = vec![0u64; n_docs.div_ceil(64)];
                std::mem::take(list).into_iter().for_each(|d| set(&mut bits, d, true));
                self.bitset_of[t as usize] = self.bitsets.len() as u32;
                self.bitsets.push(bits);
            }
        }
    }

    /// The term's bitset, or an empty slice while it is rare.
    fn bits(&self, t: u32) -> &[u64] {
        self.bitsets.get(self.bitset_of[t as usize] as usize).map_or(&[], |b| b)
    }

}

fn has(bits: &[u64], doc: u16) -> bool {
    bits[doc as usize / 64] >> (doc % 64) & 1 == 1
}

fn set(bits: &mut [u64], doc: u16, on: bool) {
    let w = &mut bits[doc as usize / 64];
    *w = *w & !(1 << (doc % 64)) | (on as u64) << (doc % 64);
}

fn ids(bits: &[u64]) -> impl Iterator<Item = u16> + '_ {
    bits.iter().enumerate().flat_map(|(w, &word)| {
        std::iter::successors(Some(word), |&x| Some(x & x.wrapping_sub(1)))
            .take_while(|&x| x != 0)
            .map(move |x| (w * 64) as u16 + x.trailing_zeros() as u16)
    })
}

fn index_chunk(docs: &[u8], start: usize, end: usize, terms: &Terms, pairs: &Pairs) -> Chunk {
    let n_docs = docs[start..end].iter().filter(|&&b| b == b'\n').count();
    assert!(n_docs <= 1 << 16, "a chunk holds at most 65,536 documents");
    let id_end = start + docs[start..end].iter().position(|&b| b == b'\t').unwrap_or(0);
    let first_doc: u32 = std::str::from_utf8(&docs[start..id_end]).unwrap().parse().unwrap_or(0);
    let mut c = Chunk {
        first_doc,
        postings: vec![Vec::new(); (pairs.first_id + pairs.count) as usize],
        bitset_of: vec![NONE; (pairs.first_id + pairs.count) as usize],
        bitsets: Vec::new(),
        ranges: Vec::with_capacity(n_docs),
    };
    let mut i = start;
    while i < end {
        let tab = i + docs[i..].iter().position(|&b| b == b'\t').unwrap();
        let doc: u32 = std::str::from_utf8(&docs[i..tab]).unwrap().parse().unwrap();
        assert_eq!(doc, first_doc + c.ranges.len() as u32, "ids must be dense");
        let line_end = tab + docs[tab..].iter().position(|&b| b == b'\n').unwrap();
        let local = c.ranges.len() as u16;
        c.ranges.push((tab as u32 + 1, line_end as u32));
        let (text, mut j, mut prev) = (&docs[tab + 1..line_end], 0, NONE);
        while j < text.len() {
            if LOWER[text[j] as usize] == 0 {
                j += 1;
                continue;
            }
            let (mut h, mut buf, mut len) = (0u64, [0u8; 32], 0);
            while j < text.len() && LOWER[text[j] as usize] != 0 {
                let ch = LOWER[text[j] as usize];
                if len < 32 {
                    buf[len] = ch;
                }
                h = hash_step(h, ch);
                len += 1;
                j += 1;
            }
            // A token that is not a query term breaks adjacency for the pairs.
            let Some(t) = (len <= 32).then(|| terms.get(h, &buf[..len])).flatten() else {
                prev = NONE;
                continue;
            };
            c.add(t, local, n_docs);
            if let Some(p) = (prev != NONE).then(|| pairs.get(prev, t)).flatten() {
                c.add(p, local, n_docs);
            }
            prev = t;
        }
        i = line_end + 1;
    }
    c
}

fn intersect<T: Ord + Copy>(a: &[T], b: &[T]) -> Vec<T> {
    let (small, big) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if small.len() * 32 < big.len() {
        return small.iter().copied().filter(|x| big.binary_search(x).is_ok()).collect();
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < small.len() && j < big.len() {
        match small[i].cmp(&big[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(small[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

fn union<T: Ord + Copy>(a: &[T], b: &[T]) -> Vec<T> {
    let (mut i, mut j, mut out) = (0, 0, Vec::with_capacity(a.len() + b.len()));
    while i < a.len() && j < b.len() {
        let (x, y) = (a[i], b[j]);
        out.push(x.min(y));
        i += (x <= y) as usize;
        j += (y <= x) as usize;
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

fn difference<T: Ord + Copy>(a: &[T], b: &[T]) -> Vec<T> {
    if a.len() * 32 < b.len() {
        return a.iter().copied().filter(|x| b.binary_search(x).is_err()).collect();
    }
    let mut j = 0;
    a.iter()
        .copied()
        .filter(|&x| {
            while j < b.len() && b[j] < x {
                j += 1;
            }
            j == b.len() || b[j] != x
        })
        .collect()
}

/// True when the document's tokens contain `phrase` (at most 8 terms) at consecutive
/// positions. Streams the tokens through a ring of the last 8 and stops at the first match.
fn contains_phrase(text: &[u8], phrase: &[&[u8]]) -> bool {
    let (k, mut ring, mut n) = (phrase.len(), [&[][..]; 8], 0);
    for token in text.split(|&b| LOWER[b as usize] == 0).filter(|t| !t.is_empty()) {
        ring[n % 8] = token;
        n += 1;
        if n >= k && (0..k).rev().all(|i| ring[(n - k + i) % 8].eq_ignore_ascii_case(phrase[i])) {
            return true;
        }
    }
    false
}

/// One chunk's answer to one query: its matches are a contiguous id range of the total.
struct Partial {
    count: u32,
    sum: u64,
    first: [u32; 10], // the min(count, 10) smallest ids
}

impl Partial {
    fn new(first_doc: u32, ids: impl Iterator<Item = u16>) -> Partial {
        let mut p = Partial { count: 0, sum: 0, first: [0; 10] };
        for d in ids.map(|d| first_doc + d as u32) {
            if p.count < 10 {
                p.first[p.count as usize] = d;
            }
            p.count += 1;
            p.sum += d as u64;
        }
        p
    }
}

/// A running BOOL result: a sorted id list, or a bitset once a frequent term joins by OR.
enum Set {
    List(Vec<u16>),
    Bits(Vec<u64>),
}

/// Applies `ops` left to right. Frequent terms use their bitset, so `the AND x` tests
/// x's few ids against bits instead of walking a list of most of the chunk.
fn run_bool(ops: &[(u8, u32)], c: &Chunk) -> Set {
    let zip = |r: &mut [u64], p: &[u64], f: fn(u64, u64) -> u64| r.iter_mut().zip(p).for_each(|(a, &b)| *a = f(*a, b));
    ops.iter().fold(Set::List(Vec::new()), |r, &(op, t)| {
        let (list, bits) = (&c.postings[t as usize], c.bits(t));
        match (r, op, bits.is_empty()) {
            (Set::List(r), b'A', false) => Set::List(r.into_iter().filter(|&d| has(bits, d)).collect()),
            (Set::List(r), b'N', false) => Set::List(r.into_iter().filter(|&d| !has(bits, d)).collect()),
            (Set::List(r), b'A', true) => Set::List(intersect(&r, list)),
            (Set::List(r), b'N', true) => Set::List(difference(&r, list)),
            (Set::List(r), _, true) => Set::List(union(&r, list)),
            (Set::List(r), _, false) => {
                let mut b = bits.to_vec();
                r.into_iter().for_each(|d| set(&mut b, d, true));
                Set::Bits(b)
            }
            (Set::Bits(b), b'A', true) => Set::List(list.iter().copied().filter(|&d| has(&b, d)).collect()),
            (Set::Bits(mut b), op, false) => {
                let f: fn(u64, u64) -> u64 = match op {
                    b'A' => |a, b| a & b,
                    b'N' => |a, b| a & !b,
                    _ => |a, b| a | b,
                };
                zip(&mut b, bits, f);
                Set::Bits(b)
            }
            (Set::Bits(mut b), op, true) => {
                list.iter().for_each(|&d| set(&mut b, d, op != b'N'));
                Set::Bits(b)
            }
        }
    })
}

/// Candidates are the documents holding every adjacent pair of the phrase: the shortest
/// id list, intersected with the other lists and filtered by the bitset pairs. A two-term
/// phrase is exactly its pair; longer ones re-check each candidate's text for the sequence.
fn run_phrase(ts: &[u32], c: &Chunk, pairs: &Pairs, words: &[&[u8]], docs: &[u8]) -> Vec<u16> {
    let ps: Vec<u32> = ts.windows(2).map(|w| pairs.get(w[0], w[1]).unwrap()).collect();
    let mut lists: Vec<&Vec<u16>> =
        ps.iter().filter(|&&p| c.bits(p).is_empty()).map(|&p| &c.postings[p as usize]).collect();
    lists.sort_by_key(|p| p.len());
    let mut candidates = match lists.first() {
        Some(p) => lists[1..].iter().fold(p.to_vec(), |r, p| intersect(&r, p)),
        None => ids(c.bits(ps[0])).collect(),
    };
    candidates.retain(|&d| ps.iter().all(|&p| c.bits(p).is_empty() || has(c.bits(p), d)));
    if ts.len() == 2 {
        return candidates;
    }
    let phrase: Vec<&[u8]> = ts.iter().map(|&t| words[t as usize]).collect();
    candidates.retain(|&d| {
        let (start, end) = c.ranges[d as usize];
        contains_phrase(&docs[start as usize..end as usize], &phrase)
    });
    candidates
}

fn answer(q: &Query, chunk: &Chunk, pairs: &Pairs, words: &[&[u8]], docs: &[u8]) -> Partial {
    match q {
        Query::Bool(ops) => match run_bool(ops, chunk) {
            Set::List(list) => Partial::new(chunk.first_doc, list.into_iter()),
            Set::Bits(bits) => Partial::new(chunk.first_doc, ids(&bits)),
        },
        Query::Phrase(ts) => Partial::new(chunk.first_doc, run_phrase(ts, chunk, pairs, words, docs).into_iter()),
    }
}

fn main() {
    let docs = map("/data/docs.txt");
    let query_text = map("/data/queries.txt");
    let mut terms = Terms::new(1 << 12);
    let queries = parse_queries(query_text, &mut terms);
    let pairs = Pairs::new(&queries, terms.words.len() as u32);
    let threads = std::thread::available_parallelism().map_or(12, |n| n.get());

    // Chunk boundaries at line starts, so every document belongs to exactly one chunk.
    // At most 32 MB a chunk keeps it under 65,536 documents (the corpus averages ~1 KB).
    let n_chunks = threads.max(docs.len() >> 25);
    let mut cuts = vec![0];
    for k in 1..n_chunks {
        let at = (docs.len() * k / n_chunks).max(*cuts.last().unwrap());
        let next = docs[at..].iter().position(|&b| b == b'\n').map_or(docs.len(), |p| at + p + 1);
        cuts.push(next);
    }
    cuts.push(docs.len());

    // Each thread indexes one chunk. Chunks hold consecutive id ranges, so a query's answer
    // is the chunks' answers in chunk order and no posting list is ever merged.
    let chunks: Vec<Chunk> = std::thread::scope(|s| {
        let handles: Vec<_> =
            cuts.windows(2).map(|w| s.spawn(|| index_chunk(docs, w[0], w[1], &terms, &pairs))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Workers take blocks of queries from a shared counter and answer each over every chunk.
    const BLOCK: usize = 256;
    let next = AtomicUsize::new(0);
    let mut blocks: Vec<(usize, String)> = std::thread::scope(|s| {
        let work = || {
            let mut done = Vec::new();
            loop {
                let b = next.fetch_add(1, Ordering::Relaxed);
                if b * BLOCK >= queries.len() {
                    return done;
                }
                let mut out = String::new();
                for q in &queries[b * BLOCK..queries.len().min((b + 1) * BLOCK)] {
                    let parts: Vec<Partial> = chunks.iter().map(|c| answer(q, c, &pairs, &terms.words, docs)).collect();
                    let (count, sum) = parts.iter().fold((0, 0), |(n, s), p| (n + p.count, s + p.sum));
                    write!(out, "{count} {sum}").unwrap();
                    let first = parts.iter().flat_map(|p| &p.first[..p.count.min(10) as usize]);
                    first.take(10).for_each(|d| write!(out, " {d}").unwrap());
                    out.push('\n');
                }
                done.push((b, out));
            }
        };
        let handles: Vec<_> = (0..threads).map(|_| s.spawn(work)).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    blocks.sort_unstable_by_key(|b| b.0);
    let out: String = blocks.into_iter().map(|b| b.1).collect();
    File::create("/out/results.txt").and_then(|mut f| f.write_all(out.as_bytes())).expect("write /out/results.txt");
    // Skip freeing ~1M posting lists one by one; the process is done.
    std::process::exit(0);
}
