//! `opendmd-pack`: prepare a card — write every GIF's prepared copy, the
//! file the player plays in its place.
//!
//! ```text
//! opendmd-pack convert <card-root> [--jobs N] [--verify] [--force]
//! opendmd-pack smooth analyse <card-root> [--min-percent 10] [--jobs N] [--write]
//! ```
//!
//! `convert` writes, for every `<card-root>/gifs/<Category>/<name>.gif`, its
//! prepared copy at `<card-root>/cache/<Category>/<xx>/<name>.gif` (see
//! `opendmd_core::layout` and `docs/cache-format.md`): a standard GIF89a,
//! LZW capped at 9 bits, delta frames, the `OPENDMD1LZW` hint —
//! `gifslim::recode_gif` in the prepared format, byte-identical to what the
//! browser and the device's own cache builder write. A copy is up to date —
//! and skipped unless `--force` — when it exists and carries the GIF's
//! mtime, which the converter copies onto every file it writes. `--verify`
//! plays every copy (converted or skipped) against its GIF, frame by frame,
//! through three decoders at once: the original through `GifSource`, the
//! copy through `GifSource` (the `gif` crate: "it is a standard GIF") and
//! the copy through `GifSlim` at cap 9 (what the player uses). Every canvas
//! and delay must be identical, except for files whose palette had to be
//! quantised (more than 255 colours), where the frame count and delays must
//! match and the colour error is reported.
//!
//! **Preparing a card is one command:** `convert <card-root> --verify`. The
//! firmware looks for each GIF's copy there and plays it when it is, so
//! playlists keep naming GIFs and nothing else changes on the card.
//!
//! **Smooth motion** (`opendmd_core::tween`): a GIF named in the card's
//! `/sets/.smooth` gets a smoothed copy — `recode_gif(tween(GIF))` plus the
//! `OPENDMD1TWN` marker, what the browser writes — and every other copy is
//! plain (a copy that disagrees with the list is re-encoded). Without a
//! list, a marked copy is kept smooth. `--verify` plays a smoothed copy
//! against the tweened frames instead of the GIF's (the same three
//! decoders) and checks its bytes are exactly the tweener's.
//! `smooth analyse <card-root> [--min-percent 10] [--jobs N] [--write]`
//! finds the files worth smoothing (and `--write` lists them).
//!
//! `www <dir> --out <pack> --version <v>` packs the built web app
//! (`make web`'s `opendmd-web/www`) into a www pack for the firmware's `www`
//! flash partition (`docs/www-pack.md`); `www-info <pack>` reads one back.
//!
//! `recode` and `gifscan` are the gifslim experiment's tools
//! (`docs/gifslim-experiment.md`).

mod recode;
mod smooth;
mod www;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use opendmd_core::gifslim::{self, RecodeStats, PREPARED};
use opendmd_core::layout;
use opendmd_core::tween::{self, Summary};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

#[derive(Parser)]
#[command(version, about = "Prepare a card's GIFs for the player")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write every GIF's prepared copy under <card-root>/cache and report.
    /// To prepare a card for the player: --verify.
    Convert {
        card_root: PathBuf,
        /// Worker threads (default: every core).
        #[arg(long)]
        jobs: Option<usize>,
        /// Play every copy against its GIF, frame by frame.
        #[arg(long)]
        verify: bool,
        /// Re-encode even when the copy is up to date.
        #[arg(long)]
        force: bool,
    },
    /// Smooth motion: which GIFs gain from in-between frames.
    Smooth {
        #[command(subcommand)]
        cmd: SmoothCmd,
    },
    /// Experiment (docs/gifslim-experiment.md): write <name>.cap<N>[d].gif
    /// beside every original GIF — a standard GIF whose LZW codes never
    /// exceed N bits — and optionally verify it frame by frame.
    Recode {
        card_root: PathBuf,
        /// Widest LZW code, 3..=12.
        #[arg(long)]
        cap: u8,
        /// Delta frames (changed rectangle, unchanged pixels transparent).
        #[arg(long, value_enum)]
        delta: recode::OnOff,
        /// Output tag instead of cap<N>[d]: <name>.<suffix>.gif.
        #[arg(long)]
        out_suffix: Option<String>,
        #[arg(long)]
        jobs: Option<usize>,
        /// Play original and copy (gif crate and gifslim) and compare.
        #[arg(long)]
        verify: bool,
        /// Leave out the OPENDMD1LZW application extension.
        #[arg(long)]
        no_hint: bool,
        /// Unchanged pixels in a delta rectangle always transparent (the
        /// textbook rule), never continuing the previous pixel's run.
        #[arg(long)]
        no_keep_runs: bool,
        /// Encode nothing: verify the copies a previous run wrote.
        #[arg(long)]
        verify_only: bool,
        /// Write a per-file table here.
        #[arg(long)]
        tsv: Option<PathBuf>,
    },
    /// Pack a built web app (`make web`'s opendmd-web/www) into a www pack
    /// for the device's `www` flash partition: every file whose `<file>.gz`
    /// is beside it is stored gzipped, the rest as they are.
    Www {
        dir: PathBuf,
        /// Where to write the pack.
        #[arg(long)]
        out: PathBuf,
        /// The version string for the header (`git describe` of the app).
        #[arg(long, conflicts_with = "version_file")]
        version: Option<String>,
        /// Read the version from this file's first line instead; the file is
        /// left out of the pack if it is inside <dir>.
        #[arg(long)]
        version_file: Option<PathBuf>,
        /// Refuse a pack bigger than this (the `www` partition: 1,310,720).
        #[arg(long, default_value_t = www::PARTITION_BYTES)]
        max_bytes: usize,
    },
    /// Print a www pack's header and entries, and check it (CRC included).
    WwwInfo { pack: PathBuf },
    /// Experiment: play every original GIF with gifslim at caps 9..12
    /// against GifSource.
    Gifscan {
        card_root: PathBuf,
        #[arg(long)]
        jobs: Option<usize>,
        #[arg(long)]
        tsv: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SmoothCmd {
    /// Analyse every GIF; print those where at least --min-percent of the
    /// frame pairs would be tweened, with the frames each would gain.
    Analyse {
        card_root: PathBuf,
        /// The threshold, % of frame pairs (the Library offers Smooth
        /// motion from 10%).
        #[arg(long, default_value_t = tween::OFFER_PERCENT)]
        min_percent: u32,
        /// Worker threads (default: every core).
        #[arg(long)]
        jobs: Option<usize>,
        /// Add the files found to the card's /sets/.smooth (lines already
        /// there are kept); `convert` then makes their copies smooth.
        #[arg(long)]
        write: bool,
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
        Cmd::Smooth {
            cmd:
                SmoothCmd::Analyse {
                    card_root,
                    min_percent,
                    jobs,
                    write,
                },
        } => smooth::analyse(&card_root, min_percent, jobs, write),
        Cmd::Recode {
            card_root,
            cap,
            delta,
            out_suffix,
            jobs,
            verify,
            no_hint,
            no_keep_runs,
            verify_only,
            tsv,
        } => recode::recode(
            &card_root,
            cap,
            delta,
            out_suffix,
            jobs,
            verify,
            !no_hint,
            !no_keep_runs,
            verify_only,
            tsv,
        ),
        Cmd::Gifscan {
            card_root,
            jobs,
            tsv,
        } => recode::gifscan(&card_root, jobs, tsv),
        Cmd::Www {
            dir,
            out,
            version,
            version_file,
            max_bytes,
        } => www::pack(&dir, &out, version, version_file, max_bytes),
        Cmd::WwwInfo { pack } => www::info(&pack),
    }
}

// ---------------------------------------------------------------------------
// convert
// ---------------------------------------------------------------------------

/// One file's result.
struct Done {
    rel: String,
    gif_bytes: u64,
    copy_bytes: u64,
    /// `None` when the file was up to date, not re-encoded.
    stats: Option<RecodeStats>,
    verify: Option<recode::Verdict>,
    frames_compared: u64,
    /// The copy is smoothed: the tweener's analysis (`None` for a plain one).
    smooth: Option<Summary>,
}

fn mtime(meta: &fs::Metadata) -> Option<SystemTime> {
    meta.modified().ok()
}

fn convert(root: &Path, jobs: Option<usize>, verify: bool, force: bool) -> Result<()> {
    let gif_root = root.join("gifs");
    if !gif_root.is_dir() {
        bail!("{}: no gifs/ directory", root.display());
    }
    // (`recode`'s experimental copies are not GIFs to convert.)
    let files = recode::find_sources(&gif_root, None)
        .with_context(|| format!("walking {}", gif_root.display()))?;
    let jobs = jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
        .max(1);
    let choice = smooth::Choice::load(root)?;
    println!(
        "found {} .gif files under {} ({jobs} jobs{}) -> {}; smooth motion: {}",
        files.len(),
        gif_root.display(),
        if verify { ", verifying" } else { "" },
        root.join("cache").display(),
        choice.describe()
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
                    .replace(std::path::MAIN_SEPARATOR, "/");
                match process(root, path, rel.clone(), force, verify, &choice) {
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
        .filter(|r| {
            matches!(
                r.verify,
                Some(recode::Verdict::Fail(_) | recode::Verdict::NeedsFull(_))
            )
        })
        .count();
    if failed > 0 {
        bail!("{failed} files failed verification");
    }
    Ok(())
}

/// A copy at `path` exists and carries the GIF's mtime.
fn is_current(path: &Path, gif_meta: &fs::Metadata) -> bool {
    fs::metadata(path)
        .is_ok_and(|m| m.len() > 13 && mtime(&m).is_some() && mtime(&m) == mtime(gif_meta))
}

fn process(
    root: &Path,
    gif_path: &Path,
    rel: String,
    force: bool,
    verify: bool,
    choice: &smooth::Choice,
) -> Result<Done> {
    let copy_path = root.join(layout::cache_rel(&rel).context("not a .gif name")?);
    let gif_meta = fs::metadata(gif_path)?;
    let marked = smooth::copy_is_marked(&copy_path);
    let want_smooth = choice.wants(&rel, marked);
    // Up to date: the GIF's mtime, and smooth exactly when it should be.
    let up_to_date = !force && is_current(&copy_path, &gif_meta) && marked == want_smooth;
    if up_to_date && !verify {
        return Ok(Done {
            rel,
            gif_bytes: gif_meta.len(),
            copy_bytes: fs::metadata(&copy_path)?.len(),
            stats: None,
            verify: None,
            frames_compared: 0,
            smooth: None,
        });
    }
    let gif = fs::read(gif_path).context("reading the GIF")?;
    // A smooth copy is `recode_gif(tween(GIF))` + the marker; the tweened
    // frames (a plain GIF) are what it is verified against.
    // A GIF the tweener cannot take (a frame past 256 colours) gets a
    // plain copy, and says so.
    let smoothed = if want_smooth {
        let run = || -> std::io::Result<_> {
            let mut s = tween::Smooth::new(&gif[..])?;
            while !s.analyse(u32::MAX)? {}
            while !s.tween(u32::MAX)? {}
            let (copy, stats) = s.prepare(&PREPARED)?;
            Ok((s, copy, stats))
        };
        match run() {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("  {rel}: not smoothed ({e}); a plain copy instead");
                None
            }
        }
    } else {
        None
    };
    let (stats, copy) = if up_to_date {
        (None, fs::read(&copy_path)?)
    } else {
        let (stats, out) = match &smoothed {
            Some((_, copy, stats)) => (stats.clone(), copy.clone()),
            None => {
                let mut out = Vec::with_capacity(gif.len());
                let stats =
                    gifslim::recode_gif(&gif[..], &mut out, &PREPARED).context("encoding")?;
                (stats, out)
            }
        };
        if let Some(parent) = copy_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Temp + rename, so an interrupted run never leaves a short copy
        // with a current mtime behind.
        let tmp = copy_path.with_extension("gif.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&out)?;
            if let Some(t) = mtime(&gif_meta) {
                f.set_modified(t)?;
            }
        }
        fs::rename(&tmp, &copy_path)?;
        (Some(stats), out)
    };
    let (verify, frames_compared) = if verify {
        // For a skipped file whether it was quantised is not known: a copy
        // whose two decoders agree while it differs from the original is
        // reported as quantised, and a fresh encode says for sure.
        let quantised = stats.as_ref().is_none_or(|s| s.quantised);
        match &smoothed {
            // Smoothed: the copy must be exactly the tweener's bytes, and
            // play the tweened frames through both decoders.
            Some((_, want, _)) if *want != copy => (
                Some(recode::Verdict::Fail(
                    "the copy is not recode(tween(GIF)) byte for byte".into(),
                )),
                0,
            ),
            Some((s, _, _)) => {
                let (v, n, _) = recode::compare(s.tweened_gif(), &copy, PREPARED.cap, quantised);
                (Some(v), n)
            }
            None => {
                let (v, n, _) = recode::compare(&gif, &copy, PREPARED.cap, quantised);
                (Some(v), n)
            }
        }
    } else {
        (None, 0)
    };
    Ok(Done {
        rel,
        gif_bytes: gif.len() as u64,
        copy_bytes: copy.len() as u64,
        stats,
        verify,
        frames_compared,
        smooth: smoothed.map(|(s, _, _)| s.summary()),
    })
}

fn verify_note(results: &[Done]) -> bool {
    results
        .iter()
        .any(|r| r.smooth.is_some() && r.verify.is_some())
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

    println!("\n=== ERRORS (no copy; the player plays the GIF, if it can) ===");
    for (rel, e) in errors {
        println!("  {rel}: {}", e.lines().next().unwrap_or(""));
    }
    if results.is_empty() {
        println!("no successful results");
        return;
    }

    let total_gif: u64 = results.iter().map(|r| r.gif_bytes).sum();
    let total_copy: u64 = results.iter().map(|r| r.copy_bytes).sum();
    let ratio = |r: &Done| r.copy_bytes as f64 / r.gif_bytes as f64;
    let mut ratios: Vec<f64> = results.iter().map(ratio).collect();
    ratios.sort_by(f64::total_cmp);
    let mut worst: Vec<&Done> = results.iter().collect();
    worst.sort_by(|a, b| ratio(b).total_cmp(&ratio(a)));

    println!("\n=== SIZE ===");
    println!(
        "total gif bytes:  {total_gif} ({:.1} MB)",
        total_gif as f64 / 1e6
    );
    println!(
        "total copy bytes: {total_copy} ({:.1} MB)",
        total_copy as f64 / 1e6
    );
    println!(
        "overall ratio copy/gif: {:.4}",
        total_copy as f64 / total_gif as f64
    );
    println!(
        "per-file ratio median: {:.4}  p95: {:.4}",
        percentile(&ratios, 50.0),
        percentile(&ratios, 95.0)
    );
    println!("worst 10 files (name, gif_size, copy_size, frames, ratio):");
    for r in worst.iter().take(10) {
        let frames = r
            .stats
            .as_ref()
            .map_or("?".to_string(), |s| s.frames.to_string());
        println!(
            "  {}  gif={}  copy={}  frames={frames}  ratio={:.3}",
            r.rel,
            r.gif_bytes,
            r.copy_bytes,
            ratio(r)
        );
    }

    let encoded: Vec<(&Done, &RecodeStats)> = results
        .iter()
        .filter_map(|r| r.stats.as_ref().map(|s| (r, s)))
        .collect();
    if !encoded.is_empty() {
        println!(
            "\n=== PALETTE{} ===",
            if skipped > 0 {
                " (encoded files only)"
            } else {
                ""
            }
        );
        println!(
            "files needing quantisation (more than 255 colours): {} / {}",
            encoded.iter().filter(|(_, s)| s.quantised).count(),
            encoded.len()
        );
        println!(
            "exactly 256 colours (deltas without a spare transparent index): {}",
            encoded.iter().filter(|(_, s)| s.opaque_deltas).count()
        );
        println!(
            "decoded with the gif crate (interlaced / Previous disposal): {}",
            encoded.iter().filter(|(_, s)| s.full_decoder).count()
        );
        let rect: u64 = encoded.iter().map(|(_, s)| s.rect_px).sum();
        let px: u64 = encoded
            .iter()
            .map(|(_, s)| s.frames as u64 * (s.width * s.height) as u64)
            .sum();
        let clears: u64 = encoded.iter().map(|(_, s)| s.lzw.clears).sum();
        println!(
            "frame rectangles: {:.1}% of all frame pixels; CLEARs forced by the cap: {clears}",
            100.0 * rect as f64 / px.max(1) as f64
        );
        println!("\n=== STRAYS (not 128x32) ===");
        for (r, s) in &encoded {
            if (s.width, s.height) != (128, 32) {
                println!("  {}: {}x{}", r.rel, s.width, s.height);
            }
        }
    }

    let verified: Vec<&Done> = results.iter().filter(|r| r.verify.is_some()).collect();
    if !verified.is_empty() {
        let (mut exact, mut lossy, mut worst_err, mut frames) = (0, 0, 0u8, 0u64);
        let mut fails = Vec::new();
        for r in &verified {
            frames += r.frames_compared;
            match r.verify.as_ref().unwrap() {
                recode::Verdict::Exact => exact += 1,
                recode::Verdict::Lossy { max_err } => {
                    lossy += 1;
                    worst_err = worst_err.max(*max_err);
                }
                recode::Verdict::NeedsFull(e) | recode::Verdict::Fail(e) => {
                    fails.push((r.rel.as_str(), e.as_str()))
                }
            }
        }
        println!(
            "\n=== VERIFICATION (original/GifSource vs copy/GifSource vs copy/GifSlim cap {}, every frame) ===",
            PREPARED.cap
        );
        println!("files verified: {} / {}", verified.len(), results.len());
        println!(
            "identical: {exact}  quantised (frames+delays match, colours approximate): {lossy} \
             (worst channel error {worst_err})  fail: {}",
            fails.len()
        );
        println!("frames compared: {frames}");
        for (rel, e) in fails.iter().take(20) {
            println!("  FAIL {rel}: {e}");
        }
    }

    let smoothed: Vec<(&Done, &Summary)> = results
        .iter()
        .filter_map(|r| r.smooth.as_ref().map(|s| (r, s)))
        .collect();
    if !smoothed.is_empty() {
        println!("\n=== SMOOTH MOTION ===");
        let added: u64 = smoothed.iter().map(|(_, s)| s.added as u64).sum();
        println!(
            "smoothed copies: {} (+{added} in-between frames){}",
            smoothed.len(),
            if verify_note(results) {
                ", verified against recode(tween(GIF))"
            } else {
                ""
            }
        );
        for (r, s) in smoothed.iter().take(20) {
            println!(
                "  {}: {}% of {} pairs, +{} frames",
                r.rel,
                s.percent(),
                s.pairs,
                s.added
            );
        }
    }

    println!("\n=== TIMING ===");
    println!("wall clock for {found} files with {jobs} jobs: {secs:.1}s");
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
    fn convert_writes_the_prepared_copy_where_the_player_looks() {
        let root = std::env::temp_dir().join(format!("opendmd-pack-test-{}", std::process::id()));
        let dir = root.join("gifs/NEW 2021");
        fs::create_dir_all(&dir).unwrap();
        // A two-frame 4x4 GIF.
        let mut gif = Vec::new();
        {
            let pal = [0u8, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];
            let mut enc = gif::Encoder::new(&mut gif, 4, 4, &pal).unwrap();
            for k in 0..2u8 {
                let f = gif::Frame {
                    width: 4,
                    height: 4,
                    buffer: std::borrow::Cow::Owned(vec![k + 1; 16]),
                    delay: 5,
                    ..Default::default()
                };
                enc.write_frame(&f).unwrap();
            }
        }
        fs::write(dir.join("SNES_A_01..gif"), &gif).unwrap();
        convert(&root, Some(1), true, false).unwrap();
        let copy = root.join(layout::cache_rel("NEW 2021/SNES_A_01..gif").unwrap());
        assert!(copy.starts_with(root.join("cache/NEW 2021")));
        let mut want = Vec::new();
        gifslim::recode_gif(&gif[..], &mut want, &PREPARED).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), want);
        // Up to date: carries the GIF's mtime, and a second run leaves it.
        let gif_meta = fs::metadata(dir.join("SNES_A_01..gif")).unwrap();
        assert!(is_current(&copy, &gif_meta));
        convert(&root, Some(1), false, false).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn convert_honours_the_smooth_list() {
        let root = std::env::temp_dir().join(format!("opendmd-pack-smooth-{}", std::process::id()));
        let dir = root.join("gifs/Other");
        fs::create_dir_all(&dir).unwrap();
        // A 2 px-per-frame scroll of a 64-wide stripe pattern, 50 ms a frame.
        let (w, h) = (128u16, 32u16);
        let pal: Vec<u8> = (0..8u8).flat_map(|i| [i * 30, 200 - i * 20, i * 7]).collect();
        let mut gif = Vec::new();
        {
            let mut enc = gif::Encoder::new(&mut gif, w, h, &pal).unwrap();
            for f in 0..8usize {
                let buf: Vec<u8> = (0..h as usize * w as usize)
                    .map(|p| {
                        let (x, y) = (p % 128 + 2 * f, p / 128);
                        ((x * 7 + y * 3 + x * x % 5) % 8) as u8
                    })
                    .collect();
                enc.write_frame(&gif::Frame {
                    width: w,
                    height: h,
                    buffer: std::borrow::Cow::Owned(buf),
                    delay: 5,
                    ..Default::default()
                })
                .unwrap();
            }
        }
        fs::write(dir.join("X.gif"), &gif).unwrap();
        let copy = root.join(layout::cache_rel("Other/X.gif").unwrap());
        // Listed: the smoothed copy, exactly the core's, verified.
        fs::create_dir_all(root.join("sets")).unwrap();
        fs::write(root.join("sets/.smooth"), "/gifs/Other/X.gif\n").unwrap();
        convert(&root, Some(1), true, false).unwrap();
        let mut want = Vec::new();
        tween::smooth_gif(&gif, &mut want, &PREPARED).unwrap();
        let got = fs::read(&copy).unwrap();
        assert!(tween::marker(&got).is_some());
        assert_eq!(got, want);
        // Up to date now; then taken off the list: made plain again.
        convert(&root, Some(1), true, false).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), want);
        fs::write(root.join("sets/.smooth"), "").unwrap();
        convert(&root, Some(1), true, false).unwrap();
        let mut plain = Vec::new();
        gifslim::recode_gif(&gif[..], &mut plain, &PREPARED).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), plain);
        // No list at all: a marked copy stays smooth.
        fs::write(&copy, &want).unwrap();
        fs::remove_file(root.join("sets/.smooth")).unwrap();
        convert(&root, Some(1), true, true).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), want);
        fs::remove_dir_all(&root).unwrap();
    }

}
