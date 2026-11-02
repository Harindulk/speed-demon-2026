//! Track D: exact point-to-point shortest paths on a 1.7M-node road graph.
//!
//! Parse the edge list in parallel into forward and reverse CSRs, then answer
//! queries on every core with a bidirectional Dijkstra.

use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

mod mmap {
    use std::fs::File;
    use std::os::fd::AsRawFd;

    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    }

    /// Maps a whole file read-only. The input is already in the page cache, so
    /// mapping it costs no memory of our own, unlike reading it into a Vec.
    pub fn map(path: &str) -> &'static [u8] {
        let file = File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let len = file.metadata().unwrap().len() as usize;
        if len == 0 {
            return &[];
        }
        const PROT_READ: i32 = 1;
        const MAP_PRIVATE: i32 = 2;
        // SAFETY: the file is read-only input that nothing modifies while we run;
        // the mapping lives until exit, so the 'static slice never dangles.
        unsafe {
            let p = mmap(std::ptr::null_mut(), len, PROT_READ, MAP_PRIVATE, file.as_raw_fd(), 0);
            assert!(p as isize != -1, "mmap {path} failed");
            std::slice::from_raw_parts(p, len)
        }
    }
}

/// Reads one unsigned decimal at `*i` and skips the single separator after it.
#[inline]
fn number(b: &[u8], i: &mut usize) -> u64 {
    let mut x = 0u64;
    while b[*i] >= b'0' {
        x = x * 10 + (b[*i] - b'0') as u64;
        *i += 1;
    }
    *i += 1;
    x
}

/// Splits `b` into about `parts` pieces, each ending just after a newline.
fn line_chunks(b: &[u8], parts: usize) -> Vec<&[u8]> {
    let mut chunks = Vec::with_capacity(parts);
    let mut start = 0;
    for k in 1..=parts {
        let mut end = (b.len() * k / parts).max(start);
        while end < b.len() && b[end - 1] != b'\n' {
            end += 1;
        }
        chunks.push(&b[start..end]);
        start = end;
    }
    chunks
}

/// Adjacency in compressed sparse row form: the edges of u are
/// `edges[first[u]..first[u + 1]]`, each a (neighbour, weight) pair.
struct Csr {
    first: Vec<u32>,
    edges: Vec<(u32, u32)>,
}

impl Csr {
    /// Builds the out-edge CSR, or the in-edge CSR when `forward` is false.
    fn build(n: usize, parsed: &[Vec<(u32, u32, u32)>], forward: bool) -> Csr {
        let ends = |&(u, v, w): &(u32, u32, u32)| if forward { (u, v, w) } else { (v, u, w) };
        let mut first = vec![0u32; n + 1];
        for e in parsed.iter().flatten() {
            first[ends(e).0 as usize + 1] += 1;
        }
        for k in 0..n {
            first[k + 1] += first[k];
        }
        let mut next = first.clone();
        let mut edges = vec![(0u32, 0u32); first[n] as usize];
        for e in parsed.iter().flatten() {
            let (a, b, w) = ends(e);
            let slot = &mut next[a as usize];
            edges[*slot as usize] = (b, w);
            *slot += 1;
        }
        Csr { first, edges }
    }

    /// The same graph with node u renamed to `label[u]`; `order` is the inverse
    /// of `label`. Rows are copied whole, in parallel blocks of new ids, so this
    /// streams instead of scattering.
    fn relabel(&self, order: &[u32], label: &[u32], threads: usize) -> Csr {
        let n = order.len();
        let mut first = Vec::with_capacity(n + 1);
        first.push(0u32);
        for &u in order {
            first.push(first[first.len() - 1] + self.neighbours(u).len() as u32);
        }
        let mut edges = vec![(0u32, 0u32); self.edges.len()];
        thread::scope(|s| {
            let mut rest = &mut edges[..];
            for k in 0..threads {
                let (lo, hi) = (n * k / threads, n * (k + 1) / threads);
                let (block, tail) = rest.split_at_mut((first[hi] - first[lo]) as usize);
                rest = tail;
                s.spawn(move || {
                    let mut slots = block.iter_mut();
                    for &u in &order[lo..hi] {
                        for (&(v, w), slot) in self.neighbours(u).iter().zip(&mut slots) {
                            *slot = (label[v as usize], w);
                        }
                    }
                });
            }
        });
        Csr { first, edges }
    }

    fn neighbours(&self, u: u32) -> &[(u32, u32)] {
        &self.edges[self.first[u as usize] as usize..self.first[u as usize + 1] as usize]
    }
}

/// Breadth-first visiting order, so that nodes close in the graph get close
/// ids. The input ids are a random shuffle, which makes every search step a
/// cache miss; after relabelling a search touches a few compact id ranges.
/// Each thread grows its own BFS region from its own root, claiming nodes
/// atomically, and the regions are laid out one after another.
/// Returns (order, label): `order[k]` is the k-th node laid out, `label` its inverse.
fn bfs_labels(g: &Csr, threads: usize) -> (Vec<u32>, Vec<u32>) {
    let n = g.first.len() - 1;
    let claimed: Vec<AtomicBool> = (0..n).map(|_| AtomicBool::new(false)).collect();
    // The plain load skips the costlier swap for the many already-claimed nodes.
    let claim = |u: u32| {
        let c = &claimed[u as usize];
        !c.load(Ordering::Relaxed) && !c.swap(true, Ordering::Relaxed)
    };
    let regions: Vec<Vec<u32>> = thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|k| {
                let claim = &claim;
                s.spawn(move || {
                    let root = (n * k / threads) as u32;
                    let mut queue = Vec::new();
                    if claim(root) {
                        queue.push(root);
                    }
                    let mut head = 0;
                    while head < queue.len() {
                        for &(v, _) in g.neighbours(queue[head]) {
                            if claim(v) {
                                queue.push(v);
                            }
                        }
                        head += 1;
                    }
                    queue
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut order = regions.concat();
    // Nodes no region reached (islands, nodes with no in-edges) go last.
    order.extend((0..n as u32).filter(|&u| claim(u)));
    let mut label = vec![0u32; n];
    for (k, &u) in order.iter().enumerate() {
        label[u as usize] = k as u32;
    }
    (order, label)
}

/// Parses graph.txt in parallel and returns its forward and reverse CSR,
/// relabelled by `bfs_labels`, plus the label of every input node id.
fn read_graph(b: &[u8], threads: usize) -> (Csr, Csr, Vec<u32>) {
    let mut i = 0;
    let n = number(b, &mut i) as usize;
    let m = number(b, &mut i) as usize;
    let parsed: Vec<Vec<(u32, u32, u32)>> = thread::scope(|s| {
        let handles: Vec<_> = line_chunks(&b[i..], threads)
            .into_iter()
            .map(|chunk| {
                s.spawn(move || {
                    let mut out = Vec::with_capacity(chunk.len() / 16);
                    let mut j = 0;
                    while j < chunk.len() {
                        let u = number(chunk, &mut j) as u32;
                        let v = number(chunk, &mut j) as u32;
                        let w = number(chunk, &mut j) as u32;
                        out.push((u, v, w));
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(parsed.iter().map(Vec::len).sum::<usize>(), m, "edge count");
    // The BFS needs only the forward graph, so it overlaps the reverse build.
    let (fwd, rev, (order, label)) = thread::scope(|s| {
        let reverse = s.spawn(|| Csr::build(n, &parsed, false));
        let fwd = Csr::build(n, &parsed, true);
        let labels = bfs_labels(&fwd, threads);
        (fwd, reverse.join().unwrap(), labels)
    });
    drop(parsed);
    let fwd = fwd.relabel(&order, &label, threads);
    let rev = rev.relabel(&order, &label, threads);
    (fwd, rev, label)
}

/// Tentative distances of one search, as an open-addressing hash map sized to
/// the search rather than to the graph: a dense u64 array per side per thread
/// would cost 12 x 2 x 14 MB on the box, while a search touches ~10^4 nodes.
struct DistMap {
    keys: Vec<u32>, // EMPTY or a node id
    vals: Vec<u64>,
    used: Vec<u32>, // occupied slots, so clearing costs only what was touched
}

const EMPTY: u32 = u32::MAX; // never a node id: n < 2^32 - 1

impl DistMap {
    /// `slots` must be a power of two.
    fn new(slots: usize) -> Self {
        DistMap { keys: vec![EMPTY; slots], vals: vec![0; slots], used: Vec::new() }
    }

    /// The slot holding u, or the empty slot where u would go (linear probing).
    /// A multiplicative hash, because the low id bits alone send the runs of
    /// nearby BFS ids from different rings into the same slots.
    fn find(&self, u: u32) -> usize {
        let mask = self.keys.len() - 1;
        let mut i = (u.wrapping_mul(0x9E37_79B9) >> 8) as usize & mask;
        while self.keys[i] != u && self.keys[i] != EMPTY {
            i = (i + 1) & mask;
        }
        i
    }

    /// Distance of u, or u64::MAX if u has not been reached.
    fn get(&self, u: u32) -> u64 {
        let i = self.find(u);
        if self.keys[i] == u { self.vals[i] } else { u64::MAX }
    }

    /// Mutable distance of u, inserted as u64::MAX if absent.
    fn entry(&mut self, u: u32) -> &mut u64 {
        if self.used.len() * 2 >= self.keys.len() {
            let old = std::mem::replace(self, DistMap::new(self.keys.len() * 2));
            for &i in &old.used {
                *self.entry(old.keys[i as usize]) = old.vals[i as usize];
            }
        }
        let i = self.find(u);
        if self.keys[i] == EMPTY {
            self.keys[i] = u;
            self.vals[i] = u64::MAX;
            self.used.push(i as u32);
        }
        &mut self.vals[i]
    }

    fn clear(&mut self) {
        for &i in &self.used {
            self.keys[i as usize] = EMPTY;
        }
        self.used.clear();
    }
}

/// Monotone priority queue for Dijkstra (a radix heap): keys pushed are never
/// below the last key popped, so an entry only needs to sit in the bucket
/// named by the highest bit where it differs from that key. Each entry moves
/// down at most 64 times, and pops scan no comparisons in a binary tree.
struct RadixHeap {
    last: u64,
    buckets: Vec<Vec<(u64, u32)>>, // bucket 0 holds keys equal to `last`
    len: usize,
}

impl RadixHeap {
    fn new() -> Self {
        RadixHeap { last: 0, buckets: vec![Vec::new(); 65], len: 0 }
    }

    fn bucket(&self, key: u64) -> usize {
        64 - (key ^ self.last).leading_zeros() as usize
    }

    fn push(&mut self, key: u64, node: u32) {
        let b = self.bucket(key);
        self.buckets[b].push((key, node));
        self.len += 1;
    }

    /// Makes bucket 0 non-empty if the heap is: moves `last` up to the
    /// smallest key of the first non-empty bucket and spreads that bucket out.
    fn refill(&mut self) {
        if !self.buckets[0].is_empty() {
            return;
        }
        let Some(i) = self.buckets.iter().position(|b| !b.is_empty()) else { return };
        let moving = std::mem::take(&mut self.buckets[i]);
        self.last = moving.iter().map(|&(k, _)| k).min().unwrap();
        for &(k, v) in &moving {
            let b = self.bucket(k);
            self.buckets[b].push((k, v));
        }
        self.buckets[i] = moving; // keep its allocation
        self.buckets[i].clear();
    }

    fn peek(&mut self) -> Option<u64> {
        self.refill();
        self.buckets[0].last().map(|&(k, _)| k)
    }

    /// Pops a minimum entry; the heap must not be empty.
    fn pop(&mut self) -> (u64, u32) {
        self.refill();
        self.len -= 1;
        self.buckets[0].pop().unwrap()
    }

    fn clear(&mut self) {
        self.buckets.iter_mut().for_each(Vec::clear);
        self.last = 0;
        self.len = 0;
    }
}

/// One direction of a bidirectional search; state is reused across queries.
struct Side {
    dist: DistMap,
    heap: RadixHeap,
}

impl Side {
    fn new() -> Self {
        Side { dist: DistMap::new(1 << 16), heap: RadixHeap::new() }
    }

    fn reset(&mut self, start: u32) {
        self.dist.clear();
        self.heap.clear();
        *self.dist.entry(start) = 0;
        self.heap.push(0, start);
    }

    fn top(&mut self) -> Option<u64> {
        self.heap.peek()
    }

    /// Settles one node and relaxes its edges; `best` becomes the shortest
    /// s-t path seen through a node both sides have labelled.
    fn step(&mut self, g: &Csr, other: &Side, best: &mut u64) {
        let (d, u) = self.heap.pop();
        if d > self.dist.get(u) {
            return; // stale entry
        }
        for &(v, w) in g.neighbours(u) {
            let nd = d + w as u64;
            let known = self.dist.entry(v);
            if nd >= *known {
                continue;
            }
            *known = nd;
            // Checking meetings only on improvement is enough: whichever side
            // labelled v last compared against the other side's label then.
            let far = other.dist.get(v);
            if far != u64::MAX {
                *best = (*best).min(nd + far);
            }
            if nd < *best {
                self.heap.push(nd, v); // a node at >= best cannot help
            }
        }
    }
}

/// Bidirectional Dijkstra: grows a search from s over the graph and from t
/// over the reversed graph, always expanding the side with the smaller frontier.
/// Balancing by frontier size, not distance, lets a dead-end side (an island
/// whose only exits weigh ~2^32) run dry in a few steps instead of making the
/// other side settle the whole graph first.
fn shortest_path(fwd: &Csr, rev: &Csr, f: &mut Side, b: &mut Side, s: u32, t: u32) -> Option<u64> {
    if s == t {
        return Some(0);
    }
    f.reset(s);
    b.reset(t);
    let mut best = u64::MAX;
    // Once one side runs dry every path has been seen: the edge into its last
    // node was relaxed against the other side's start, whose label is 0.
    while let (Some(df), Some(db)) = (f.top(), b.top()) {
        if df + db >= best {
            break;
        }
        if f.heap.len <= b.heap.len {
            f.step(fwd, b, &mut best);
        } else {
            b.step(rev, f, &mut best);
        }
    }
    (best != u64::MAX).then_some(best)
}

fn main() {
    let threads = thread::available_parallelism().map_or(1, |p| p.get());
    let (fwd, rev, label) = read_graph(mmap::map("/data/graph.txt"), threads);

    let q = mmap::map("/data/queries.txt");
    let mut queries = Vec::new();
    let mut i = 0;
    while i < q.len() {
        let (s, t) = (number(q, &mut i), number(q, &mut i));
        queries.push((label[s as usize], label[t as usize]));
    }

    let next = AtomicUsize::new(0);
    let mut answers: Vec<Option<u64>> = vec![None; queries.len()];
    let results: Vec<Vec<(usize, Option<u64>)>> = thread::scope(|s| {
        let handles: Vec<_> = (0..threads.min(queries.len()).max(1))
            .map(|_| {
                s.spawn(|| {
                    let (mut f, mut b) = (Side::new(), Side::new());
                    let mut done = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(src, dst)) = queries.get(k) else { break };
                        done.push((k, shortest_path(&fwd, &rev, &mut f, &mut b, src, dst)));
                    }
                    done
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (k, d) in results.into_iter().flatten() {
        answers[k] = d;
    }

    let mut out = String::with_capacity(answers.len() * 8);
    for d in answers {
        match d {
            Some(d) => out.push_str(&d.to_string()),
            None => out.push_str("-1"),
        }
        out.push('\n');
    }
    File::create("/out/distances.txt").and_then(|mut f| f.write_all(out.as_bytes())).unwrap();
}
