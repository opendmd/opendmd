//! The playing file's read-ahead: a ring of [`BLOCK`]-byte blocks of the
//! file, at block-aligned offsets, filled ahead of the player's position by
//! another task and served to the player's reads from memory. A card that
//! turns slow for a while (a read ten times its usual time, or a read
//! queued behind a write's busy time) then delays the filling task, not
//! the frame.
//!
//! This is the ring's bookkeeping alone (which block to fetch next, where
//! a fetched block goes, what a read can be served); the firmware's
//! `readahead` keeps one behind a lock, fills it on CPU0 and serves the
//! display core's raw reads from it.

/// One block: gifslim reads its file through a 2 KiB buffer, so the
/// player's reads are mostly one block each, at block-aligned offsets.
pub const BLOCK: usize = 2048;

/// `N` blocks of the file.
pub struct Ring<const N: usize> {
    /// Each block's file offset (a multiple of [`BLOCK`]) and its length
    /// (less than a block only at the end of the file); `None` when free.
    held: [Option<(u64, u32)>; N],
    /// `N` blocks, block `k` at `k * BLOCK` (on the heap, never the stack).
    data: Box<[u8]>,
}

impl<const N: usize> Default for Ring<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Ring<N> {
    pub fn new() -> Self {
        Self::try_new().expect("read-ahead ring")
    }

    /// `None` when the heap has not `N` blocks (the firmware's way).
    pub fn try_new() -> Option<Self> {
        let mut v = Vec::new();
        v.try_reserve_exact(N * BLOCK).ok()?;
        v.resize(N * BLOCK, 0u8);
        Some(Ring {
            held: [None; N],
            data: v.into_boxed_slice(),
        })
    }

    fn block(&self, k: usize) -> &[u8] {
        &self.data[k * BLOCK..(k + 1) * BLOCK]
    }

    /// Forget every block (another file, or this one written).
    pub fn clear(&mut self) {
        self.held = [None; N];
    }

    fn find(&self, off: u64) -> Option<usize> {
        self.held.iter().position(|h| h.is_some_and(|(o, _)| o == off))
    }

    /// Copy what the ring holds from `pos` on into `buf`, across blocks
    /// while they follow each other: the bytes served, 0 when the ring has
    /// none at `pos`.
    pub fn serve(&self, pos: u64, buf: &mut [u8]) -> usize {
        let mut done = 0usize;
        while done < buf.len() {
            let at = pos + done as u64;
            let off = at - at % BLOCK as u64;
            let Some(k) = self.find(off) else { break };
            let (_, len) = self.held[k].unwrap();
            let within = (at - off) as usize;
            if within >= len as usize {
                break;
            }
            let n = (len as usize - within).min(buf.len() - done);
            buf[done..done + n].copy_from_slice(&self.block(k)[within..within + n]);
            done += n;
        }
        done
    }

    /// Bytes held from `pos` on without a gap (what the player could read
    /// now without the card).
    pub fn ahead(&self, pos: u64) -> u64 {
        let mut at = pos;
        loop {
            let off = at - at % BLOCK as u64;
            let Some(k) = self.find(off) else { break };
            let (_, len) = self.held[k].unwrap();
            let end = off + len as u64;
            if end <= at {
                break;
            }
            at = end;
            if (len as usize) < BLOCK {
                break;
            }
        }
        at - pos
    }

    /// The next block to fetch for a player at `pos` in a file of `size`
    /// bytes: the first block from `pos`'s on, within the ring's reach (`N`
    /// blocks), that is not held and starts before the end. `None`: the
    /// ring is as full as it can be.
    pub fn want(&self, pos: u64, size: u64) -> Option<u64> {
        let first = pos - pos % BLOCK as u64;
        (0..N as u64)
            .map(|i| first + i * BLOCK as u64)
            .take_while(|&off| off < size)
            .find(|&off| self.find(off).is_none())
    }

    /// Keep `bytes` as the block at `off` (block-aligned; at most a block)
    /// for a player now at `pos`: in a free slot, or one whose block is
    /// behind `pos` or out of the ring's reach. False if every slot holds a
    /// block still wanted (nothing changes).
    pub fn put(&mut self, off: u64, bytes: &[u8], pos: u64) -> bool {
        debug_assert!(off.is_multiple_of(BLOCK as u64) && bytes.len() <= BLOCK);
        let first = pos - pos % BLOCK as u64;
        let reach = first + (N * BLOCK) as u64;
        let k = match self.find(off) {
            Some(k) => k,
            None => {
                let spare = self.held.iter().position(|h| match h {
                    None => true,
                    Some((o, _)) => *o < first || *o >= reach,
                });
                let Some(k) = spare else { return false };
                k
            }
        };
        self.data[k * BLOCK..k * BLOCK + bytes.len()].copy_from_slice(bytes);
        self.held[k] = Some((off, bytes.len() as u32));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i * 7 + i / 251) as u8).collect()
    }

    fn block(f: &[u8], off: u64) -> &[u8] {
        let o = off as usize;
        &f[o..(o + BLOCK).min(f.len())]
    }

    /// A filler keeping the ring full ahead of a reader, the reader served
    /// from it: every byte read is the file's, in any read size.
    #[test]
    fn serves_the_files_bytes_ahead_of_the_reader() {
        let f = file(BLOCK * 9 + 300);
        for step in [1usize, 4, 511, 512, 2048, 2049, 5000] {
            let mut r = Ring::<4>::new();
            let mut pos = 0u64;
            let mut got = Vec::new();
            while (pos as usize) < f.len() {
                while let Some(off) = r.want(pos, f.len() as u64) {
                    assert!(r.put(off, block(&f, off), pos));
                }
                let mut buf = vec![0u8; step];
                let n = r.serve(pos, &mut buf);
                assert!(n > 0, "step {step}: nothing at {pos}");
                got.extend_from_slice(&buf[..n]);
                pos += n as u64;
            }
            assert_eq!(got, f, "step {step}");
        }
    }

    #[test]
    fn want_stops_at_the_reach_and_the_end() {
        let mut r = Ring::<4>::new();
        let size = (BLOCK * 3 + 10) as u64;
        let f = file(size as usize);
        let mut fetched = Vec::new();
        while let Some(off) = r.want(0, size) {
            fetched.push(off);
            r.put(off, block(&f, off), 0);
        }
        assert_eq!(fetched, [0, 2048, 4096, 6144]);
        assert_eq!(r.ahead(0), size);
        assert_eq!(r.ahead(100), size - 100);
        // A bigger file: four blocks from the reader's block, no more.
        let mut r = Ring::<4>::new();
        let f = file(BLOCK * 20);
        while let Some(off) = r.want(5000, f.len() as u64) {
            r.put(off, block(&f, off), 5000);
        }
        assert_eq!(r.ahead(5000), (4096 + 4 * BLOCK - 5000) as u64);
        assert_eq!(r.want(5000, f.len() as u64), None);
    }

    #[test]
    fn put_reuses_blocks_behind_or_out_of_reach_never_wanted_ones() {
        let f = file(BLOCK * 20);
        let mut r = Ring::<4>::new();
        for off in [0u64, 2048, 4096, 6144] {
            assert!(r.put(off, block(&f, off), 0));
        }
        // All four still wanted at 0: refused.
        assert!(!r.put(8192, block(&f, 8192), 0));
        // The reader moved past the first block: it goes.
        assert!(r.put(8192, block(&f, 8192), 2048));
        let mut b = [0u8; 10];
        assert_eq!(r.serve(0, &mut b), 0);
        assert_eq!(r.serve(8192, &mut b), 10);
        assert_eq!(&b, &f[8192..8202]);
        // A seek back to 0 (the GIF loops): the blocks out of reach go.
        for off in [0u64, 2048, 4096, 6144] {
            assert!(r.put(off, block(&f, off), 0));
        }
        assert_eq!(r.ahead(0), (4 * BLOCK) as u64);
    }

    #[test]
    fn a_gap_ends_the_serve_and_clear_forgets() {
        let f = file(BLOCK * 6);
        let mut r = Ring::<4>::new();
        r.put(0, block(&f, 0), 0);
        r.put(4096, block(&f, 4096), 0);
        let mut b = vec![0u8; 3 * BLOCK];
        assert_eq!(r.serve(10, &mut b), BLOCK - 10);
        assert_eq!(r.ahead(10), (BLOCK - 10) as u64);
        assert_eq!(r.want(10, f.len() as u64), Some(2048));
        r.clear();
        assert_eq!(r.serve(10, &mut b), 0);
        assert_eq!(r.ahead(0), 0);
    }
}
