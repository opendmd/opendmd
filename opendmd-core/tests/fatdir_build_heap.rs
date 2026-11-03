//! The FAT index's build over a real FAT32 image (`mkfs.fat`, mtools: the
//! test says so and passes without them) shaped like a card: `/gifs/<folder>`
//! and the copies' buckets `/cache/<category>/<xx>/`. The build
//! (`fatdir::build_index` over `fatdir::CardWalker`) must allocate nothing,
//! however many folders and buckets the card has: on the rig (~10,000 GIFs,
//! ~5,000 buckets) a list of the buckets was a 64 KB allocation that failed
//! and aborted the board. Every file with a long name gets its record; the
//! folders' names are the ones FatFs lists (long, or 8.3 with NT case).

use opendmd_core::fatdir::{self, Geometry, SS};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Command;

/// Counts this thread's allocations while `COUNTING` is set.
struct Counting;
thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = COUNTING.try_with(|c| {
            if c.get() {
                ALLOCS.with(|a| a.set(a.get() + 1));
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static A: Counting = Counting;

const MKFS: &str = "/sbin/mkfs.fat";

fn run(cmd: &mut Command) {
    let out = cmd.output().expect("spawn");
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn fnv(b: &[u8]) -> u32 {
    b.iter().fold(0x811c_9dc5u32, |h, &c| (h ^ c as u32).wrapping_mul(0x0100_0193))
}

#[test]
fn a_card_shaped_build_allocates_nothing_and_indexes_every_long_name() {
    if !Path::new(MKFS).exists() || Command::new("mcopy").arg("--version").output().is_err() {
        eprintln!("skipping: {MKFS} or mtools missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("fatdir-build-heap-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let src = dir.join("tree");
    // Folders: a mixed-case long name, an 8.3 upper-case one, one with a
    // space; and the copies' buckets as FatFs names them (`1b`).
    let folders = ["Consoles", "ARCADE", "Pixel Art", "A_Much_Longer_Folder_Name_Here"];
    let mut want_files = BTreeSet::new();
    let mut want_dirs = BTreeSet::new();
    for (i, f) in folders.iter().enumerate() {
        fs::create_dir_all(src.join("gifs").join(f)).unwrap();
        want_dirs.insert(format!("/gifs/{f}"));
        for j in 0..(60 + 90 * i) {
            let name = format!("{}_Title_Number_{j:04}.gif", f.replace(' ', "_"));
            fs::write(src.join("gifs").join(f).join(&name), b"GIF89a").unwrap();
            want_files.insert(format!("/gifs/{f}/{name}"));
            let bucket = format!("{:02x}", fnv(name.as_bytes()) & 0xff);
            let b = src.join("cache").join(f).join(&bucket);
            fs::create_dir_all(&b).unwrap();
            fs::write(b.join(&name), b"GIF89a").unwrap();
            want_files.insert(format!("/cache/{f}/{bucket}/{name}"));
            want_dirs.insert(format!("/cache/{f}/{bucket}"));
        }
    }
    // The cache's own files and hidden folders are not buckets.
    fs::create_dir_all(src.join("cache/.tmp/zz")).unwrap();
    fs::write(src.join("cache/.names"), b"x").unwrap();
    let img = dir.join("card.img");
    run(Command::new(MKFS).args(["-C", "-F", "32", "-S", "512", "-s", "8"]).arg(&img).arg("200000"));
    run(Command::new("mcopy").env("MTOOLS_SKIP_CHECK", "1").arg("-s").arg("-Q").arg("-i").arg(&img).arg(src.join("gifs")).arg(src.join("cache")).arg("::"));

    let mut f = fs::File::open(&img).unwrap();
    let mut b = [0u8; SS];
    f.read_exact(&mut b).unwrap();
    let u16_ = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u32;
    let u32_ = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let csize = b[13] as u32;
    let fatbase = u16_(14);
    let database = fatbase + b[16] as u32 * u32_(36);
    let geo = Geometry { csize, fatbase, database, n_fatent: (u32_(32) - database) / csize + 2 };
    let root = u32_(44);
    let mut read = |lba: u32, buf: &mut [u8]| -> std::io::Result<()> {
        f.seek(SeekFrom::Start(lba as u64 * SS as u64))?;
        f.read_exact(buf)
    };
    // `/gifs` and `/cache` from the root (the device: `f_opendir`).
    let mut roots = (None, None);
    let mut buf = vec![0u8; SS];
    fatdir::walk_dirs(&geo, root, &mut read, &mut buf, &mut |d| {
        match d.name.to_ascii_lowercase().as_slice() {
            b"gifs" => roots.0 = Some(d.cluster),
            b"cache" => roots.1 = Some(d.cluster),
            _ => {}
        }
        Ok(())
    })
    .unwrap();
    let mut bufs = vec![0u8; fatdir::INDEXED_DIR_BUFS];
    let mut got_dirs = BTreeSet::new();
    fatdir::each_indexed_dir(&geo, roots.0, roots.1, &mut read, &mut bufs, &mut |d, _| {
        assert_eq!(d.gif, d.path.starts_with("/gifs/"));
        got_dirs.insert(d.path.to_string());
        Ok(())
    })
    .unwrap();
    // mtools stores `1b` as an 8.3 name with NT case: compare as FatFs would.
    let low = |s: &BTreeSet<String>| s.iter().map(|p| p.to_ascii_lowercase()).collect::<BTreeSet<_>>();
    assert_eq!(low(&got_dirs), low(&want_dirs));
    assert!(got_dirs.contains("/gifs/Consoles") && got_dirs.contains("/gifs/ARCADE"), "{got_dirs:?}");

    // The build, its output hashed rather than kept: no allocation in it.
    let mut scratch = vec![0u8; 8 * SS];
    let mut group = vec![0u8; 4096];
    let mut between = || Ok(());
    let mut out = Vec::with_capacity(RECS_MAX);
    let mut w = fatdir::CardWalker {
        geo,
        gifs: roots.0,
        cache: roots.1,
        read: &mut read,
        bufs: &mut bufs,
        scratch: &mut scratch,
        between: &mut between,
    };
    let mut put = |b: &[u8]| -> std::io::Result<()> {
        assert!(out.len() + b.len() <= out.capacity(), "the test's buffer");
        out.extend_from_slice(b);
        Ok(())
    };
    COUNTING.with(|c| c.set(true));
    let (count, walks) = fatdir::build_index(&mut put, &geo, &mut w, &mut group).unwrap();
    COUNTING.with(|c| c.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0, "the build allocated");
    // (Long names that start in the previous cluster have no record: they
    // open by path, ~1% here with 4 KB clusters.)
    assert!(count as usize <= want_files.len() && count as usize * 100 >= want_files.len() * 97, "{count} of {}", want_files.len());
    assert!(walks > 3, "{walks} walks: the group was meant to take several");
    let h = fatdir::Header::decode(&out).unwrap();
    assert_eq!(h.dirs as usize, want_dirs.len());
    let fan = |b: usize| u32::from_le_bytes(out[fatdir::HEADER + b * 4..][..4].try_into().unwrap());
    let missing = want_files
        .iter()
        .filter(|p| {
            let k = fatdir::hash(p.as_bytes());
            let bk = (k >> 24) as usize;
            !(fan(bk)..fan(bk + 1)).any(|r| out[fatdir::RECORDS_AT as usize + r as usize * 8..][..4] == k.to_le_bytes())
        })
        .count();
    assert_eq!(missing, want_files.len() - count as usize, "every record is one of the card's files");
    eprintln!("{count} records, {} directories, {walks} walks, no allocation", h.dirs);
    let _ = fs::remove_dir_all(&dir);
}

const RECS_MAX: usize = 1 << 20;
