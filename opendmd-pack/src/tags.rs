//! `tags check <card-root> [--search "<words>"]`: the card's `/sets/.tags`
//! against its GIFs — the lines whose GIF is gone (a card edited on a PC),
//! the user's tags with their counts, and the details the names give
//! (`opendmd_core::tags::details`); with `--search`, the GIFs a search
//! finds (`opendmd_core::query`, what a smart playlist would hold).

use anyhow::{Context, Result};
use opendmd_core::manifest::Entry;
use opendmd_core::query::parse_query;
use opendmd_core::tags::{details, TagSet};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

/// Every `/gifs/<folder>/<name>.gif` on the card, by folder then name.
fn card_gifs(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let gifs = root.join("gifs");
    let mut dirs: Vec<_> = fs::read_dir(&gifs)
        .with_context(|| format!("reading {}", gifs.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    dirs.sort_by_key(|e| e.file_name());
    for d in dirs {
        let folder = d.file_name().to_string_lossy().into_owned();
        let mut names: Vec<String> = fs::read_dir(d.path())?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_ascii_lowercase().ends_with(".gif") && !n.starts_with('.'))
            .collect();
        names.sort();
        out.extend(names.into_iter().map(|n| format!("/gifs/{folder}/{n}")));
    }
    Ok(out)
}

pub fn check(root: &Path, search: Option<&str>) -> Result<()> {
    let file = root.join("sets/.tags");
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let set = TagSet::from_text(&text);
    let gifs = card_gifs(root)?;
    let have: HashSet<String> = gifs.iter().map(|p| p.to_ascii_lowercase()).collect();

    println!(
        "{} GIFs; /sets/.tags: {} B, {} lines",
        gifs.len(),
        text.len(),
        set.len()
    );
    let gone: Vec<&str> = set
        .iter()
        .filter(|l| !have.contains(&l.path.to_ascii_lowercase()))
        .map(|l| l.path.as_str())
        .collect();
    println!("lines whose GIF is not on the card: {}", gone.len());
    for p in &gone {
        println!("  {p}");
    }
    let rated = set.iter().filter(|l| l.rating.is_some()).count();
    println!("rated: {rated}");
    for (t, n) in set.tag_counts() {
        println!("  tag {t}: {n}");
    }

    let (mut named, mut lent) = (0usize, 0usize);
    let mut is: BTreeMap<&str, usize> = BTreeMap::new();
    let mut authors = 0usize;
    for p in &gifs {
        let e = Entry::from_path(p);
        let d = details(&e);
        match (&e.platform, &d.platform) {
            (Some(_), _) => named += 1,
            (None, Some(_)) => lent += 1,
            _ => {}
        }
        for w in d.is {
            *is.entry(w).or_default() += 1;
        }
        authors += usize::from(d.by.is_some());
    }
    println!(
        "platform from the name: {named}; from the folder: {lent}; none: {}",
        gifs.len() - named - lent
    );
    println!("with an author: {authors}");
    for (w, n) in is {
        println!("  is:{w}: {n}");
    }

    if let Some(s) = search {
        let q = parse_query(s).map_err(|e| anyhow::anyhow!("at {}: {}", e.at, e.message))?;
        let found = q.run(gifs.iter().map(String::as_str), Some(&set));
        println!("{} ({}): {} GIFs", q.render(), q.describe(), found.len());
        for p in found {
            println!("  {p}");
        }
    }
    Ok(())
}
