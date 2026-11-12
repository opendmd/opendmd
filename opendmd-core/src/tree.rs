//! A directory removed with everything under it, deepest first, in bounded
//! batches: `DELETE /api/files/<dir>?recursive=1` (firmware
//! `files::remove_tree`, under `cache/` and the retired `odm/` tree).
//!
//! A directory's names are read before any of them is removed (FatFs must
//! not be iterating a directory while its entries go), but at most
//! `batch` of them at a time: read up to `batch`, remove those, read the
//! directory again from its start, until it is empty. A folder of the
//! retired tree can hold thousands of entries (the stock card's Consoles
//! has ~3,800): all of them in one `Vec<(String, bool)>` was ~250 KB, an
//! allocation the display's heap cannot make (an abort, not an error).
//! Each batch's room is taken fallibly too: no room is an
//! [`io::ErrorKind::OutOfMemory`] error, never an abort.

use std::io;

/// What the removal needs of a filesystem (`StdFs` for real files).
pub trait TreeFs {
    /// Up to `max` of `dir`'s entries (name, is a directory), in the
    /// filesystem's order, pushed into `out` (empty, with room for `max`).
    fn list(&mut self, dir: &str, max: usize, out: &mut Vec<(String, bool)>) -> io::Result<()>;
    fn remove_file(&mut self, path: &str) -> io::Result<()>;
    fn remove_dir(&mut self, path: &str) -> io::Result<()>;
}

/// The filesystem `std::fs` gives (on the display, ESP-IDF's VFS over FatFs).
pub struct StdFs;

impl TreeFs for StdFs {
    fn list(&mut self, dir: &str, max: usize, out: &mut Vec<(String, bool)>) -> io::Result<()> {
        // (The listing is dropped, the directory closed, before anything in
        // it is removed: the caller removes only after this returns.)
        for entry in std::fs::read_dir(dir)? {
            if out.len() >= max {
                break;
            }
            let entry = entry?;
            // `d_type`, not a `stat`: a stat re-scans the directory on FAT.
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            out.push((entry.file_name().to_string_lossy().into_owned(), is_dir));
        }
        Ok(())
    }
    fn remove_file(&mut self, path: &str) -> io::Result<()> {
        std::fs::remove_file(path)
    }
    fn remove_dir(&mut self, path: &str) -> io::Result<()> {
        std::fs::remove_dir(path)
    }
}

/// How deep below `dir` the removal goes; a tree deeper than this is left
/// (`done` false) rather than recursed into.
pub const MAX_DEPTH: u32 = 8;

/// Remove `dir` and everything under it, `batch` names at a time:
/// `(entries removed, done)`. `over()` is asked before each removal: true
/// stops there (`done` false, the caller asks again later). `between()`
/// runs after each removal (the display yields there, so the player's reads
/// get the FAT lock). `NotFound` if `dir` is not there.
pub fn remove_tree(
    fs: &mut impl TreeFs,
    dir: &str,
    batch: usize,
    over: &mut impl FnMut() -> bool,
    between: &mut impl FnMut(),
) -> io::Result<(u32, bool)> {
    let mut removed = 0;
    let done = go(fs, dir, batch.max(1), over, between, &mut removed, 0)?;
    Ok((removed, done))
}

fn go(
    fs: &mut impl TreeFs,
    dir: &str,
    batch: usize,
    over: &mut impl FnMut() -> bool,
    between: &mut impl FnMut(),
    removed: &mut u32,
    depth: u32,
) -> io::Result<bool> {
    let mut names: Vec<(String, bool)> = Vec::new();
    names.try_reserve_exact(batch).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "no heap to list the directory"))?;
    loop {
        names.clear();
        fs.list(dir, batch, &mut names)?;
        if names.is_empty() {
            break;
        }
        for (name, is_dir) in names.drain(..) {
            if over() {
                return Ok(false);
            }
            let p = format!("{dir}/{name}");
            if is_dir {
                if depth >= MAX_DEPTH || !go(fs, &p, batch, over, between, removed, depth + 1)? {
                    return Ok(false);
                }
            } else {
                fs.remove_file(&p)?;
                *removed += 1;
            }
            between();
        }
    }
    fs.remove_dir(dir)?;
    *removed += 1;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Paths -> is a directory; counts the largest listing asked and
    /// whether anything was removed from a directory while it was listed.
    #[derive(Default)]
    struct Mem {
        tree: BTreeMap<String, bool>,
        largest: usize,
        lists: u32,
    }

    impl Mem {
        fn add(&mut self, p: &str, is_dir: bool) {
            self.tree.insert(p.to_string(), is_dir);
        }
        fn children(&self, dir: &str) -> Vec<(String, bool)> {
            let pre = format!("{dir}/");
            self.tree
                .iter()
                .filter_map(|(p, d)| p.strip_prefix(&pre).filter(|r| !r.contains('/')).map(|r| (r.to_string(), *d)))
                .collect()
        }
    }

    impl TreeFs for Mem {
        fn list(&mut self, dir: &str, max: usize, out: &mut Vec<(String, bool)>) -> io::Result<()> {
            if self.tree.get(dir) != Some(&true) {
                return Err(io::ErrorKind::NotFound.into());
            }
            self.lists += 1;
            out.extend(self.children(dir).into_iter().take(max));
            self.largest = self.largest.max(out.len());
            Ok(())
        }
        fn remove_file(&mut self, p: &str) -> io::Result<()> {
            match self.tree.remove(p) {
                Some(false) => Ok(()),
                _ => Err(io::ErrorKind::NotFound.into()),
            }
        }
        fn remove_dir(&mut self, p: &str) -> io::Result<()> {
            if !self.children(p).is_empty() {
                return Err(io::Error::other("directory not empty"));
            }
            match self.tree.remove(p) {
                Some(true) => Ok(()),
                _ => Err(io::ErrorKind::NotFound.into()),
            }
        }
    }

    /// `/odm` with a folder of `n` files and a nested bucket of 5.
    fn odm(n: usize) -> Mem {
        let mut m = Mem::default();
        m.add("/sd/odm", true);
        m.add("/sd/odm/Consoles", true);
        for i in 0..n {
            m.add(&format!("/sd/odm/Consoles/GAME_{i:04}.odm"), false);
        }
        m.add("/sd/odm/Consoles/ab", true);
        for i in 0..5 {
            m.add(&format!("/sd/odm/Consoles/ab/x{i}.odm"), false);
        }
        m.add("/sd/keep.txt", false);
        m
    }

    #[test]
    fn a_folder_of_thousands_goes_in_batches_never_listed_whole() {
        let mut m = odm(3_800);
        let (removed, done) = remove_tree(&mut m, "/sd/odm", 64, &mut || false, &mut || {}).unwrap();
        assert!(done);
        // 3,800 + 5 files, the bucket, Consoles and odm.
        assert_eq!(removed, 3_800 + 5 + 3);
        assert_eq!(m.largest, 64, "no listing past the batch");
        assert!(m.lists > 3_800 / 64, "read again batch by batch ({} listings)", m.lists);
        assert_eq!(m.tree.keys().collect::<Vec<_>>(), ["/sd/keep.txt"], "only what was outside it is left");
    }

    #[test]
    fn a_spent_budget_stops_and_the_next_call_finishes() {
        let mut m = odm(500);
        let mut left = 200u32;
        let (first, done) = remove_tree(&mut m, "/sd/odm", 32, &mut || { left = left.saturating_sub(1); left == 0 }, &mut || {}).unwrap();
        assert!(!done);
        // (One ask for Consoles itself, then 198 files, then the 200th stops.)
        assert_eq!(first, 198);
        let (second, done) = remove_tree(&mut m, "/sd/odm", 32, &mut || false, &mut || {}).unwrap();
        assert!(done);
        assert_eq!(first + second, 500 + 5 + 3);
        assert!(!m.tree.contains_key("/sd/odm"));
    }

    #[test]
    fn a_missing_directory_is_not_found_and_a_batch_of_zero_still_works() {
        let mut m = odm(3);
        let e = remove_tree(&mut m, "/sd/nothing", 8, &mut || false, &mut || {}).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        let (removed, done) = remove_tree(&mut m, "/sd/odm", 0, &mut || false, &mut || {}).unwrap();
        assert!(done && removed == 3 + 5 + 3 && m.largest == 1);
    }

    #[test]
    fn deeper_than_the_cap_is_left_not_done() {
        let mut m = Mem::default();
        let mut p = String::from("/sd/odm");
        m.add(&p, true);
        for i in 0..=MAX_DEPTH + 1 {
            p = format!("{p}/d{i}");
            m.add(&p, true);
        }
        let (_, done) = remove_tree(&mut m, "/sd/odm", 8, &mut || false, &mut || {}).unwrap();
        assert!(!done);
        assert!(m.tree.contains_key("/sd/odm"));
    }

    /// The same through real files (StdFs), a batch smaller than the folder.
    #[test]
    fn real_files_through_std_fs() {
        let root = std::env::temp_dir().join(format!("opendmd-tree-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&root);
        let odm = root.join("odm");
        std::fs::create_dir_all(odm.join("Consoles/ab")).unwrap();
        for i in 0..150 {
            std::fs::write(odm.join(format!("Consoles/G_{i:03}.odm")), b"x").unwrap();
        }
        std::fs::write(odm.join("Consoles/ab/y.odm"), b"y").unwrap();
        std::fs::write(root.join("keep.txt"), b"k").unwrap();
        let mut yields = 0;
        let (removed, done) = remove_tree(&mut StdFs, odm.to_str().unwrap(), 16, &mut || false, &mut || yields += 1).unwrap();
        assert!(done);
        assert_eq!(removed, 150 + 1 + 3);
        // After each entry: 150 files and the bucket in Consoles, y.odm in it, Consoles in odm.
        assert_eq!(yields, 150 + 1 + 1 + 1);
        assert!(!odm.exists() && root.join("keep.txt").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
