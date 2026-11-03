//! The card's names file (`/cache/.names`, written with the FAT index): the
//! file names of every `/gifs/<folder>`, read one folder at a time without
//! reading the rest (`GET /api/files?names=1`: naming a new GIF).
//!
//! Layout: each folder's names, a line each (`name\n`), folder after folder;
//! then a table of `(folder key, offset, length)` (little-endian `u32`s, 12
//! bytes a folder); then a 12-byte trailer: the table's offset, the number
//! of folders and [`MAGIC`]. A reader reads the trailer, the table, and the
//! one folder's bytes. A folder's key is [`dir_key`] (its path, lowercased:
//! FAT folders match without case); two folders with one key would mix
//! names, which only costs a create-only PUT's retry.

use crate::fatdir;
use std::io::{self, Read, Seek, SeekFrom, Write};

pub const MAGIC: &[u8; 4] = b"ODN2";
/// The longest name a reader takes (bytes): a FAT long name is at most 255
/// UTF-16 units, at most 765 bytes of UTF-8. A longer line means a damaged
/// file: the reader stops there rather than read on into its heap.
pub const NAME_MAX: usize = 768;
const TRAILER: u64 = 12;
/// More folders than any card has: a damaged trailer is refused.
const MAX_DIRS: u32 = 1 << 16;

/// A folder's key: `/gifs/Consoles` and `/gifs/consoles/` are one.
pub fn dir_key(dir: &str) -> u32 {
    fatdir::hash(dir.trim_end_matches('/').to_ascii_lowercase().as_bytes())
}

/// Writes a names file: [`begin`](Self::begin) a folder, its
/// [`name`](Self::name)s, the next folder, ..., then [`finish`](Self::finish).
pub struct Writer<W: Write> {
    w: W,
    at: u32,
    table: Vec<(u32, u32, u32)>,
}

impl<W: Write> Writer<W> {
    pub fn new(w: W) -> Self {
        Writer { w, at: 0, table: Vec::new() }
    }

    pub fn begin(&mut self, dir: &str) {
        self.table.push((dir_key(dir), self.at, 0));
    }

    pub fn name(&mut self, name: &[u8]) -> io::Result<()> {
        self.w.write_all(name)?;
        self.w.write_all(b"\n")?;
        let n = name.len() as u32 + 1;
        self.at += n;
        if let Some(last) = self.table.last_mut() {
            last.2 += n;
        }
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        let table_at = self.at;
        for &(k, off, len) in &self.table {
            self.w.write_all(&k.to_le_bytes())?;
            self.w.write_all(&off.to_le_bytes())?;
            self.w.write_all(&len.to_le_bytes())?;
        }
        self.w.write_all(&table_at.to_le_bytes())?;
        self.w.write_all(&(self.table.len() as u32).to_le_bytes())?;
        self.w.write_all(MAGIC)?;
        self.w.flush()?;
        Ok(self.w)
    }
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

/// The trailer: `(table offset, folders)`, or `None` if this is not a names
/// file of this layout (an older one, or cut short).
pub fn trailer<R: Read + Seek>(r: &mut R) -> io::Result<Option<(u32, u32)>> {
    let len = r.seek(SeekFrom::End(0))?;
    if len < TRAILER {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(len - TRAILER))?;
    let mut t = [0u8; TRAILER as usize];
    r.read_exact(&mut t)?;
    let (at, n) = (u32_at(&t, 0), u32_at(&t, 4));
    if &t[8..] != MAGIC || n > MAX_DIRS || at as u64 + n as u64 * 12 + TRAILER != len {
        return Ok(None);
    }
    Ok(Some((at, n)))
}

/// Where `dir`'s names are: `(offset, length)`, the reader left at the
/// offset; `Ok(None)` if the file has no such folder (or is not a names
/// file of this layout).
pub fn find<R: Read + Seek>(r: &mut R, dir: &str) -> io::Result<Option<(u32, u32)>> {
    let Some((at, n)) = trailer(r)? else { return Ok(None) };
    let key = dir_key(dir);
    r.seek(SeekFrom::Start(at as u64))?;
    let mut e = [0u8; 12 * 32];
    let mut left = n as usize;
    let mut found = None;
    while left > 0 && found.is_none() {
        let k = left.min(32);
        r.read_exact(&mut e[..k * 12])?;
        found = e[..k * 12].as_chunks::<12>().0.iter().find(|c| u32_at(&c[..], 0) == key).map(|c| (u32_at(&c[..], 4), u32_at(&c[..], 8)));
        left -= k;
    }
    match found {
        // A folder's names lie before the table: anything else is a damaged
        // file, refused (so a reader never reads past them).
        Some((off, len)) if off as u64 + len as u64 <= at as u64 => {
            r.seek(SeekFrom::Start(off as u64))?;
            Ok(found)
        }
        Some(_) => Ok(None),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Cursor};

    fn names_of(file: &[u8], dir: &str) -> Option<Vec<String>> {
        let mut c = Cursor::new(file);
        let (_, len) = find(&mut c, dir).unwrap()?;
        let lines = BufReader::new(c.take(len as u64)).lines().map(Result::unwrap).collect();
        Some(lines)
    }

    #[test]
    fn each_folder_read_alone() {
        let mut w = Writer::new(Vec::new());
        w.begin("/gifs/Consoles");
        for n in ["SNES_Plok_08.gif", "NES_Mario_01.gif"] {
            w.name(n.as_bytes()).unwrap();
        }
        w.begin("/gifs/Empty");
        w.begin("/gifs/Arcade");
        w.name(b"ARCADE_Pac-Man_01.gif").unwrap();
        let file = w.finish().unwrap();
        assert_eq!(names_of(&file, "/gifs/Consoles").unwrap(), ["SNES_Plok_08.gif", "NES_Mario_01.gif"]);
        assert_eq!(names_of(&file, "/gifs/consoles/").unwrap(), ["SNES_Plok_08.gif", "NES_Mario_01.gif"]);
        assert_eq!(names_of(&file, "/gifs/Arcade").unwrap(), ["ARCADE_Pac-Man_01.gif"]);
        assert_eq!(names_of(&file, "/gifs/Empty").unwrap(), Vec::<String>::new());
        assert_eq!(names_of(&file, "/gifs/Other"), None);
    }

    #[test]
    fn many_folders_and_a_damaged_or_old_file() {
        let mut w = Writer::new(Vec::new());
        for d in 0..100 {
            w.begin(&format!("/gifs/D{d}"));
            w.name(format!("f{d}.gif").as_bytes()).unwrap();
        }
        let file = w.finish().unwrap();
        assert_eq!(names_of(&file, "/gifs/D77").unwrap(), ["f77.gif"]);
        // The older layout (`/gifs/<dir>/<name>` lines) and a cut file.
        assert_eq!(trailer(&mut Cursor::new(b"/gifs/Consoles/a.gif\n".to_vec())).unwrap(), None);
        assert_eq!(trailer(&mut Cursor::new(file[..file.len() - 1].to_vec())).unwrap(), None);
        assert_eq!(names_of(&file[..file.len() - 3], "/gifs/D1"), None);
    }

    #[test]
    fn a_folder_reaching_into_the_table_is_refused() {
        let mut w = Writer::new(Vec::new());
        w.begin("/gifs/A");
        w.name(b"a.gif").unwrap();
        let mut file = w.finish().unwrap();
        // The folder's length (its table entry's third u32) made too long.
        let at = u32_at(&file, file.len() - 12) as usize;
        file[at + 8..at + 12].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(find(&mut Cursor::new(&file), "/gifs/A").unwrap(), None);
    }
}
