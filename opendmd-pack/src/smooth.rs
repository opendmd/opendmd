//! Smooth motion on the command line (`opendmd_core::tween`,
//! `docs/tweening-experiment.md`).
//!
//! - `smooth analyse <card-root> [--min-percent 10] [--jobs N] [--write]`
//!   analyses every GIF and prints the files where at least that share of
//!   the frame pairs would get in-betweens (the Library's threshold is 10%),
//!   with the frames each would gain, and totals. `--write` adds them to the
//!   card's list (`/sets/.smooth`, `tween::LIST_PATH`): the lines already
//!   there stay (a choice made in the browser is kept), the new ones are
//!   added, and the file is written sorted.
//! - `convert` honours the list (see [`Choice`]).

use crate::recode::{find_sources, jobs_or_all, parallel};
use anyhow::{bail, Context, Result};
use opendmd_core::tween::{self, Smooth, Summary};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The list file on a card.
pub fn list_file(root: &Path) -> PathBuf {
    root.join(tween::LIST_PATH.trim_start_matches('/'))
}

/// Which copies `convert` makes smooth.
pub enum Choice {
    /// The card has a list: exactly the GIFs it names (lowercased `/gifs/…`
    /// paths) get smoothed copies, and every other copy is plain — a copy
    /// that disagrees is re-encoded.
    Listed(BTreeSet<String>),
    /// No list on the card: new copies are plain, and a copy that carries
    /// the marker is kept smooth.
    Unlisted,
}

impl Choice {
    pub fn load(root: &Path) -> Result<Self> {
        let path = list_file(root);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Choice::Listed(
                tween::parse_list(&text)
                    .into_iter()
                    .map(|p| p.to_lowercase())
                    .collect(),
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Choice::Unlisted),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Should the copy of `gifs/<rel>` be smooth, given whether the copy on
    /// the card (if any) carries the marker?
    pub fn wants(&self, rel: &str, copy_marked: bool) -> bool {
        match self {
            Choice::Listed(set) => set.contains(&format!("/gifs/{rel}").to_lowercase()),
            Choice::Unlisted => copy_marked,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Choice::Listed(s) => format!("{} files listed in {}", s.len(), tween::LIST_PATH),
            Choice::Unlisted => format!("no {}", tween::LIST_PATH),
        }
    }
}

/// Does the file at `path` carry the Smooth motion marker? Reads only its
/// first KB.
pub fn copy_is_marked(path: &Path) -> bool {
    use std::io::Read;
    let Ok(f) = fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::with_capacity(1024);
    if f.take(1024).read_to_end(&mut head).is_err() {
        return false;
    }
    tween::marker(&head).is_some()
}

/// Analyse one GIF.
pub fn analyse_bytes(gif: Vec<u8>) -> Result<Summary> {
    let mut s = Smooth::new(gif)?;
    while !s.analyse(u32::MAX)? {}
    Ok(s.summary())
}

pub fn analyse(root: &Path, min_percent: u32, jobs: Option<usize>, write: bool) -> Result<()> {
    let gif_root = root.join("gifs");
    if !gif_root.is_dir() {
        bail!("{}: no gifs/ directory", root.display());
    }
    let files =
        find_sources(&gif_root, None).with_context(|| format!("walking {}", gif_root.display()))?;
    let jobs = jobs_or_all(jobs);
    println!(
        "analysing {} GIFs under {} ({jobs} jobs), threshold {min_percent}% of frame pairs",
        files.len(),
        gif_root.display()
    );
    let t0 = Instant::now();
    let (results, errors) = parallel(&files, jobs, &gif_root, |path, _| {
        analyse_bytes(fs::read(path)?)
    });
    let secs = t0.elapsed().as_secs_f64();
    let at_or_above =
        |s: &Summary| s.pairs > 0 && 100 * s.tweened as u64 >= min_percent as u64 * s.pairs as u64;
    let mut hits: Vec<&(String, Summary)> =
        results.iter().filter(|(_, s)| at_or_above(s)).collect();
    hits.sort_by(|a, b| {
        let pa = a.1.tweened as f64 / a.1.pairs as f64;
        let pb = b.1.tweened as f64 / b.1.pairs as f64;
        pb.total_cmp(&pa).then_with(|| a.0.cmp(&b.0))
    });
    println!(
        "\n{:>7} {:>7} {:>8} {:>7}  file",
        "percent", "frames", "+frames", "tweened"
    );
    for (rel, s) in &hits {
        println!(
            "{:>6.1}% {:>7} {:>8} {:>7}  /gifs/{rel}",
            100.0 * s.tweened as f64 / s.pairs as f64,
            s.frames,
            s.added,
            s.tweened
        );
    }
    let all_pairs: u64 = results.iter().map(|r| r.1.pairs as u64).sum();
    let all_tweened: u64 = results.iter().map(|r| r.1.tweened as u64).sum();
    let all_added: u64 = results.iter().map(|r| r.1.added as u64).sum();
    let all_frames: u64 = results.iter().map(|r| r.1.frames as u64).sum();
    let hit_added: u64 = hits.iter().map(|r| r.1.added as u64).sum();
    let hit_frames: u64 = hits.iter().map(|r| r.1.frames as u64).sum();
    let with_any = results.iter().filter(|r| r.1.tweened > 0).count();
    println!("\n=== TOTALS ===");
    println!(
        "files analysed: {}  errors: {}  with any tweenable pair: {with_any}",
        results.len(),
        errors.len()
    );
    println!(
        "files at or above {min_percent}%: {}  their frames {hit_frames} -> {} (+{hit_added})",
        hits.len(),
        hit_frames + hit_added
    );
    println!(
        "whole card: {all_tweened} of {all_pairs} pairs tweenable ({:.2}%), +{all_added} frames on {all_frames} ({:.1}%)",
        100.0 * all_tweened as f64 / all_pairs.max(1) as f64,
        100.0 * all_added as f64 / all_frames.max(1) as f64
    );
    for (rel, e) in &errors {
        println!("  error {rel}: {}", e.lines().next().unwrap_or(""));
    }
    println!("wall clock: {secs:.1}s with {jobs} jobs");

    if write {
        let path = list_file(root);
        let old = fs::read_to_string(&path).unwrap_or_default();
        let mut lines = tween::parse_list(&old);
        let have: BTreeSet<String> = lines.iter().map(|l| l.to_lowercase()).collect();
        let before = lines.len();
        for (rel, _) in &hits {
            let p = format!("/gifs/{rel}");
            if !have.contains(&p.to_lowercase()) {
                lines.push(p);
            }
        }
        lines.sort();
        lines.dedup();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, tween::render_list(&lines))?;
        fs::rename(&tmp, &path)?;
        println!(
            "wrote {}: {} lines ({} were there, {} added)",
            path.display(),
            lines.len(),
            before,
            lines.len() - before.min(lines.len())
        );
    }
    Ok(())
}
