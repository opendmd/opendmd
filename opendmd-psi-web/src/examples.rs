//! The examples (`opendmd-web/formulas/*.psi`) read on the host, for the
//! tests and the contact sheet (`examples/sheet.rs`). Each file is a header
//! of comment lines — `-- author: …`, `-- licence: …`, and `-- bundle` on
//! the ones the web app ships (`tools/formulas-json.py` writes those to
//! `www/formulas.json`) — then the formula, which says its own name and
//! what it is (`name "…"`, `about "…"`), read here from the compiler.

extern crate std;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::path::Path;

/// One example.
#[derive(Clone, Debug, Default)]
pub struct Example {
    /// The file's name without `.psi` (the gallery's slug).
    pub slug: String,
    /// Its `name "…"` and `about "…"` lines' text.
    pub name: String,
    pub about: String,
    /// The formula: the file without its header (what the app gets).
    pub src: String,
    /// It carries `-- bundle`: the web app (and so the panel) ships it.
    pub bundled: bool,
}

/// The header's lines (comments the gallery reads; not the formula's).
const HEADER: [&str; 4] = ["-- author:", "-- licence:", "-- license:", "-- bundle"];

/// A file's header lines and the formula after them.
pub fn split(text: &str) -> (Vec<&str>, &str) {
    let mut head = Vec::new();
    let mut rest = text;
    while let Some(line) = rest.split_inclusive('\n').next() {
        if !HEADER.iter().any(|h| line.starts_with(h)) {
            break;
        }
        head.push(line.trim_end());
        rest = &rest[line.len()..];
    }
    (head, rest)
}

/// The folder the examples live in.
pub fn folder() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../opendmd-web/formulas"))
}

/// Every example in `dir`, in file-name order (as `formulas.json` and the
/// gallery list them).
pub fn examples_in(dir: &Path) -> Vec<Example> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "psi"))
        .collect();
    files.sort();
    let mut prog = Box::<opendmd_psi::Program>::default();
    files
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            let (head, src) = split(&text);
            let slug = p.file_stem().unwrap().to_string_lossy().to_string();
            let d = opendmd_psi::compile_declared(src, &mut prog)
                .unwrap_or_else(|e| panic!("{slug}: {}", e.render(src)));
            let text_of = |r: Option<(u32, u32)>| {
                r.map_or(String::new(), |(a, b)| src[a as usize..b as usize].to_string())
            };
            Example {
                name: text_of(d.name),
                about: text_of(d.about),
                bundled: head.contains(&"-- bundle"),
                src: src.to_string(),
                slug,
            }
        })
        .collect()
}

/// Every example (`opendmd-web/formulas/`).
pub fn examples() -> Vec<Example> {
    examples_in(folder())
}
