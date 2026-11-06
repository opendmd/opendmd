//! `opendmd-pack www` / `www-info`: the web app as a www pack
//! (`opendmd_core::www_pack`, `docs/www-pack.md`).

use anyhow::{bail, Context, Result};
use opendmd_core::www_pack::{self, Pack, PackFile};
use std::fs;
use std::path::{Path, PathBuf};

/// The `www` partition in `firmware/partitions.csv`: 0x100000 (table v3; the
/// smaller of the two layouts: the 8 MB table's is 0x200000, and one pack
/// fits both).
pub const PARTITION_BYTES: usize = 0x10_0000;

/// Every regular file under `dir` (dotfiles and directories skipped), as
/// `(pack path, file)`, sorted.
fn walk(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                let rel = path
                    .strip_prefix(dir)?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((rel, path));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The files of a built web app as the device serves them: `<file>.gz` in
/// place of `<file>` wherever `make web` wrote one (the firmware sends it
/// with `Content-Encoding: gzip`, as it did from the card), the files
/// `skip` names (and their `.gz`) left out.
pub fn collect(dir: &Path, skip: &[&Path]) -> Result<Vec<PackFile>> {
    let all = walk(dir)?;
    let names: std::collections::HashSet<&str> = all.iter().map(|(r, _)| r.as_str()).collect();
    // (By their folder's real path and their name: a skipped file need not exist.)
    let real = |p: &Path| Some(fs::canonicalize(p.parent()?).ok()?.join(p.file_name()?));
    let skip: Vec<PathBuf> = skip.iter().filter_map(|s| real(s)).collect();
    let mut files = Vec::new();
    for (rel, path) in &all {
        let base = rel.strip_suffix(".gz").unwrap_or(rel);
        if real(&dir.join(base)).is_some_and(|r| skip.contains(&r)) {
            continue;
        }
        let (logical, gzip) = match rel.strip_suffix(".gz") {
            // A `.gz` beside its original is that file's content; a lone
            // `.gz` stands for its original.
            Some(base) if names.contains(base) => continue,
            Some(base) => (base.to_string(), true),
            None if names.contains(format!("{rel}.gz").as_str()) => (rel.clone(), true),
            None => (rel.clone(), false),
        };
        let source = if gzip && !rel.ends_with(".gz") {
            path.with_file_name(format!(
                "{}.gz",
                path.file_name().unwrap().to_string_lossy()
            ))
        } else {
            path.clone()
        };
        let data = fs::read(&source).with_context(|| format!("reading {}", source.display()))?;
        files.push(PackFile {
            path: logical,
            data,
            gzip,
        });
    }
    Ok(files)
}

/// The web demo's player: published with the site's demo, never packed.
pub const DEMO_ONLY: &str = "js/demo.js";

pub fn pack(
    dir: &Path,
    out: &Path,
    version: Option<String>,
    version_file: Option<PathBuf>,
    max_bytes: usize,
) -> Result<()> {
    let version = match (version, &version_file) {
        (Some(v), _) => v,
        (None, Some(f)) => fs::read_to_string(f)
            .with_context(|| format!("reading {}", f.display()))?
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
        (None, None) => bail!("--version or --version-file is required"),
    };
    // The version file is the header's, and the web demo's player (js/demo.js)
    // is only ever loaded from the site: neither goes on the device.
    let demo = dir.join(DEMO_ONLY);
    let mut skip = vec![demo.as_path()];
    skip.extend(version_file.as_deref());
    let files = collect(dir, &skip)?;
    if !files.iter().any(|f| f.path == "index.html") {
        bail!("{} has no index.html: run `make web` first", dir.display());
    }
    let count = files.len();
    let gzipped = files.iter().filter(|f| f.gzip).count();
    let bytes = www_pack::build(&version, files)?;
    let pct = bytes.len() as f64 * 100.0 / max_bytes as f64;
    println!(
        "pack: {} B of {} KB ({pct:.1}%), {count} files ({gzipped} gzipped), version '{version}'",
        group(bytes.len()),
        group(max_bytes / 1024),
    );
    if bytes.len() > max_bytes {
        bail!(
            "the pack is {} B, over the {} B www partition",
            bytes.len(),
            max_bytes
        );
    }
    // Written whole, then renamed: a half-written pack is never flashed.
    let tmp = out.with_extension("pack.tmp");
    fs::write(&tmp, &bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, out)?;
    // Read back through the reader the firmware uses.
    let check = fs::read(out)?;
    Pack::parse(&check)?.verify()?;
    println!("{}", out.display());
    Ok(())
}

pub fn info(path: &Path) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let pack = Pack::parse(&bytes)?;
    println!(
        "version '{}', {} entries, {} B (file {} B), CRC {:08x}",
        pack.version(),
        pack.count(),
        pack.len(),
        bytes.len(),
        pack.header().crc32
    );
    for e in pack.entries() {
        println!(
            "{:>9} {} {}",
            e.data.len(),
            if e.gzip { "gz" } else { "  " },
            e.path
        );
    }
    pack.verify()?;
    println!("ok");
    Ok(())
}

/// `1310720` -> `1,310,720`.
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_a_built_app_the_way_the_device_serves_it() {
        let dir = std::env::temp_dir().join(format!("opendmd-www-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("js")).unwrap();
        fs::create_dir_all(dir.join("pkg")).unwrap();
        fs::write(dir.join("index.html"), "raw html").unwrap();
        fs::write(dir.join("index.html.gz"), "gz html").unwrap();
        fs::write(dir.join("js/a.js"), "raw js").unwrap();
        fs::write(dir.join("js/a.js.gz"), "gz js").unwrap();
        fs::write(dir.join("pkg/x.png"), "png").unwrap();
        // The web demo's player, and a lone .gz of it: the site's only.
        fs::write(dir.join("js/demo.js"), "demo").unwrap();
        fs::write(dir.join("js/demo.js.gz"), "gz demo").unwrap();
        fs::write(dir.join("version.txt"), "v9-test\n").unwrap();
        fs::write(dir.join(".hidden"), "no").unwrap();
        let out = dir.join("out.pack");
        pack(
            &dir,
            &out,
            None,
            Some(dir.join("version.txt")),
            PARTITION_BYTES,
        )
        .unwrap();
        let bytes = fs::read(&out).unwrap();
        let p = Pack::parse(&bytes).unwrap();
        p.verify().unwrap();
        assert_eq!(p.version(), "v9-test");
        let got: Vec<_> = p
            .entries()
            .map(|e| (e.path, std::str::from_utf8(e.data).unwrap(), e.gzip))
            .collect();
        assert_eq!(
            got,
            [
                ("index.html", "gz html", true),
                ("js/a.js", "gz js", true),
                ("pkg/x.png", "png", false),
            ]
        );
        fs::remove_file(dir.join("js/demo.js")).unwrap();
        let only_gz = collect(&dir, &[&dir.join("js/demo.js")]).unwrap();
        assert!(
            only_gz.iter().all(|f| f.path != "js/demo.js"),
            "a lone demo.js.gz is left out too"
        );
        // Over the limit: refused, nothing written.
        let small = dir.join("small.pack");
        assert!(pack(&dir, &small, Some("v".into()), None, 100).is_err());
        assert!(!small.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn groups_digits() {
        assert_eq!(group(1_310_720), "1,310,720");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1,000");
    }
}
