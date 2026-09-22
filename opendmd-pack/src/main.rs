//! `opendmd-pack`: turn a card's GIFs into ODM v1 files and point a manifest
//! at them.
//!
//! ```text
//! opendmd-pack convert  <card-root> [--jobs N] [--verify] [--force]
//! opendmd-pack manifest <card-root> [--manifest filelist.txt] [--out filelist.odm.txt]
//! ```
//!
//! `convert` writes `<name>.odm` beside every `<name>.gif` under
//! `<card-root>/gifs/` and prints the same report `tools/odm-proto.py` does.
//! A file is up to date — and skipped unless `--force` — when its `.odm`
//! exists, is at least a header long, and carries the `.gif`'s mtime, which
//! the converter copies onto every file it writes. `--verify` decodes every
//! `.odm` (converted or skipped) back through `OdmSource` and compares it with
//! `GifSource` on the original, frame by frame: canvases and delays must be
//! identical, except for files whose palette had to be quantised, where the
//! frame count and delays must match and the colour error is reported.
//!
//! `manifest` rewrites a manifest's `/gifs/....gif` lines to `.odm` wherever
//! that file exists on the card, keeping order, blank lines and line endings.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use opendmd_core::canvas::Canvas;
use opendmd_core::odm::{self, EncodeStats, OdmSource};
use opendmd_core::player::GifSource;
use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

#[derive(Parser)]
#[command(version, about = "Convert a card's GIFs to ODM v1")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write <name>.odm beside every .gif under <card-root>/gifs and report.
    Convert {
        card_root: PathBuf,
        /// Worker threads (default: every core).
        #[arg(long)]
        jobs: Option<usize>,
        /// Decode every .odm back and compare it with the GIF, frame by frame.
        #[arg(long)]
        verify: bool,
        /// Re-encode even when the .odm is up to date.
        #[arg(long)]
        force: bool,
    },
    /// Rewrite a manifest's paths to .odm wherever the .odm exists.
    Manifest {
        card_root: PathBuf,
        /// Manifest to read, relative to the card root.
        #[arg(long, default_value = "filelist.txt")]
        manifest: PathBuf,
        /// Where to write the result, relative to the card root.
        #[arg(long, default_value = "filelist.odm.txt")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Convert {
            card_root,
            jobs,
            verify,
            force,
        } => convert(&card_root, jobs, verify, force),
        Cmd::Manifest {
            card_root,
            manifest,
            out,
        } => rewrite_manifest(&card_root, &manifest, &out),
    }
}

// ---------------------------------------------------------------------------
// convert
// ---------------------------------------------------------------------------

/// One file's result.
struct Done {
    rel: String,
    gif_bytes: u64,
    odm_bytes: u64,
    /// `None` when the file was up to date and not re-encoded.
    stats: Option<EncodeStats>,
    verify: Option<Verdict>,
}

enum Verdict {
    /// Every canvas and delay identical.
    Exact {
        frames: u64,
    },
    /// Quantised palette: frames and delays match, colours within `max_err`.
    Lossy {
        frames: u64,
        max_err: u8,
    },
    Fail(String),
}

fn find_gifs(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            find_gifs(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn mtime(meta: &fs::Metadata) -> Option<SystemTime> {
    meta.modified().ok()
}

fn convert(root: &Path, jobs: Option<usize>, verify: bool, force: bool) -> Result<()> {
    let gif_root = root.join("gifs");
    if !gif_root.is_dir() {
        bail!("{}: no gifs/ directory", root.display());
    }
    let mut files = Vec::new();
    find_gifs(&gif_root, &mut files).with_context(|| format!("walking {}", gif_root.display()))?;
    files.sort();
    let jobs = jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
        .max(1);
    println!(
        "found {} .gif files under {} ({jobs} jobs{})",
        files.len(),
        gif_root.display(),
        if verify { ", verifying" } else { "" }
    );

    let t0 = Instant::now();
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let results: Mutex<Vec<Done>> = Mutex::new(Vec::with_capacity(files.len()));
    let errors: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = files.get(i) else { break };
                let rel = path
                    .strip_prefix(&gif_root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned();
                match process(path, rel.clone(), force, verify) {
                    Ok(done) => results.lock().unwrap().push(done),
                    Err(e) => errors
                        .lock()
                        .unwrap()
                        .push((rel, format!("{e}: {}", e.root_cause()))),
                }
                let n = finished.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(500) {
                    eprintln!(
                        "  {n}/{} done, {:.1}s elapsed",
                        files.len(),
                        t0.elapsed().as_secs_f64()
                    );
                }
            });
        }
    });
    let elapsed = t0.elapsed().as_secs_f64();
    let mut results = results.into_inner().unwrap();
    let mut errors = errors.into_inner().unwrap();
    results.sort_by(|a, b| a.rel.cmp(&b.rel));
    errors.sort();
    report(&results, &errors, files.len(), jobs, elapsed);
    let failed = results
        .iter()
        .filter(|r| matches!(r.verify, Some(Verdict::Fail(_))))
        .count();
    if failed > 0 {
        bail!("{failed} files failed verification");
    }
    Ok(())
}

fn process(gif_path: &Path, rel: String, force: bool, verify: bool) -> Result<Done> {
    let odm_path = gif_path.with_extension("odm");
    let gif_meta = fs::metadata(gif_path)?;
    let up_to_date = !force
        && fs::metadata(&odm_path).is_ok_and(|m| {
            m.len() >= odm::HEADER_LEN as u64
                && mtime(&m).is_some()
                && mtime(&m) == mtime(&gif_meta)
        });

    if up_to_date && !verify {
        return Ok(Done {
            rel,
            gif_bytes: gif_meta.len(),
            odm_bytes: fs::metadata(&odm_path)?.len(),
            stats: None,
            verify: None,
        });
    }
    let gif = fs::read(gif_path).context("reading the GIF")?;
    let (stats, odm_bytes) = if up_to_date {
        (None, fs::metadata(&odm_path)?.len())
    } else {
        let mut out = Vec::new();
        let stats = odm::encode_gif(&gif[..], &mut out).context("encoding")?;
        // Temp + rename, so an interrupted run never leaves a short .odm
        // with a current mtime behind.
        let tmp = odm_path.with_extension("odm.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&out)?;
            if let Some(t) = mtime(&gif_meta) {
                f.set_modified(t)?;
            }
        }
        fs::rename(&tmp, &odm_path)?;
        (Some(stats), out.len() as u64)
    };

    let verify = if verify {
        let odm_file = BufReader::new(fs::File::open(&odm_path)?);
        Some(
            compare(&gif, odm_file, stats.as_ref().map(|s| s.quantised))
                .unwrap_or_else(|e| Verdict::Fail(format!("{e:#}"))),
        )
    } else {
        None
    };
    Ok(Done {
        rel,
        gif_bytes: gif.len() as u64,
        odm_bytes,
        stats,
        verify,
    })
}

/// Play both files side by side, exactly as the engine would.
/// `quantised` is what the encoder reported; for a skipped (up-to-date) file it
/// is unknown, and a 256-entry palette is taken as the sign of quantisation.
fn compare(gif: &[u8], odm: impl Read, quantised: Option<bool>) -> Result<Verdict> {
    let mut g = GifSource::new(gif)?;
    let mut o = OdmSource::new(odm)?;
    if g.logical_size() != o.logical_size() {
        return Ok(Verdict::Fail(format!(
            "size {:?} vs {:?}",
            g.logical_size(),
            o.logical_size()
        )));
    }
    let (mut cg, mut co) = (Canvas::new(), Canvas::new());
    let mut frames = 0u64;
    let mut max_err = 0u8;
    loop {
        let (dg, d_o) = (g.next_frame(&mut cg)?, o.next_frame(&mut co)?);
        if dg != d_o {
            return Ok(Verdict::Fail(format!(
                "frame {frames}: delay {dg:?} vs {d_o:?}"
            )));
        }
        if dg.is_none() {
            break;
        }
        for (a, b) in cg.as_bytes().iter().zip(co.as_bytes()) {
            max_err = max_err.max(a.abs_diff(*b));
        }
        frames += 1;
    }
    let quantised = quantised.unwrap_or(o.palette_len() == 256);
    Ok(match (max_err, quantised) {
        (0, _) => Verdict::Exact { frames },
        (e, true) => Verdict::Lossy { frames, max_err: e },
        (e, false) => Verdict::Fail(format!("pixels differ by up to {e}")),
    })
}

/// numpy's default (linear) percentile, so the numbers line up with the
/// prototype's report.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = p / 100.0 * (sorted.len() - 1) as f64;
    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64)
}

fn report(results: &[Done], errors: &[(String, String)], found: usize, jobs: usize, secs: f64) {
    let skipped = results.iter().filter(|r| r.stats.is_none()).count();
    println!(
        "convert pass done: {} ok ({} encoded, {skipped} up to date), {} errors, {secs:.1}s",
        results.len(),
        results.len() - skipped,
        errors.len()
    );

    println!("\n=== ERRORS (skipped files) ===");
    for (rel, e) in errors {
        println!("  {rel}: {}", e.lines().next().unwrap_or(""));
    }
    if results.is_empty() {
        println!("no successful results");
        return;
    }

    let total_gif: u64 = results.iter().map(|r| r.gif_bytes).sum();
    let total_odm: u64 = results.iter().map(|r| r.odm_bytes).sum();
    let ratio = |r: &Done| r.odm_bytes as f64 / r.gif_bytes as f64;
    let mut ratios: Vec<f64> = results.iter().map(ratio).collect();
    ratios.sort_by(f64::total_cmp);
    let mut worst: Vec<&Done> = results.iter().collect();
    worst.sort_by(|a, b| ratio(b).total_cmp(&ratio(a)));

    println!("\n=== SIZE ===");
    println!(
        "total gif bytes: {total_gif} ({:.1} MB)",
        total_gif as f64 / 1e6
    );
    println!(
        "total odm bytes: {total_odm} ({:.1} MB)",
        total_odm as f64 / 1e6
    );
    println!(
        "overall ratio odm/gif: {:.4}",
        total_odm as f64 / total_gif as f64
    );
    println!(
        "per-file ratio median: {:.4}  p95: {:.4}",
        percentile(&ratios, 50.0),
        percentile(&ratios, 95.0)
    );
    println!("worst 10 files (name, gif_size, odm_size, frames, ratio):");
    for r in worst.iter().take(10) {
        let frames = r
            .stats
            .as_ref()
            .map_or("?".to_string(), |s| s.frames.to_string());
        println!(
            "  {}  gif={}  odm={}  frames={frames}  ratio={:.3}",
            r.rel,
            r.gif_bytes,
            r.odm_bytes,
            ratio(r)
        );
    }

    let encoded: Vec<&EncodeStats> = results.iter().filter_map(|r| r.stats.as_ref()).collect();
    if !encoded.is_empty() {
        let keep: u64 = encoded.iter().map(|s| s.keep_px).sum();
        let fill: u64 = encoded.iter().map(|s| s.fill_px).sum();
        let lit: u64 = encoded.iter().map(|s| s.lit_px).sum();
        let total = (keep + fill + lit).max(1);
        let pct = |n: u64| 100.0 * n as f64 / total as f64;
        println!(
            "\n=== OP USAGE (archive-wide pixel share{}) ===",
            if skipped > 0 {
                ", encoded files only"
            } else {
                ""
            }
        );
        println!("total pixels (sum over all frames): {total}");
        println!("KEEP: {keep} ({:.2}%)", pct(keep));
        println!("FILL: {fill} ({:.2}%)", pct(fill));
        println!("LIT:  {lit} ({:.2}%)", pct(lit));

        println!("\n=== PALETTE ===");
        let quantised: Vec<&&EncodeStats> = encoded.iter().filter(|s| s.quantised).collect();
        println!(
            "files needing quantisation (>256 colours): {} / {}",
            quantised.len(),
            encoded.len()
        );
        println!(
            "files with a single-colour palette: {}",
            encoded.iter().filter(|s| s.palette_len == 1).count()
        );
        println!(
            "files with <=4 colours: {}",
            encoded.iter().filter(|s| s.palette_len <= 4).count()
        );
        let pal_bytes: Vec<usize> = encoded.iter().map(|s| s.palette_len * 3).collect();
        println!(
            "palette bytes per file: max={} mean={:.1}",
            pal_bytes.iter().max().unwrap_or(&0),
            pal_bytes.iter().sum::<usize>() as f64 / pal_bytes.len() as f64
        );

        println!("\n=== STRAYS (not 128x32) ===");
        for (r, s) in results
            .iter()
            .filter_map(|r| r.stats.as_ref().map(|s| (r, s)))
        {
            if (s.width, s.height) != (128, 32) {
                println!("  {}: {}x{}", r.rel, s.width, s.height);
            }
        }
    }

    let verified: Vec<&Done> = results.iter().filter(|r| r.verify.is_some()).collect();
    if !verified.is_empty() {
        let (mut exact, mut lossy, mut frames) = (0, Vec::new(), 0u64);
        let mut fails = Vec::new();
        for r in &verified {
            match r.verify.as_ref().unwrap() {
                Verdict::Exact { frames: f } => {
                    exact += 1;
                    frames += f;
                }
                Verdict::Lossy { frames: f, max_err } => {
                    lossy.push((r.rel.as_str(), *max_err));
                    frames += f;
                }
                Verdict::Fail(e) => fails.push((r.rel.as_str(), e.as_str())),
            }
        }
        println!("\n=== VERIFICATION (OdmSource vs GifSource, every frame) ===");
        println!("files verified: {} / {}", verified.len(), results.len());
        println!(
            "identical: {exact}  quantised (frames+delays match, colours approximate): {}  fail: {}",
            lossy.len(),
            fails.len()
        );
        println!("frames compared: {frames}");
        if !lossy.is_empty() {
            let worst = lossy.iter().map(|l| l.1).max().unwrap_or(0);
            println!("quantised files: worst per-channel error {worst}");
        }
        for (rel, e) in fails.iter().take(20) {
            println!("  FAIL {rel}: {e}");
        }
    }

    println!("\n=== TIMING ===");
    println!("wall clock for {found} files with {jobs} jobs: {secs:.1}s");
}

// ---------------------------------------------------------------------------
// manifest
// ---------------------------------------------------------------------------

/// `Some(path with .odm)` if `line` names a `.gif` whose `.odm` sits beside it.
fn odm_for(root: &Path, line: &str) -> Option<String> {
    let cut = line.len().checked_sub(4)?;
    if !line.get(cut..)?.eq_ignore_ascii_case(".gif") {
        return None;
    }
    let odm = format!("{}.odm", &line[..cut]);
    root.join(odm.trim_start_matches('/'))
        .is_file()
        .then_some(odm)
}

fn rewrite_manifest(root: &Path, manifest: &Path, out: &Path) -> Result<()> {
    let src = root.join(manifest);
    let dst = root.join(out);
    let text = fs::read_to_string(&src).with_context(|| format!("reading {}", src.display()))?;
    let (mut total, mut switched) = (0usize, 0usize);
    let mut missing = Vec::new();
    let mut absent = 0usize;
    let mut result = String::with_capacity(text.len());
    for raw in text.split_inclusive('\n') {
        // Keep the line ending (and any trailing whitespace) byte for byte.
        let body = raw.trim_end_matches(['\n', '\r']);
        let path = body.trim_end();
        let tail = &raw[path.len()..];
        if path.trim().is_empty() {
            result.push_str(raw);
            continue;
        }
        total += 1;
        match odm_for(root, path) {
            Some(odm) => {
                switched += 1;
                result.push_str(&odm);
                result.push_str(tail);
            }
            None => {
                if !root.join(path.trim_start_matches('/')).is_file() {
                    absent += 1;
                } else if path.to_ascii_lowercase().ends_with(".gif") {
                    missing.push(path.to_string());
                }
                result.push_str(raw);
            }
        }
    }
    if dst == src {
        bail!("refusing to overwrite the manifest being read; pick another --out");
    }
    fs::write(&dst, result).with_context(|| format!("writing {}", dst.display()))?;
    println!(
        "{}: {total} entries, {switched} now .odm, {} still .gif (no .odm beside them), \
         {absent} unchanged because the file itself is not on the card -> {}",
        src.display(),
        missing.len(),
        dst.display()
    );
    for m in missing.iter().take(20) {
        println!("  still gif: {m}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_matches_numpy_linear() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 50.0), 2.5);
        assert!((percentile(&v, 95.0) - 3.85).abs() < 1e-12);
        assert_eq!(percentile(&[7.0], 95.0), 7.0);
    }

    #[test]
    fn manifest_lines_switch_only_where_the_odm_exists() {
        let root = std::env::temp_dir().join(format!("opendmd-pack-test-{}", std::process::id()));
        let dir = root.join("gifs/NEW 2021");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SNES_A_01..odm"), b"x").unwrap();
        fs::write(
            root.join("filelist.txt"),
            "/gifs/NEW 2021/SNES_A_01..gif\r\n\n/gifs/NEW 2021/SNES_B_01.gif\n",
        )
        .unwrap();
        rewrite_manifest(&root, Path::new("filelist.txt"), Path::new("out.txt")).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("out.txt")).unwrap(),
            "/gifs/NEW 2021/SNES_A_01..odm\r\n\n/gifs/NEW 2021/SNES_B_01.gif\n"
        );
        assert!(
            rewrite_manifest(&root, Path::new("filelist.txt"), Path::new("filelist.txt")).is_err()
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
