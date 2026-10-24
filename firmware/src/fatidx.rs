//! **Spike** (`--features fat-index`, docs/research/fat-index.md): a side
//! index over the card's existing FAT layout, so an open skips the linear
//! directory scan.
//!
//! For each GIF under the indexed directories the index holds the position
//! of its short (8.3) directory entry: the sector (relative to the FAT data
//! region) and the slot (0-15) in it, keyed by an FNV-1a hash of the
//! card path. An open reads that one sector, checks that the entry is live
//! and that its long-name entries spell exactly the expected name (or its
//! 8.3 name does), and fills a FatFs `FIL` from the entry the way `f_open`
//! does — read-only. Anything that does not match falls back to the normal
//! path open (a stale index costs one sector read, never a wrong file).
//!
//! No FatFs source is patched: everything used is public in `ff.h` (the
//! `FATFS`/`FIL` structs, `ff_mutex_take/give`, `ff_disk_read`, `f_read`,
//! `f_lseek`, `f_close`, `f_opendir`). The sector is read under the volume
//! mutex, from `fs->win` when FatFs has it there (possibly dirty), else from
//! the card. With `FF_FS_LOCK = 0` (sdkconfig) FatFs keeps no open-file
//! table, so a `FIL` it did not open is indistinguishable from one it did.
//!
//! The `FIL` is our own heap allocation (`sizeof(FIL)` = 4,120 B with
//! `FF_MAX_SS` 4096), not one of the VFS's preallocated slots, so the
//! player and server read it through [`IdxFile`], not `std::fs::File`.
//!
//! On-card format, `/fatidx.bin`, little-endian: a 32-byte header (`FIDX`,
//! version 1, count, the volume's `n_fatent`, `database`, `csize`), a
//! 257-entry `u32` fanout by the hash's top byte, then `count` 8-byte
//! records `{hash: u32, loc: u32}` grouped by that byte, `loc =
//! rel_sector << 4 | slot`.

use esp_idf_sys as sys;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

use crate::config;

pub const INDEX: &str = "/sdcard/fatidx.bin";
const TMP: &str = "/sdcard/fatidx.tmp";
const NEW: &str = "/sdcard/fatidx.new";
const MAGIC: &[u8; 4] = b"FIDX";
const HEADER: u64 = 32;
const FANOUT: usize = 257;
const RECORDS_AT: u64 = HEADER + (FANOUT as u64) * 4;
/// Records gathered per pass when grouping by bucket (8 B each).
const PASS_RECORDS: usize = 1024;
const SS: usize = 512;
/// Sectors read per batch while walking a directory.
const BATCH: usize = 8;

pub static PLAYER: AtomicBool = AtomicBool::new(false);
pub static SERVER: AtomicBool = AtomicBool::new(false);

static HITS: AtomicU32 = AtomicU32::new(0);
static ABSENT: AtomicU32 = AtomicU32::new(0);
static STALE: AtomicU32 = AtomicU32::new(0);
static XCLUST: AtomicU32 = AtomicU32::new(0);
static OTHER: AtomicU32 = AtomicU32::new(0);
static HIT_US: AtomicU32 = AtomicU32::new(0);
static HIT_MAX_US: AtomicU32 = AtomicU32::new(0);

struct Loaded {
    file: File,
    fanout: Box<[u32; FANOUT]>,
}

static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);
static FS: AtomicPtr<sys::FATFS> = AtomicPtr::new(std::ptr::null_mut());
static FS_ID: AtomicU32 = AtomicU32::new(0);

fn fnv(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h
}

fn fr_err(r: sys::FRESULT) -> io::Error {
    let code = match r {
        sys::FRESULT_FR_DISK_ERR | sys::FRESULT_FR_INT_ERR => sys::EIO,
        sys::FRESULT_FR_TIMEOUT => sys::ETIMEDOUT,
        sys::FRESULT_FR_NOT_READY => sys::ENODEV,
        sys::FRESULT_FR_INVALID_OBJECT => sys::EBADF,
        sys::FRESULT_FR_NO_FILE | sys::FRESULT_FR_NO_PATH => sys::ENOENT,
        _ => sys::EIO,
    };
    io::Error::from_raw_os_error(code as i32)
}

/// The mounted volume, from an `f_opendir` of its root (cached; the mount
/// id is checked so a remount is noticed).
fn volume() -> io::Result<*mut sys::FATFS> {
    let fs = FS.load(Relaxed);
    // SAFETY: the FATFS lives as long as the mount, which is the program's.
    if !fs.is_null() && unsafe { (*fs).fs_type != 0 && (*fs).id as u32 == FS_ID.load(Relaxed) } {
        return Ok(fs);
    }
    let (fs, _) = open_dir("")?;
    // SAFETY: as above.
    unsafe {
        // `ssize` exists only when FatFs supports more than one sector size
        // (FF_MAX_SS 4096 from CONFIG_WL_SECTOR_SIZE); otherwise it is 512.
        #[cfg(esp_idf_wl_sector_size_4096)]
        let ss_ok = (*fs).ssize as usize == SS;
        #[cfg(not(esp_idf_wl_sector_size_4096))]
        let ss_ok = true;
        if (*fs).fs_type != 3 || !ss_ok {
            return Err(io::Error::other("fatidx: FAT32 with 512 B sectors only"));
        }
        FS_ID.store((*fs).id as u32, Relaxed);
    }
    FS.store(fs, Relaxed);
    Ok(fs)
}

/// `f_opendir` a card directory: its volume and start cluster.
fn open_dir(card_dir: &str) -> io::Result<(*mut sys::FATFS, u32)> {
    let path = std::ffi::CString::new(format!("0:{}", if card_dir.is_empty() { "/" } else { card_dir }))
        .map_err(io::Error::other)?;
    // SAFETY: FF_DIR is plain data; f_opendir fills it, f_closedir releases nothing
    // but its validity (no lock table: FF_FS_LOCK = 0).
    unsafe {
        let mut d: sys::FF_DIR = std::mem::zeroed();
        let r = sys::f_opendir(&mut d, path.as_ptr());
        if r != sys::FRESULT_FR_OK {
            return Err(fr_err(r));
        }
        let out = (d.obj.fs, d.obj.sclust);
        sys::f_closedir(&mut d);
        Ok(out)
    }
}

/// The volume mutex FatFs takes in every call (`lock_volume`).
struct VolLock(i32);
impl VolLock {
    fn take(fs: *mut sys::FATFS) -> io::Result<VolLock> {
        // SAFETY: ldrv is the volume's mutex index; the call is what FatFs makes.
        let vol = unsafe { (*fs).ldrv } as i32;
        if unsafe { sys::ff_mutex_take(vol) } == 0 {
            return Err(fr_err(sys::FRESULT_FR_TIMEOUT));
        }
        Ok(VolLock(vol))
    }
}
impl Drop for VolLock {
    fn drop(&mut self) {
        // SAFETY: taken in `take`.
        unsafe { sys::ff_mutex_give(self.0) };
    }
}

/// Read `n` sectors from `lba` into `buf`, coherent with FatFs's window:
/// a sector FatFs holds in `win` (perhaps dirty, not yet written) comes from
/// there. The caller holds the volume lock.
unsafe fn read_sectors(fs: *mut sys::FATFS, lba: u32, n: usize, buf: &mut [u8]) -> io::Result<()> {
    let r = sys::ff_disk_read((*fs).pdrv, buf.as_mut_ptr(), lba, n as u32);
    if r != 0 {
        return Err(io::Error::from_raw_os_error(sys::EIO as i32));
    }
    let w = (*fs).winsect;
    if w >= lba && w < lba + n as u32 {
        let at = (w - lba) as usize * SS;
        std::ptr::copy_nonoverlapping(std::ptr::addr_of!((*fs).win).cast::<u8>(), buf[at..at + SS].as_mut_ptr(), SS);
    }
    Ok(())
}

/// The cluster after `clust` in its FAT32 chain, or None at the end.
fn next_cluster(fs: *mut sys::FATFS, clust: u32, buf: &mut [u8]) -> io::Result<Option<u32>> {
    let _l = VolLock::take(fs)?;
    // SAFETY: lock held; fields are the mounted volume's geometry.
    unsafe {
        let lba = (*fs).fatbase + clust / 128;
        read_sectors(fs, lba, 1, &mut buf[..SS])?;
        let o = (clust as usize % 128) * 4;
        let v = u32::from_le_bytes(buf[o..o + 4].try_into().unwrap()) & 0x0FFF_FFFF;
        Ok((v >= 2 && v < (*fs).n_fatent).then_some(v))
    }
}

const LFN_OFS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

fn sfn_sum(e: &[u8]) -> u8 {
    e[..11].iter().fold(0u8, |s, &b| (s >> 1 | s << 7).wrapping_add(b))
}

/// The name an 8.3 entry shows with no long name (NT case bits applied).
fn sfn_name(e: &[u8], out: &mut Vec<u16>) {
    out.clear();
    let lower_body = e[12] & 0x08 != 0;
    let lower_ext = e[12] & 0x10 != 0;
    for (i, &b) in e[..8].iter().enumerate() {
        if b == b' ' {
            break;
        }
        let b = if i == 0 && b == 0x05 { 0xE5 } else { b };
        out.push(if lower_body { b.to_ascii_lowercase() } else { b } as u16);
    }
    if e[8] != b' ' {
        out.push(b'.' as u16);
        for &b in &e[8..11] {
            if b == b' ' {
                break;
            }
            out.push(if lower_ext { b.to_ascii_lowercase() } else { b } as u16);
        }
    }
}

pub struct BuildStats {
    pub dirs: u32,
    pub files: u32,
    pub sectors: u32,
    pub walk_ms: u32,
    pub write_ms: u32,
    pub passes: u32,
    pub bytes: u64,
}

/// Walk `card_dirs` (each a card directory, e.g. `/gifs/Consoles`) and write
/// the index. Card I/O only through FatFs's lock in short batches.
pub fn build(card_dirs: &[String]) -> io::Result<BuildStats> {
    let t0 = Instant::now();
    let fs = volume()?;
    let mut counts = vec![0u32; 256];
    let mut st = BuildStats { dirs: 0, files: 0, sectors: 0, walk_ms: 0, write_ms: 0, passes: 0, bytes: 0 };
    let mut tmp = BufWriter::with_capacity(1024, File::create(TMP)?);
    let mut buf = vec![0u8; SS * BATCH];
    let mut fat = vec![0u8; SS];
    let mut lfn: Vec<u16> = Vec::with_capacity(260);
    let mut name16: Vec<u16> = Vec::with_capacity(260);
    let mut name = String::with_capacity(300);
    // SAFETY: geometry fields of the mounted volume, fixed while mounted.
    let (csize, database) = unsafe { ((*fs).csize as u32, (*fs).database) };
    for dir in card_dirs {
        let (_, sclust) = open_dir(dir)?;
        if sclust < 2 {
            continue;
        }
        st.dirs += 1;
        let mut clust = Some(sclust);
        let (mut lfn_sum, mut lfn_next) = (0u8, 0u8);
        'dir: while let Some(c) = clust {
            let first = database + (c - 2) * csize;
            let mut s = 0;
            while s < csize {
                let n = (BATCH as u32).min(csize - s) as usize;
                {
                    let _l = VolLock::take(fs)?;
                    // SAFETY: lock held.
                    unsafe { read_sectors(fs, first + s, n, &mut buf[..n * SS])? };
                }
                st.sectors += n as u32;
                for k in 0..n {
                    let rel = first + s + k as u32 - database;
                    for slot in 0..16 {
                        let e = &buf[k * SS + slot * 32..][..32];
                        match e[0] {
                            0x00 => break 'dir,
                            0xE5 => {
                                lfn.clear();
                                lfn_next = 0;
                                continue;
                            }
                            _ => {}
                        }
                        let attr = e[11] & 0x3F;
                        if attr == 0x0F {
                            let ord = e[0];
                            if ord & 0x40 != 0 {
                                lfn.clear();
                                lfn.resize((ord & 0x3F) as usize * 13, 0xFFFF);
                                lfn_sum = e[13];
                                lfn_next = ord & 0x3F;
                            } else if ord != lfn_next || e[13] != lfn_sum {
                                lfn.clear();
                                lfn_next = 0;
                                continue;
                            }
                            if lfn_next == 0 {
                                continue;
                            }
                            let base = (lfn_next as usize - 1) * 13;
                            for (j, &o) in LFN_OFS.iter().enumerate() {
                                lfn[base + j] = u16::from_le_bytes([e[o], e[o + 1]]);
                            }
                            lfn_next -= 1;
                            continue;
                        }
                        let had_lfn = lfn_next == 0 && !lfn.is_empty() && lfn_sum == sfn_sum(e);
                        if attr & 0x18 == 0 {
                            if had_lfn {
                                let end = lfn.iter().position(|&c| c == 0 || c == 0xFFFF).unwrap_or(lfn.len());
                                name16.clear();
                                name16.extend_from_slice(&lfn[..end]);
                            } else {
                                sfn_name(e, &mut name16);
                            }
                            name.clear();
                            name.push_str(dir);
                            name.push('/');
                            name.extend(char::decode_utf16(name16.iter().copied()).map(|c| c.unwrap_or('?')));
                            let h = fnv(&name);
                            counts[(h >> 24) as usize] += 1;
                            tmp.write_all(&h.to_le_bytes())?;
                            tmp.write_all(&((rel << 4) | slot as u32).to_le_bytes())?;
                            st.files += 1;
                        }
                        lfn.clear();
                        lfn_next = 0;
                    }
                }
                s += n as u32;
                std::thread::yield_now();
            }
            clust = next_cluster(fs, c, &mut fat)?;
        }
    }
    drop(buf);
    tmp.flush()?;
    drop(tmp);
    st.walk_ms = t0.elapsed().as_millis() as u32;

    // Group by bucket: passes over the temp file, each gathering the buckets
    // that fit PASS_RECORDS into RAM.
    let t1 = Instant::now();
    let mut fanout = Box::new([0u32; FANOUT]);
    for b in 0..256 {
        fanout[b + 1] = fanout[b] + counts[b];
    }
    let total = fanout[256];
    *LOADED.lock().unwrap() = None;
    let mut out = BufWriter::with_capacity(1024, File::create(NEW)?);
    let mut hdr = [0u8; HEADER as usize];
    hdr[..4].copy_from_slice(MAGIC);
    hdr[4..6].copy_from_slice(&1u16.to_le_bytes());
    hdr[8..12].copy_from_slice(&total.to_le_bytes());
    // SAFETY: geometry of the mounted volume.
    unsafe {
        hdr[12..16].copy_from_slice(&(*fs).n_fatent.to_le_bytes());
        hdr[16..20].copy_from_slice(&database.to_le_bytes());
    }
    hdr[20..22].copy_from_slice(&(csize as u16).to_le_bytes());
    out.write_all(&hdr)?;
    for v in fanout.iter() {
        out.write_all(&v.to_le_bytes())?;
    }
    let mut group = vec![0u8; PASS_RECORDS * 8];
    let mut fill = vec![0u32; 256];
    let mut b0 = 0usize;
    while b0 < 256 {
        let mut b1 = b0;
        while b1 < 256 && fanout[b1 + 1] - fanout[b0] <= PASS_RECORDS as u32 {
            b1 += 1;
        }
        if b1 == b0 {
            return Err(io::Error::other("fatidx: a bucket larger than a pass"));
        }
        st.passes += 1;
        fill[b0..b1].iter_mut().for_each(|f| *f = 0);
        let mut rd = BufReader::with_capacity(1024, File::open(TMP)?);
        let mut rec = [0u8; 8];
        loop {
            match rd.read_exact(&mut rec) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            let b = rec[3] as usize;
            if b >= b0 && b < b1 {
                let at = (fanout[b] - fanout[b0] + fill[b]) as usize * 8;
                group[at..at + 8].copy_from_slice(&rec);
                fill[b] += 1;
            }
        }
        let n = (fanout[b1] - fanout[b0]) as usize * 8;
        out.write_all(&group[..n])?;
        b0 = b1;
    }
    out.flush()?;
    drop(out);
    let _ = fs::remove_file(INDEX);
    fs::rename(NEW, INDEX)?;
    let _ = fs::remove_file(TMP);
    st.write_ms = t1.elapsed().as_millis() as u32;
    st.bytes = RECORDS_AT + total as u64 * 8;
    Ok(st)
}

fn load() -> io::Result<Loaded> {
    let mut file = File::open(INDEX)?;
    let mut hdr = [0u8; HEADER as usize];
    file.read_exact(&mut hdr)?;
    if &hdr[..4] != MAGIC || hdr[4..6] != 1u16.to_le_bytes() {
        return Err(io::Error::other("fatidx: not an index"));
    }
    let fs = volume()?;
    // SAFETY: geometry of the mounted volume.
    let geo_ok = unsafe {
        hdr[12..16] == (*fs).n_fatent.to_le_bytes() && hdr[16..20] == (*fs).database.to_le_bytes()
    };
    if !geo_ok {
        return Err(io::Error::other("fatidx: built on another volume"));
    }
    let mut raw = vec![0u8; FANOUT * 4];
    file.read_exact(&mut raw)?;
    let mut fanout = Box::new([0u32; FANOUT]);
    for (i, c) in raw.chunks_exact(4).enumerate() {
        fanout[i] = u32::from_le_bytes(c.try_into().unwrap());
    }
    Ok(Loaded { file, fanout })
}

/// Drop the loaded index (its file handle) — before the card file changes.
pub fn unload() {
    *LOADED.lock().unwrap() = None;
}

enum Verify {
    Ok(IdxFile),
    Mismatch,
    CrossCluster,
}

/// Open `card` (`/gifs/<dir>/<name>.gif`) through the index. `None`: not
/// indexed, stale, or not indexable — the caller opens by path.
pub fn open(card: &str) -> Option<io::Result<IdxFile>> {
    let t = Instant::now();
    let name = card.rsplit('/').next()?;
    if !name.is_ascii() || name.is_empty() || name.len() > 255 {
        OTHER.fetch_add(1, Relaxed);
        return None;
    }
    let h = fnv(card);
    let mut cands = [0u32; 16];
    let mut nc = 0;
    {
        let mut g = LOADED.lock().unwrap();
        if g.is_none() {
            match load() {
                Ok(l) => *g = Some(l),
                Err(_) => {
                    OTHER.fetch_add(1, Relaxed);
                    return None;
                }
            }
        }
        let l = g.as_mut()?;
        let b = (h >> 24) as usize;
        let (lo, hi) = (l.fanout[b], l.fanout[b + 1]);
        let mut left = hi - lo;
        if let Err(e) = l.file.seek(SeekFrom::Start(RECORDS_AT + lo as u64 * 8)) {
            return Some(Err(e));
        }
        let mut chunk = [0u8; 256];
        while left > 0 && nc < cands.len() {
            let n = (left as usize).min(32);
            if let Err(e) = l.file.read_exact(&mut chunk[..n * 8]) {
                return Some(Err(e));
            }
            for r in chunk[..n * 8].chunks_exact(8) {
                if u32::from_le_bytes(r[..4].try_into().unwrap()) == h && nc < cands.len() {
                    cands[nc] = u32::from_le_bytes(r[4..].try_into().unwrap());
                    nc += 1;
                }
            }
            left -= n as u32;
        }
    }
    if nc == 0 {
        ABSENT.fetch_add(1, Relaxed);
        return None;
    }
    let name16: Vec<u16> = name.bytes().map(u16::from).collect();
    let mut xc = false;
    for &loc in &cands[..nc] {
        match verify(loc, &name16) {
            Ok(Verify::Ok(f)) => {
                let us = t.elapsed().as_micros() as u32;
                HITS.fetch_add(1, Relaxed);
                HIT_US.fetch_add(us, Relaxed);
                HIT_MAX_US.fetch_max(us, Relaxed);
                return Some(Ok(f));
            }
            Ok(Verify::Mismatch) => {}
            Ok(Verify::CrossCluster) => xc = true,
            Err(e) => return Some(Err(e)),
        }
    }
    if xc { &XCLUST } else { &STALE }.fetch_add(1, Relaxed);
    None
}

fn verify(loc: u32, name16: &[u16]) -> io::Result<Verify> {
    let fs = volume()?;
    let rel = loc >> 4;
    let slot = (loc & 15) as usize;
    let nent = name16.len().div_ceil(13);
    // Two sectors: [0..512) the one before, [512..1024) the entry's.
    let mut buf = vec![0u8; 2 * SS];
    let mut sfn = Vec::with_capacity(12);
    let _l = VolLock::take(fs)?;
    // SAFETY: lock held for every access to the volume below.
    unsafe {
        let csize = (*fs).csize as u32;
        if rel >= ((*fs).n_fatent - 2) * csize {
            return Ok(Verify::Mismatch);
        }
        let lba = (*fs).database + rel;
        read_sectors(fs, lba, 1, &mut buf[SS..])?;
        let e = &buf[SS + slot * 32..][..32];
        let attr = e[11];
        if e[0] == 0 || e[0] == 0xE5 || attr & 0x3F == 0x0F || attr & 0x18 != 0 {
            return Ok(Verify::Mismatch);
        }
        // FatFs's own lookup matches a name against the long name or the
        // 8.3 one, so either is this file. The 8.3 one first: no I/O.
        sfn_name(e, &mut sfn);
        if sfn != name16 {
            if nent == 0 || nent > 20 {
                return Ok(Verify::Mismatch);
            }
            if slot < nent {
                if rel % csize == 0 {
                    // The long name starts in the previous cluster, which
                    // the index does not locate: open by path.
                    return Ok(Verify::CrossCluster);
                }
                read_sectors(fs, lba - 1, 1, &mut buf[..SS])?;
            }
            let sum = sfn_sum(&buf[SS + slot * 32..][..32]);
            for k in 1..=nent {
                let l = &buf[(16 + slot - k) * 32..][..32];
                let ord = (k as u8) | if k == nent { 0x40 } else { 0 };
                if l[11] & 0x3F != 0x0F || l[13] != sum || l[0] != ord {
                    return Ok(Verify::Mismatch);
                }
                for (j, &o) in LFN_OFS.iter().enumerate() {
                    let pos = (k - 1) * 13 + j;
                    let want = match pos.cmp(&name16.len()) {
                        std::cmp::Ordering::Less => name16[pos],
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 0xFFFF,
                    };
                    if u16::from_le_bytes([l[o], l[o + 1]]) != want {
                        return Ok(Verify::Mismatch);
                    }
                }
            }
        }
        let e = &buf[SS + slot * 32..][..32];
        let layout = std::alloc::Layout::new::<sys::FIL>();
        let p = std::alloc::alloc_zeroed(layout) as *mut sys::FIL;
        if p.is_null() {
            return Err(io::Error::from(io::ErrorKind::OutOfMemory));
        }
        let mut fil = Box::from_raw(p);
        fil.obj.fs = fs;
        fil.obj.id = (*fs).id;
        fil.obj.attr = attr;
        fil.obj.stat = 0;
        fil.obj.sclust = (u16::from_le_bytes([e[20], e[21]]) as u32) << 16 | u16::from_le_bytes([e[26], e[27]]) as u32;
        fil.obj.objsize = u32::from_le_bytes(e[28..32].try_into().unwrap());
        fil.flag = sys::FA_READ as u8;
        fil.dir_sect = lba;
        Ok(Verify::Ok(IdxFile(fil)))
    }
}

/// A file opened through the index: FatFs's own `f_read`/`f_lseek` on a
/// `FIL` we filled (each takes the volume lock itself).
pub struct IdxFile(Box<sys::FIL>);

// SAFETY: like a VFS file descriptor, the FIL may be used from any task;
// FatFs serialises access to the volume.
unsafe impl Send for IdxFile {}

impl IdxFile {
    pub fn len(&self) -> u64 {
        self.0.obj.objsize as u64
    }
}

impl Read for IdxFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut br: sys::UINT = 0;
        // SAFETY: a FIL filled in `verify`; buf is valid for its length.
        let r = unsafe { sys::f_read(&mut *self.0, buf.as_mut_ptr().cast(), buf.len() as sys::UINT, &mut br) };
        if r != sys::FRESULT_FR_OK {
            return Err(fr_err(r));
        }
        Ok(br as usize)
    }
}

impl Seek for IdxFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let len = self.len() as i64;
        let to = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(d) => len + d,
            SeekFrom::Current(d) => self.0.fptr as i64 + d,
        };
        if to < 0 {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        // SAFETY: as in `read`.
        let r = unsafe { sys::f_lseek(&mut *self.0, to as sys::FSIZE_t) };
        if r != sys::FRESULT_FR_OK {
            return Err(fr_err(r));
        }
        Ok(self.0.fptr as u64)
    }
}

impl Drop for IdxFile {
    fn drop(&mut self) {
        // SAFETY: read-only FIL: f_close only invalidates it (no lock table).
        unsafe { sys::f_close(&mut *self.0) };
    }
}

pub fn stats_json() -> String {
    let hits = HITS.load(Relaxed);
    format!(
        r#"{{"player":{},"server":{},"hits":{hits},"absent":{},"stale":{},"xclust":{},"other":{},"hit_avg_us":{},"hit_max_us":{},"loaded":{},"fil_bytes":{}}}"#,
        PLAYER.load(Relaxed),
        SERVER.load(Relaxed),
        ABSENT.load(Relaxed),
        STALE.load(Relaxed),
        XCLUST.load(Relaxed),
        OTHER.load(Relaxed),
        HIT_US.load(Relaxed).checked_div(hits).unwrap_or(0),
        HIT_MAX_US.load(Relaxed),
        LOADED.lock().unwrap().is_some(),
        std::mem::size_of::<sys::FIL>(),
    )
}

pub fn reset_stats() {
    for a in [&HITS, &ABSENT, &STALE, &XCLUST, &OTHER, &HIT_US, &HIT_MAX_US] {
        a.store(0, Relaxed);
    }
}

/// Every directory under `/gifs` (the build's default).
pub fn gif_dirs() -> io::Result<Vec<String>> {
    let mut v = Vec::new();
    for e in fs::read_dir(format!("{}/gifs", config::SD_MOUNT))? {
        let e = e?;
        if e.file_type()?.is_dir() {
            v.push(format!("/gifs/{}", e.file_name().to_string_lossy()));
        }
    }
    Ok(v)
}

/// The bench: for `n` paths of a list file from line `from`, open each
/// through the index (first, so its directory sector is not the one the
/// path open's scan just left in FatFs's window) and by path, read both
/// whole and compare. Times only the opens.
pub fn bench(list: &str, from: usize, n: usize) -> io::Result<String> {
    let f = BufReader::with_capacity(512, File::open(format!("{}{list}", config::SD_MOUNT))?);
    let mut idx_us = Vec::with_capacity(n);
    let mut path_us = Vec::with_capacity(n);
    let (mut same, mut differ, mut fallback) = (0, 0, 0);
    let mut buf = vec![0u8; 2048];
    let crc = |r: &mut dyn Read, buf: &mut [u8]| -> io::Result<(u32, u64)> {
        let (mut h, mut len) = (0x811c_9dc5u32, 0u64);
        loop {
            let k = r.read(buf)?;
            if k == 0 {
                return Ok((h, len));
            }
            for &b in &buf[..k] {
                h = (h ^ b as u32).wrapping_mul(0x0100_0193);
            }
            len += k as u64;
        }
    };
    for line in io::BufRead::lines(f).skip(from).take(n) {
        let card = line?;
        let card = card.trim();
        if card.is_empty() {
            continue;
        }
        let t = Instant::now();
        let a = open(card);
        let ia = t.elapsed().as_micros() as u32;
        let a = match a {
            Some(Ok(mut f)) => {
                idx_us.push(ia);
                Some(crc(&mut f, &mut buf)?)
            }
            Some(Err(e)) => return Err(e),
            None => {
                fallback += 1;
                None
            }
        };
        let t = Instant::now();
        let mut p = File::open(format!("{}{card}", config::SD_MOUNT))?;
        path_us.push(t.elapsed().as_micros() as u32);
        let b = crc(&mut p, &mut buf)?;
        drop(p);
        match a {
            Some(a) if a == b => same += 1,
            Some(_) => differ += 1,
            None => {}
        }
        std::thread::yield_now();
    }
    let list_json = |v: &[u32]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",");
    Ok(format!(
        r#"{{"from":{from},"same":{same},"differ":{differ},"fallback":{fallback},"idx_us":[{}],"path_us":[{}]}}"#,
        list_json(&idx_us),
        list_json(&path_us)
    ))
}
