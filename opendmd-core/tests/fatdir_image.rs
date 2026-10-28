//! `fatdir` against real FAT32 images made by `mkfs.fat` and filled by
//! mtools (both must be installed: the test says so and passes without them).
//! Covers long and 8.3 names (NT case bits), deleted entries and their reuse,
//! a rename, multi-cluster files, and long names that start in the previous
//! cluster (512 B clusters make that common).

use opendmd_core::fatdir::{self, Found, Geometry, Verify, SS};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
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
    f: fs::File,
    geo: Geometry,
    root: u32,
}

impl Img {
    fn open(p: &Path) -> Img {
        let mut f = fs::File::open(p).unwrap();
        let mut b = [0u8; SS];
        f.read_exact(&mut b).unwrap();
        let u16_ = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u32;
        let u32_ = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!(u16_(11), 512);
        let csize = b[13] as u32;
        let fatbase = u16_(14);
        let database = fatbase + b[16] as u32 * u32_(36);
        let n_fatent = (u32_(32) - database) / csize + 2;
        Img { f, geo: Geometry { csize, fatbase, database, n_fatent }, root: u32_(44) }
    }

    fn read(&mut self, lba: u32, buf: &mut [u8]) -> std::io::Result<()> {
        self.f.seek(SeekFrom::Start(lba as u64 * SS as u64))?;
        self.f.read_exact(buf)
    }

    /// A subdirectory's first cluster, by its 8.3 name (upper case).
    fn subdir(&mut self, dir: u32, sfn: &str) -> u32 {
        let mut want = [b' '; 11];
        want[..sfn.len()].copy_from_slice(sfn.as_bytes());
        let mut c = Some(dir);
        let mut buf = [0u8; SS];
        while let Some(cl) = c {
            for s in 0..self.geo.csize {
                let lba = self.geo.database + (cl - 2) * self.geo.csize + s;
                self.read(lba, &mut buf).unwrap();
                for e in buf.as_chunks::<32>().0 {
                    if e[11] & 0x10 != 0 && e[11] != 0x0F && e[..11] == want {
                        return (u16::from_le_bytes([e[20], e[21]]) as u32) << 16
                            | u16::from_le_bytes([e[26], e[27]]) as u32;
                    }
                }
            }
            let geo = self.geo;
            c = fatdir::next_cluster(&geo, cl, &mut |l, b| self.read(l, b), &mut [0u8; SS]).unwrap();
        }
        panic!("no {sfn}");
    }

    /// name -> (loc, prev_rel)
    fn walk(&mut self, dir: u32) -> HashMap<String, (u32, Option<u32>)> {
        let geo = self.geo;
        let mut out = HashMap::new();
        let mut scratch = vec![0u8; SS * 8];
        fatdir::walk(&geo, dir, &mut |l, b| self.read(l, b), &mut scratch, &mut |f: Found| {
            let name = String::from_utf8(f.name.to_vec()).unwrap();
            assert!(out.insert(name, (f.loc, f.prev_rel)).is_none());
            Ok(())
        })
        .unwrap();
        out
    }

    fn verify(&mut self, loc: u32, prev_rel: Option<u32>, name: &str) -> Verify {
        let (rel, slot) = (loc >> 4, (loc & 15) as usize);
        let mut cur = [0u8; SS];
        self.read(self.geo.database + rel, &mut cur).unwrap();
        match fatdir::verify(&cur, None, slot, name.as_bytes()) {
            Verify::NeedPrev => {
                // Across a cluster boundary only an extra record knows the
                // sector before; without one the device opens by path.
                let p = if !rel.is_multiple_of(self.geo.csize) {
                    rel - 1
                } else {
                    match prev_rel {
                        Some(p) => p,
                        None => return Verify::NeedPrev,
                    }
                };
                let mut prev = [0u8; SS];
                self.read(self.geo.database + p, &mut prev).unwrap();
                fatdir::verify(&cur, Some(&prev), slot, name.as_bytes())
            }
            v => v,
        }
    }

    fn content(&mut self, mut cluster: u32, size: u32) -> Vec<u8> {
        let geo = self.geo;
        let mut v = Vec::new();
        let mut buf = vec![0u8; SS * geo.csize as usize];
        while v.len() < size as usize {
            self.read(geo.database + (cluster - 2) * geo.csize, &mut buf).unwrap();
            v.extend_from_slice(&buf);
            match fatdir::next_cluster(&geo, cluster, &mut |l, b| self.read(l, b), &mut [0u8; SS]).unwrap() {
                Some(c) => cluster = c,
                None => break,
            }
        }
        v.truncate(size as usize);
        v
    }
}

fn names(n: usize) -> Vec<String> {
    let words = ["Super", "Mega", "Turbo", "Final", "Fight", "Street", "Hero", "Quest", "Dragon", "Ninja"];
    let mut v = vec![
        "FI0000.GIF".to_string(),
        "fi0001.gif".to_string(),
        "MD_Dr.gif".to_string(),
        "Abcdefgh1.gif".to_string(), // 13 characters: one LFN entry, no terminator
        format!("{}.gif", "Y".repeat(22)), // 26: two full entries
        format!("Long_{}.gif", "x".repeat(150)),
        "Caf\u{e9}_Non_ASCII_Name.gif".to_string(), // not indexed: opens by path
    ];
    let mut i = 0;
    while v.len() < n {
        let mut s = format!("Fx{i:04}_");
        let len = 9 + (i * 7919) % 50;
        while s.len() < len {
            s.push_str(words[(i + s.len()) % words.len()]);
            s.push('_');
        }
        s.truncate(len);
        v.push(format!("{s}.gif"));
        i += 1;
    }
    v
}

fn body(name: &str) -> Vec<u8> {
    // Unique bytes; a few multi-cluster sizes.
    let n = 40 + (fatdir::hash(name.as_bytes()) % 3000) as usize + if name.contains("_3") { 9000 } else { 0 };
    (0..n).map(|i| (i as u32 ^ fatdir::hash(name.as_bytes())) as u8).collect()
}

fn image(cluster_sectors: u32, kb: u32) -> Option<(PathBuf, tempdir::Dir)> {
    if !tools() {
        eprintln!("skipping: {MKFS} or mtools missing");
        return None;
    }
    let dir = tempdir::Dir::new();
    let img = dir.0.join("card.img");
    run(Command::new(MKFS).args(["-C", "-F", "32", "-S", "512", "-s", &cluster_sectors.to_string()]).arg(&img).arg(kb.to_string()));
    Some((img, dir))
}

mod tempdir {
    use std::sync::atomic::{AtomicU64, Ordering};

    // Process-wide counter: tests run in parallel threads, and pid + nanos can collide.
    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new() -> Dir {
            let p = std::env::temp_dir().join(format!(
                "fatdir-test-{}-{}-{}",
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

fn mcopy(img: &Path, src: &Path, names: &[String], to: &str) {
    for chunk in names.chunks(200) {
        let mut c = Command::new("mcopy");
        c.env("LC_ALL", "C.UTF-8");
        c.arg("-i").arg(img).arg("-n").arg("-o");
        for n in chunk {
            c.arg(src.join(n));
        }
        c.arg(to);
        run(&mut c);
    }
}

fn exercise(cluster_sectors: u32, kb: u32, count: usize) {
    let Some((img, dir)) = image(cluster_sectors, kb) else { return };
    let src = dir.0.join("src");
    fs::create_dir_all(&src).unwrap();
    let all = names(count);
    for n in &all {
        fs::write(src.join(n), body(n)).unwrap();
    }
    run(Command::new("mmd").arg("-i").arg(&img).args(["::/gifs", "::/gifs/Big"]));
    let (first, later) = all.split_at(count * 3 / 4);
    mcopy(&img, &src, first, "::/gifs/Big/");
    // Delete every 5th, so later files reuse the slots.
    let gone: Vec<&String> = first.iter().step_by(5).skip(1).collect();
    for chunk in gone.chunks(100) {
        let mut c = Command::new("mdel");
        c.arg("-i").arg(&img);
        for n in chunk {
            c.arg(format!("::/gifs/Big/{n}"));
        }
        run(&mut c);
    }
    mcopy(&img, &src, later, "::/gifs/Big/");

    let mut im = Img::open(&img);
    let gifs = im.subdir(im.root, "GIFS");
    let big = im.subdir(gifs, "BIG");
    let found = im.walk(big);
    // Names that fit 8.3 exactly have no long name and are not indexed
    // (they open by path).
    let short = |n: &str| n == "FI0000.GIF" || n == "fi0001.gif" || !n.is_ascii();
    let live: Vec<&String> = all.iter().filter(|n| !gone.contains(n) && !short(n)).collect();
    assert_eq!(found.len(), live.len(), "every live long-named file indexed once");
    assert!(!found.contains_key("FI0000.GIF") && !found.contains_key("fi0001.gif"));
    assert!(found.keys().all(|k| k.is_ascii()), "a non-ASCII long name is not indexed");
    let mut extras = 0;
    for n in &live {
        let &(loc, prev) = found.get(n.as_str()).unwrap_or_else(|| panic!("{n} not found"));
        extras += prev.is_some() as usize;
        match im.verify(loc, prev, n) {
            Verify::Ok { cluster, size, .. } => {
                let want = body(n);
                assert_eq!(size as usize, want.len(), "{n}");
                assert_eq!(im.content(cluster, size), want, "{n}: contents");
            }
            v => panic!("{n}: {v:?}"),
        }
        // The same name in another ASCII case is the same file to FatFs.
        assert!(matches!(im.verify(loc, prev, &n.to_ascii_uppercase()), Verify::Ok { .. }), "{n}");
        // Another name at the same place is never accepted.
        assert!(!matches!(im.verify(loc, prev, &format!("{n}x")), Verify::Ok { .. }));
    }
    if cluster_sectors == 1 {
        assert!(extras > 0, "512 B clusters must produce records needing the previous cluster");
    }

    // Now change the card behind the index: delete one, rename one.
    let (del, ren, keep) = (live[7].clone(), live[11].clone(), live[13].clone());
    run(Command::new("mdel").arg("-i").arg(&img).arg(format!("::/gifs/Big/{del}")));
    run(Command::new("mren").arg("-i").arg(&img).arg(format!("::/gifs/Big/{ren}")).arg("::/gifs/Big/Renamed_File_Name.gif"));
    let mut im = Img::open(&img);
    for (n, ok) in [(&del, false), (&ren, false), (&keep, true)] {
        let &(loc, prev) = found.get(n.as_str()).unwrap();
        let v = im.verify(loc, prev, n);
        assert_eq!(matches!(v, Verify::Ok { .. }), ok, "{n}: {v:?}");
    }
    let again = im.walk(big);
    assert!(again.contains_key("Renamed_File_Name.gif") && !again.contains_key(ren.as_str()));

    // The same name written again (an upload replacing a GIF): the index's
    // record, wherever the entry now is, must give the new contents or none.
    let over = live[17].clone();
    fs::write(src.join(&over), b"GIF89a replaced contents").unwrap();
    mcopy(&img, &src, std::slice::from_ref(&over), "::/gifs/Big/");
    let mut im = Img::open(&img);
    let &(loc, prev) = found.get(over.as_str()).unwrap();
    if let Verify::Ok { cluster, size, .. } = im.verify(loc, prev, &over) {
        assert_eq!(im.content(cluster, size), b"GIF89a replaced contents");
    }
    let fresh = im.walk(big);
    let &(loc, prev) = fresh.get(over.as_str()).unwrap();
    let Verify::Ok { cluster, size, .. } = im.verify(loc, prev, &over) else { panic!("{over}") };
    assert_eq!(im.content(cluster, size), b"GIF89a replaced contents");

    // A directory renamed on a PC and a new one made under the old name:
    // the old records still verify (the entries did not move), so only the
    // index's directory list can tell. It does: the cluster changed.
    let indexed = vec![(fatdir::hash(b"/gifs/Big"), big)];
    run(Command::new("mren").arg("-i").arg(&img).args(["::/gifs/Big", "::/gifs/Old"]));
    run(Command::new("mmd").arg("-i").arg(&img).arg("::/gifs/Big"));
    mcopy(&img, &src, std::slice::from_ref(&keep), "::/gifs/Big/");
    let mut im = Img::open(&img);
    let &(loc, prev) = found.get(keep.as_str()).unwrap();
    assert!(matches!(im.verify(loc, prev, &keep), Verify::Ok { .. }), "the stale record still verifies");
    let new_big = im.subdir(gifs, "BIG");
    assert_ne!(new_big, big);
    assert!(!fatdir::dirs_match(&indexed, &[(fatdir::hash(b"/gifs/Big"), new_big)]));
    eprintln!("{} files, {} extra records, {} sectors per cluster", live.len(), extras, cluster_sectors);
}

#[test]
fn small_clusters_long_names_across_cluster_boundaries() {
    exercise(1, 40_000, 400);
}

#[test]
fn larger_clusters_and_a_bigger_directory() {
    exercise(8, 300_000, 1200);
}

/// The card identity's volume half (`card::volume`, read from the boot
/// sector on the device): what `mkfs.fat` wrote, for FAT32 and FAT16; files
/// copied on leave it alone; a format makes another.
#[test]
fn card_identity_from_the_boot_sector() {
    use opendmd_core::card;
    if !tools() {
        eprintln!("skipping: {MKFS} or mtools missing");
        return;
    }
    let dir = tempdir::Dir::new();
    let boot = |img: &Path| {
        let mut b = [0u8; SS];
        fs::File::open(img).unwrap().read_exact(&mut b).unwrap();
        card::volume(&b)
    };
    let img = dir.0.join("card.img");
    let mkfs = |fat: &str, serial: &str, label: Option<&str>, kb: &str| {
        let _ = fs::remove_file(&img);
        let mut c = Command::new(MKFS);
        c.args(["-C", "-F", fat, "-S", "512", "-i", serial]);
        if let Some(l) = label {
            c.args(["-n", l]);
        }
        run(c.arg(&img).arg(kb));
    };

    mkfs("32", "DEADBEEF", Some("OPENDMD"), "40000");
    let v = boot(&img).expect("a FAT32 boot sector");
    assert_eq!((v.serial, v.label().as_str()), (0xDEAD_BEEF, "OPENDMD"));
    let id = card::identity(0x1234, Some(&v), 40_000 * 1024);

    // Files copied on (a computer, the API): the same card.
    let src = dir.0.join("src");
    fs::create_dir_all(&src).unwrap();
    let names = names(20);
    for n in &names {
        fs::write(src.join(n), body(n)).unwrap();
    }
    run(Command::new("mmd")
        .arg("-i")
        .arg(&img)
        .args(["::/gifs", "::/gifs/Consoles"]));
    mcopy(&img, &src, &names, "::/gifs/Consoles/");
    assert_eq!(
        card::identity(0x1234, boot(&img).as_ref(), 40_000 * 1024),
        id
    );

    // Formatted again (another serial, as any formatter picks): another card.
    mkfs("32", "DEADBEF0", Some("OPENDMD"), "40000");
    assert_ne!(
        card::identity(0x1234, boot(&img).as_ref(), 40_000 * 1024),
        id
    );

    // No label given: the boot sector's "NO NAME".
    mkfs("32", "00000001", None, "40000");
    assert_eq!(boot(&img).unwrap().label(), "");

    // FAT16 (a small card): the record sits elsewhere.
    mkfs("16", "CAFEF00D", Some("SMALL"), "20000");
    let v = boot(&img).expect("a FAT16 boot sector");
    assert_eq!((v.serial, v.label().as_str()), (0xCAFE_F00D, "SMALL"));
}
