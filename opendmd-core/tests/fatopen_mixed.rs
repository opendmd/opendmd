//! The player's index opens without FatFs's volume lock (`fatidx::open_at`,
//! display-isolation 2026-11-02): the file's 8.3 entry and its FAT chain
//! are read from the card through the card's I/O lock alone, while a FatFs
//! call on another task may be part-way through changing the same
//! directory sector or FAT sector (its window holding a newer copy than
//! the card, its other sectors written or not yet).
//!
//! The claim: for a file the write does not touch (a write intent covers
//! the ones it does: those opens are refused), every mixture of the card's
//! sectors from before the write and after it gives the same entry, the
//! same chain and the same bytes. Tested on real FAT32 images (mkfs.fat,
//! mtools): for a create, a delete and a rename in the file's own
//! directory sector, writes that change its FAT sector, and a directory
//! growing a cluster, every sector the write changed is taken from before
//! or after in many random mixtures (and both extremes), and every
//! untouched file is opened from each mixture by its record from before.

use opendmd_core::fatdir::{self, Found, Geometry, Verify, SS};
use opendmd_core::fatread::{chain, RawFile};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const MKFS: &str = "/sbin/mkfs.fat";

fn tools() -> bool {
    let ok = Path::new(MKFS).exists() && Command::new("mcopy").arg("--version").output().is_ok();
    if !ok && std::env::var_os("OPENDMD_SKIP_FAT_IMAGES").is_none() {
        panic!("{MKFS} or mtools missing: install dosfstools and mtools (OPENDMD_SKIP_FAT_IMAGES=1 skips)");
    }
    ok
}

fn run(cmd: &mut Command) {
    let out = cmd.output().expect("spawn");
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn geometry(b: &[u8]) -> (Geometry, u32) {
    let u16_ = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u32;
    let u32_ = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let csize = b[13] as u32;
    let fatbase = u16_(14);
    let database = fatbase + b[16] as u32 * u32_(36);
    let total = if u16_(19) != 0 { u16_(19) } else { u32_(32) };
    (Geometry { csize, fatbase, database, n_fatent: (total - database) / csize + 2 }, u32_(44))
}

/// A card image in memory.
#[derive(Clone)]
struct Card(Vec<u8>);

impl Card {
    fn read(&self, lba: u32, buf: &mut [u8]) -> std::io::Result<()> {
        let at = lba as usize * SS;
        let src = self.0.get(at..at + buf.len()).ok_or_else(|| std::io::Error::other("past the card"))?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

/// Files of the root directory: name -> (loc, prev_rel), by the index's walk.
fn records(card: &Card, geo: &Geometry, root: u32) -> HashMap<String, (u32, Option<u32>)> {
    let mut out = HashMap::new();
    let mut scratch = vec![0u8; SS * 8];
    fatdir::walk(geo, root, &mut |l, b| card.read(l, b), &mut scratch, &mut |f: Found| {
        out.insert(String::from_utf8(f.name.to_vec()).unwrap(), (f.loc, f.prev_rel));
        Ok(())
    })
    .unwrap();
    out
}

/// Open by record from `card` as `open_at` does (verify the entry, read the
/// chain), and read the whole file: `None` if the entry does not verify.
fn open_read(card: &Card, geo: &Geometry, loc: u32, prev_rel: Option<u32>, name: &str) -> Option<Vec<u8>> {
    let (rel, slot) = (loc >> 4, (loc & 15) as usize);
    let mut cur = [0u8; SS];
    card.read(geo.database + rel, &mut cur).ok()?;
    let mut v = fatdir::verify(&cur, None, slot, name.as_bytes());
    if v == Verify::NeedPrev {
        let p = if rel % geo.csize != 0 { rel - 1 } else { prev_rel? };
        let mut prev = [0u8; SS];
        card.read(geo.database + p, &mut prev).ok()?;
        v = fatdir::verify(&cur, Some(&prev), slot, name.as_bytes());
    }
    let Verify::Ok { cluster, size, .. } = v else { return None };
    let ext = chain(geo, cluster, size, &mut |l, b| card.read(l, b)).ok()?;
    let mut f = RawFile::new(ext);
    let mut out = vec![0u8; size as usize];
    let mut dev = |lba: u32, n: u32, b: &mut [u8]| {
        let _ = n;
        card.read(lba, b)
    };
    let mut done = 0;
    while done < out.len() {
        let n = f.read(&mut out[done..], &mut dev).ok()?;
        if n == 0 {
            break;
        }
        done += n;
    }
    Some(out)
}

/// A tiny PRNG for the mixtures (no dependency).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

struct Img {
    path: PathBuf,
    _dir: tempdir::Dir,
}

impl Img {
    fn new(cluster_sectors: u32, kb: u32) -> Option<Img> {
        if !tools() {
            return None;
        }
        let dir = tempdir::Dir::new();
        let path = dir.0.join("card.img");
        run(Command::new(MKFS).args(["-C", "-F", "32", "-S", "512", "-s", &cluster_sectors.to_string()]).arg(&path).arg(kb.to_string()));
        Some(Img { path, _dir: dir })
    }
    fn put(&self, name: &str, bytes: &[u8]) {
        let src = self._dir.0.join(format!("src-{name}"));
        fs::write(&src, bytes).unwrap();
        run(Command::new("mcopy").env("LC_ALL", "C.UTF-8").arg("-i").arg(&self.path).arg("-o").arg(&src).arg(format!("::/{name}")));
    }
    fn del(&self, name: &str) {
        run(Command::new("mdel").arg("-i").arg(&self.path).arg(format!("::/{name}")));
    }
    fn ren(&self, from: &str, to: &str) {
        run(Command::new("mren").arg("-i").arg(&self.path).arg(format!("::/{from}")).arg(to));
    }
    fn card(&self) -> Card {
        let mut v = Vec::new();
        fs::File::open(&self.path).unwrap().read_to_end(&mut v).unwrap();
        Card(v)
    }
}

fn body(seed: u32, n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i as u32).wrapping_mul(2_654_435_761) ^ seed.wrapping_mul(40503)) as u8).collect()
}

/// Every untouched file opens the same from every mixture of `before` and
/// `after` (the sectors the write changed, each from one or the other).
fn check(before: &Card, after: &Card, touched: &[&str], what: &str) {
    let (g, _) = geometry(&before.0[..SS]);
    let fat_end = g.fatbase + (g.n_fatent * 4).div_ceil(SS as u32);
    let (geo, root) = geometry(&before.0[..SS]);
    let recs = records(before, &geo, root);
    let want: HashMap<&String, Vec<u8>> = recs
        .iter()
        .filter(|(n, _)| !touched.contains(&n.as_str()))
        .map(|(n, (loc, pr))| (n, open_read(before, &geo, *loc, *pr, n).unwrap_or_else(|| panic!("{what}: {n} does not open before"))))
        .collect();
    assert!(!want.is_empty());
    let changed: Vec<usize> = (0..before.0.len() / SS).filter(|&s| before.0[s * SS..(s + 1) * SS] != after.0[s * SS..(s + 1) * SS]).collect();
    assert!(!changed.is_empty(), "{what}: the write changed nothing");
    // It changed a directory sector and a FAT sector the untouched files'
    // records and chains share (else the test proves nothing).
    if what != "rename" {
        assert!(changed.iter().any(|&s| (s as u32) >= g.fatbase && (s as u32) < fat_end), "{what}: no FAT sector changed");
    }
    assert!(changed.iter().any(|&s| (s as u32) >= g.database), "{what}: no directory or data sector changed");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ changed.len() as u64);
    for k in 0..200 {
        let mut mixed = before.clone();
        for &s in &changed {
            let take = match k {
                0 => false,
                1 => true,
                _ => rng.next() & 1 == 1,
            };
            if take {
                mixed.0[s * SS..(s + 1) * SS].copy_from_slice(&after.0[s * SS..(s + 1) * SS]);
            }
        }
        for (n, bytes) in &want {
            let (loc, pr) = recs[*n];
            let got = open_read(&mixed, &geo, loc, pr, n);
            assert!(got.as_deref() == Some(&bytes[..]), "{what}: {n} differs in mixture {k} ({} sectors changed)", changed.len());
        }
    }
}

/// Small clusters (one sector): many files share a directory sector and a
/// FAT sector, and a directory grows a cluster after 16 entries.
fn card_with_files() -> Option<Img> {
    let img = Img::new(1, 20_000)?;
    for k in 0..12 {
        img.put(&format!("NES_Existing_Game_{k:02}.gif"), &body(k, 300 + 700 * k as usize));
    }
    Some(img)
}

#[test]
fn a_create_in_the_same_directory_and_fat_sectors() {
    let Some(img) = card_with_files() else { return };
    let before = img.card();
    img.put("NES_Brand_New_Arrival_01.gif", &body(99, 5000));
    check(&before, &img.card(), &["NES_Brand_New_Arrival_01.gif"], "create");
}

#[test]
fn a_delete_in_the_same_directory_and_fat_sectors() {
    let Some(img) = card_with_files() else { return };
    let before = img.card();
    img.del("NES_Existing_Game_05.gif");
    check(&before, &img.card(), &["NES_Existing_Game_05.gif"], "delete");
}

#[test]
fn a_rename_in_the_same_directory_sector() {
    let Some(img) = card_with_files() else { return };
    let before = img.card();
    img.ren("NES_Existing_Game_07.gif", "NES_Renamed_To_Something_Longer_07.gif");
    check(&before, &img.card(), &["NES_Existing_Game_07.gif", "NES_Renamed_To_Something_Longer_07.gif"], "rename");
}

#[test]
fn a_directory_growing_a_cluster_mid_open() {
    let Some(img) = card_with_files() else { return };
    let before = img.card();
    // Long names (3 entries each): the root's one-sector clusters fill and
    // it is given more.
    let mut names = Vec::new();
    for k in 0..10 {
        let n = format!("SNES_A_Much_Longer_Name_For_Growth_{k:02}.gif");
        img.put(&n, &body(200 + k, 900));
        names.push(n);
    }
    let after = img.card();
    let clusters = |c: &Card| {
        let (geo, root) = geometry(&c.0[..SS]);
        let (mut n, mut cl) = (1, root);
        while let Some(next) = fatdir::next_cluster(&geo, cl, &mut |l, b| c.read(l, b), &mut [0u8; SS]).unwrap() {
            n += 1;
            cl = next;
        }
        n
    };
    assert!(clusters(&after) > clusters(&before), "the root did not grow");
    let touched: Vec<&str> = names.iter().map(String::as_str).collect();
    check(&before, &after, &touched, "growth");
}

#[test]
fn a_delete_then_reuse_of_the_freed_clusters() {
    let Some(img) = card_with_files() else { return };
    let before = img.card();
    img.del("NES_Existing_Game_03.gif");
    img.put("NES_Reuses_The_Space_01.gif", &body(7, 2600));
    check(&before, &img.card(), &["NES_Existing_Game_03.gif", "NES_Reuses_The_Space_01.gif"], "delete+reuse");
}

mod tempdir {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new() -> Dir {
            let p = std::env::temp_dir().join(format!(
                "fatopen-test-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

