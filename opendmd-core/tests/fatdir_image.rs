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
    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new() -> Dir {
            let p = std::env::temp_dir().join(format!(
                "fatdir-test-{}-{}",
                std::process::id(),
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
    let short = |n: &str| n == "FI0000.GIF" || n == "fi0001.gif";
    let live: Vec<&String> = all.iter().filter(|n| !gone.contains(n) && !short(n)).collect();
    assert_eq!(found.len(), live.len(), "every live long-named file indexed once");
    assert!(!found.contains_key("FI0000.GIF") && !found.contains_key("fi0001.gif"));
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
