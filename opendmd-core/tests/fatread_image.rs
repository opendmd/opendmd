//! `fatread` (the player's reads without FatFs's volume lock) against real
//! FAT32 images made by `mkfs.fat` and filled by mtools (both must be
//! installed: the test says so and passes without them). Every file is read
//! through its extents and compared, byte for byte, with what mtools reads
//! back, at the cluster sizes a card may have (512 B to 32 KB) and with:
//! a fragmented file (written into the holes deleted files left), files
//! that end exactly at a cluster's end, an empty and a one-byte file, odd
//! read sizes and seeks, a card error mid-read (the position is left for
//! the fallback to read the same bytes), and what a stale extent map reads
//! after the file was deleted and its clusters reused (why every writer
//! must find the file let go first: `OpenSet`).

use opendmd_core::fatdir::{self, Found, Geometry, Verify, SS};
use opendmd_core::fatread::{chain, Extents, OpenSet, RawFile, Sectors};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

const MKFS: &str = "/sbin/mkfs.fat";

fn tools() -> bool {
    Path::new(MKFS).exists() && Command::new("mcopy").arg("--version").output().is_ok()
}

fn run(cmd: &mut Command) {
    let out = cmd.output().expect("spawn");
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
}

struct Img {
    path: PathBuf,
    geo: Geometry,
    root: u32,
}

impl Img {
    fn open(path: &Path) -> Img {
        let mut f = fs::File::open(path).unwrap();
        let mut b = [0u8; SS];
        f.read_exact(&mut b).unwrap();
        let u16_ = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u32;
        let u32_ = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!(u16_(11), 512);
        let csize = b[13] as u32;
        let fatbase = u16_(14);
        let database = fatbase + b[16] as u32 * u32_(36);
        let total = if u16_(19) != 0 { u16_(19) } else { u32_(32) };
        let n_fatent = (total - database) / csize + 2;
        Img { path: path.to_path_buf(), geo: Geometry { csize, fatbase, database, n_fatent }, root: u32_(44) }
    }

    fn reader(&self) -> impl FnMut(u32, &mut [u8]) -> io::Result<()> {
        let mut f = fs::File::open(&self.path).unwrap();
        move |lba, buf| {
            f.seek(SeekFrom::Start(lba as u64 * SS as u64))?;
            f.read_exact(buf)
        }
    }

    /// name -> (first cluster, size), in the root directory.
    fn files(&self) -> HashMap<String, (u32, u32)> {
        let mut read = self.reader();
        let mut found = Vec::new();
        let mut scratch = vec![0u8; SS * 8];
        fatdir::walk(&self.geo, self.root, &mut read, &mut scratch, &mut |f: Found| {
            found.push((String::from_utf8(f.name.to_vec()).unwrap(), f.loc, f.prev_rel));
            Ok(())
        })
        .unwrap();
        let mut out = HashMap::new();
        for (name, loc, prev_rel) in found {
            let (rel, slot) = (loc >> 4, (loc & 15) as usize);
            let mut cur = [0u8; SS];
            read(self.geo.database + rel, &mut cur).unwrap();
            let mut v = fatdir::verify(&cur, None, slot, name.as_bytes());
            if v == Verify::NeedPrev {
                // The sector before: in this cluster, or the walk's record
                // of where the name started.
                let p = if rel % self.geo.csize != 0 { rel - 1 } else { prev_rel.expect("a name across clusters") };
                let mut prev = [0u8; SS];
                read(self.geo.database + p, &mut prev).unwrap();
                v = fatdir::verify(&cur, Some(&prev), slot, name.as_bytes());
            }
            match v {
                Verify::Ok { cluster, size, .. } => {
                    out.insert(name, (cluster, size));
                }
                v => panic!("{name}: {v:?}"),
            }
        }
        out
    }

    fn extents(&self, name: &str) -> Extents {
        let (first, size) = self.files()[name];
        let mut read = self.reader();
        chain(&self.geo, first, size, &mut |lba, b| read(lba, b)).unwrap()
    }

    /// What mtools reads back.
    fn mtype(&self, name: &str) -> Vec<u8> {
        let out = Command::new("mtype").arg("-i").arg(&self.path).arg(format!("::/{name}")).output().unwrap();
        assert!(out.status.success());
        out.stdout
    }
}

/// The card's sectors, counting calls (and failing one on request).
struct Dev<F: FnMut(u32, &mut [u8]) -> io::Result<()>> {
    read: F,
    calls: usize,
    fail_at: Option<usize>,
}

impl<F: FnMut(u32, &mut [u8]) -> io::Result<()>> Sectors for Dev<F> {
    fn read(&mut self, lba: u32, count: u32, buf: &mut [u8]) -> io::Result<()> {
        self.calls += 1;
        if self.fail_at == Some(self.calls) {
            return Err(io::Error::other("card error (test)"));
        }
        assert_eq!(buf.len(), count as usize * SS);
        (self.read)(lba, buf)
    }
}

fn dev(img: &Img) -> Dev<impl FnMut(u32, &mut [u8]) -> io::Result<()>> {
    Dev { read: img.reader(), calls: 0, fail_at: None }
}

/// Read all of `name` in chunks of `chunk` bytes.
fn read_all(img: &Img, name: &str, chunk: usize) -> Vec<u8> {
    let mut f = RawFile::new(img.extents(name));
    let mut d = dev(img);
    let mut out = Vec::new();
    let mut buf = vec![0u8; chunk];
    loop {
        let n = f.read(&mut buf, &mut d).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

fn body(seed: u32, n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i as u32).wrapping_mul(2654435761) ^ seed) as u8).collect()
}

fn image(cluster_sectors: u32, kb: u32) -> Option<(Img, tempdir::Dir)> {
    if !tools() {
        eprintln!("skipping: {MKFS} or mtools missing");
        return None;
    }
    let dir = tempdir::Dir::new();
    let path = dir.0.join("card.img");
    run(Command::new(MKFS).args(["-C", "-F", "32", "-S", "512", "-s", &cluster_sectors.to_string()]).arg(&path).arg(kb.to_string()));
    Some((Img::open(&path), dir))
}

fn put(img: &Img, dir: &Path, name: &str, bytes: &[u8]) {
    let src = dir.join(name);
    fs::write(&src, bytes).unwrap();
    run(Command::new("mcopy").arg("-i").arg(&img.path).arg("-o").arg(&src).arg(format!("::/{name}")));
}

/// Point FSInfo's next-free hint at the volume's start, so the next file
/// written is allocated from the first free cluster on: into the holes.
fn rewind_allocation(img: &Img) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().read(true).write(true).open(&img.path).unwrap();
    let mut b = [0u8; SS];
    f.read_exact(&mut b).unwrap();
    let fsinfo = u16::from_le_bytes([b[48], b[49]]) as u64;
    f.seek(SeekFrom::Start(fsinfo * SS as u64 + 492)).unwrap();
    f.write_all(&2u32.to_le_bytes()).unwrap();
}

fn del(img: &Img, name: &str) {
    run(Command::new("mdel").arg("-i").arg(&img.path).arg(format!("::/{name}")));
}

/// The files of one card: a fragmented one, exact cluster multiples, an
/// empty and a one-byte file.
fn exercise(cluster_sectors: u32, kb: u32) {
    let Some((img, dir)) = image(cluster_sectors, kb) else { return };
    let cb = cluster_sectors as usize * SS;
    // Ten one-cluster files, every other one deleted: holes the next file
    // is written into, so its chain jumps.
    for k in 0..10 {
        put(&img, &dir.0, &format!("Hole_Filler_Number_{k}.bin"), &body(k, cb));
    }
    for k in (0..10).step_by(2) {
        del(&img, &format!("Hole_Filler_Number_{k}.bin"));
    }
    rewind_allocation(&img);
    let frag = body(77, cb * 7 + 123);
    put(&img, &dir.0, "NES_Fragmented_Long_01.gif", &frag);
    let exact = body(5, cb * 3);
    put(&img, &dir.0, "NES_Exact_Clusters_01.gif", &exact);
    let one_cluster = body(6, cb);
    put(&img, &dir.0, "NES_One_Cluster_Only_01.gif", &one_cluster);
    put(&img, &dir.0, "NES_Empty_File_Here_01.gif", &[]);
    put(&img, &dir.0, "NES_Single_Byte_File_01.gif", &[42]);

    let e = img.extents("NES_Fragmented_Long_01.gif");
    assert!(e.runs().len() > 1, "{cluster_sectors}: the fragmented file is not: {:?}", e.runs());
    for (name, want) in [("NES_Fragmented_Long_01.gif", &frag), ("NES_Exact_Clusters_01.gif", &exact), ("NES_One_Cluster_Only_01.gif", &one_cluster)] {
        assert_eq!(&img.mtype(name), want, "{name}: mtools disagrees with what was written");
        for chunk in [1, 7, 511, 512, 513, 4096, cb + 1, 1 << 20] {
            assert_eq!(&read_all(&img, name, chunk), want, "{cluster_sectors}-sector clusters, {name}, {chunk} B reads");
        }
    }
    assert!(read_all(&img, "NES_Empty_File_Here_01.gif", 512).is_empty());
    assert_eq!(read_all(&img, "NES_Single_Byte_File_01.gif", 512), vec![42]);

    // Seeks: anywhere, then reads that straddle sectors, clusters and runs,
    // and past the end.
    let mut f = RawFile::new(img.extents("NES_Fragmented_Long_01.gif"));
    let mut d = dev(&img);
    for &at in &[0usize, 1, 511, 512, cb - 1, cb, cb + 700, frag.len() - 3, frag.len(), frag.len() + 10] {
        f.seek(at as u64);
        let mut buf = vec![0u8; 1500];
        let n = f.read(&mut buf, &mut d).unwrap();
        let want = &frag[at.min(frag.len())..(at + 1500).min(frag.len())];
        assert_eq!(&buf[..n], want, "seek {at}");
        assert_eq!(f.pos(), (at + n) as u64);
    }
}

#[test]
fn reads_match_mtools_at_512_byte_clusters() {
    exercise(1, 20_000);
}

#[test]
fn reads_match_mtools_at_1_kb_clusters() {
    exercise(2, 40_000);
}

#[test]
fn reads_match_mtools_at_4_kb_clusters() {
    exercise(8, 300_000);
}

#[test]
fn reads_match_mtools_at_32_kb_clusters() {
    exercise(64, 2_200_000);
}

/// A card error mid-read: nothing of the call counts, so the fallback
/// (FatFs's own read from the same position) reads exactly the bytes the
/// failed call would have.
#[test]
fn a_card_error_leaves_the_position_for_the_fallback() {
    let Some((img, dir)) = image(1, 20_000) else { return };
    let want = body(9, 9000);
    put(&img, &dir.0, "NES_Card_Error_Test_01.gif", &want);
    let mut f = RawFile::new(img.extents("NES_Card_Error_Test_01.gif"));
    let mut d = dev(&img);
    let mut buf = vec![0u8; 700];
    assert_eq!(f.read(&mut buf, &mut d).unwrap(), 700);
    // The next read needs a partial sector then whole ones: fail its
    // second card call.
    d.fail_at = Some(d.calls + 2);
    let mut big = vec![0u8; 3000];
    assert!(f.read(&mut big, &mut d).is_err());
    assert_eq!(f.pos(), 700, "the failed read moved the position");
    // The fallback's view: the same bytes from 700.
    d.fail_at = None;
    assert_eq!(f.read(&mut big, &mut d).unwrap(), 3000);
    assert_eq!(&big[..], &want[700..3700]);
}

/// A different sector size is not ours: the image helper insists on 512,
/// and a cluster size of 0 (a damaged boot sector) is refused, not divided by.
#[test]
fn only_512_byte_sectors_and_real_clusters() {
    let geo = Geometry { csize: 0, fatbase: 32, database: 100, n_fatent: 100 };
    assert!(chain(&geo, 2, 10, &mut |_, _| Ok(())).is_err());
}

/// Why a writer must find the file let go first: the player's extent map
/// of a file deleted and written over reads the new file's bytes. With the
/// file in the `OpenSet`, the writer refuses instead.
#[test]
fn a_stale_map_reads_another_file_so_writers_check_the_open_set() {
    let Some((img, dir)) = image(1, 20_000) else { return };
    let old = body(1, 4000);
    put(&img, &dir.0, "NES_Old_01.gif", &old);
    let ext = img.extents("NES_Old_01.gif");
    let mut open = OpenSet::<6>::default();
    let slot = open.insert("/gifs/Consoles/NES_Old_01.gif").unwrap();

    // The writer's check (`fatidx::release_for_write` on the device): held,
    // so it asks the player to let go, and refuses while it has not.
    let may_write = |o: &OpenSet<6>, p: &str| !o.holds(p);
    assert!(!may_write(&open, "/gifs/consoles/nes_old_01.GIF"), "a rename/delete/move of the playing file must wait");
    assert!(may_write(&open, "/gifs/Consoles/NES_Other_01.gif"));

    // What would happen without it: delete, and a new file takes the clusters.
    del(&img, "NES_Old_01.gif");
    rewind_allocation(&img);
    let new = body(0xA5A5_5A5A, 4000);
    put(&img, &dir.0, "NES_New_01.gif", &new);
    let mut f = RawFile::new(ext);
    let mut buf = vec![0u8; 4000];
    let n = f.read(&mut buf, &mut dev(&img)).unwrap();
    assert!(buf[..n] != old[..], "the stale map still read the old bytes (the clusters were not reused)");
    assert!(buf[..n] == new[..], "it reads the file written over it");

    // Once the player let go, the writer goes ahead.
    open.remove(slot);
    assert!(may_write(&open, "/gifs/Consoles/NES_Old_01.gif"));
}

mod tempdir {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new() -> Dir {
            let p = std::env::temp_dir().join(format!(
                "fatread-test-{}-{}-{}",
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
