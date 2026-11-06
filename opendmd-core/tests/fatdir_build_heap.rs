//! The FAT index's build over a real FAT32 image (`mkfs.fat`, mtools: the
//! test says so and passes without them) shaped like a card: `/gifs/<folder>`
//! and the copies' buckets `/cache/<category>/<xx>/`. The build
//! (`fatdir::Cursor`, `walk`, `gather`, `tmp_dirs`, as the device runs it) must allocate nothing,
//! however many folders and buckets the card has: on a test panel (~10,000 GIFs,
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
    // The directories, as the device's cursor gives them (stopped and
    // started again every 7, as the device's slices are).
    let mut cursor = Box::new(fatdir::Cursor::new(roots.0, roots.1));
    let mut got_dirs = BTreeSet::new();
    while let Some(d) = cursor.next(&geo, &mut read).unwrap() {
        let path = if d.gif {
            format!("/gifs/{}", std::str::from_utf8(&cursor.name[..d.name_len]).unwrap())
        } else {
            want_dirs.iter().find(|p| fatdir::hash(p.as_bytes()) == d.key).cloned().unwrap_or_else(|| format!("?{:08x}", d.key))
        };
        assert_eq!(fatdir::hash(path.as_bytes()), d.key, "{path}");
        got_dirs.insert(path);
    }
    // mtools stores `1b` as an 8.3 name with NT case: compare as FatFs would.
    let low = |s: &BTreeSet<String>| s.iter().map(|p| p.to_ascii_lowercase()).collect::<BTreeSet<_>>();
    assert_eq!(low(&got_dirs), low(&want_dirs));
    assert!(got_dirs.contains("/gifs/Consoles") && got_dirs.contains("/gifs/ARCADE"), "{got_dirs:?}");

    // The build as the device runs it: one walk into temporary records,
    // then passes over them with a 2 KB group, then the directories. Its
    // output kept in buffers taken before: no allocation in it.
    let mut cursor = Box::new(fatdir::Cursor::new(roots.0, roots.1));
    let mut scratch = vec![0u8; 4 * SS];
    let mut group = vec![0u8; 2048];
    let mut chunk = vec![0u8; 2048];
    let mut tmp: Vec<u8> = Vec::with_capacity(RECS_MAX);
    let mut out: Vec<u8> = Vec::with_capacity(RECS_MAX);
    let mut fanout = [0u32; fatdir::FANOUT];
    let mut set = fatdir::DirSet::default();
    COUNTING.with(|c| c.set(true));
    let mut k = 0;
    loop {
        k += 1;
        if k % 7 == 0 {
            cursor.refresh();
        }
        let Some(d) = cursor.next(&geo, &mut read).unwrap() else { break };
        set.add((d.key, d.first));
        assert!(tmp.len() + 16 <= tmp.capacity());
        tmp.extend_from_slice(&fatdir::TMP_DIR);
        tmp.extend_from_slice(&fatdir::tmp_record(d.key, d.first));
        let seed = fatdir::hash_more(d.key, b"/");
        fatdir::walk(&geo, d.first, &mut read, &mut scratch, &mut |f| {
            if f.prev_rel.is_none() {
                let h = fatdir::hash_more(seed, f.name);
                assert!(tmp.len() + 8 <= tmp.capacity());
                tmp.extend_from_slice(&fatdir::tmp_record(h, f.loc));
                fanout[(h >> 24) as usize + 1] += 1;
            }
            Ok(())
        })
        .unwrap();
    }
    for b in 0..256 {
        fanout[b + 1] += fanout[b];
    }
    let count = fanout[256];
    let h = fatdir::Header { count, n_fatent: geo.n_fatent, database: geo.database, dirs: set_n(&set) };
    out.extend_from_slice(&h.encode());
    for v in fanout {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let (mut b0, mut passes) = (0, 0);
    while b0 < 256 {
        let mut at = 0;
        let t = &tmp;
        let (b1, n) = fatdir::gather(
            &mut |buf: &mut [u8]| {
                let n = buf.len().min(t.len() - at);
                buf[..n].copy_from_slice(&t[at..at + n]);
                at += n;
                Ok(n)
            },
            &fanout,
            b0,
            &mut group,
            &mut chunk,
        )
        .unwrap();
        assert!(out.len() + n <= out.capacity());
        out.extend_from_slice(&group[..n]);
        b0 = b1;
        passes += 1;
    }
    let mut at = 0;
    let t = &tmp;
    let mut again = fatdir::DirSet::default();
    fatdir::tmp_dirs(
        &mut |buf: &mut [u8]| {
            let n = buf.len().min(t.len() - at);
            buf[..n].copy_from_slice(&t[at..at + n]);
            at += n;
            Ok(n)
        },
        &mut chunk,
        &mut |k, c| {
            again.add((k, c));
            out.extend_from_slice(&k.to_le_bytes());
            out.extend_from_slice(&c.to_le_bytes());
            Ok(())
        },
    )
    .unwrap();
    COUNTING.with(|c| c.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0, "the build allocated");
    assert_eq!(again, set);
    let walks = passes;
    // (Long names that start in the previous cluster have no record: they
    // open by path, ~1% here with 4 KB clusters.)
    assert!(count as usize <= want_files.len() && count as usize * 100 >= want_files.len() * 97, "{count} of {}", want_files.len());
    assert!(walks > 3, "{walks} passes: the group was meant to take several");
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
    assert_eq!(out.len(), fatdir::RECORDS_AT as usize + count as usize * 8 + want_dirs.len() * 8);
    eprintln!("{count} records, {} directories, one walk and {walks} passes of a 2 KB group, no allocation", h.dirs);
    let _ = fs::remove_dir_all(&dir);
}

/// A [`fatdir::DirSet`]'s count (by adding nothing: compare with sets).
fn set_n(s: &fatdir::DirSet) -> u32 {
    s.len()
}

const RECS_MAX: usize = 1 << 20;
