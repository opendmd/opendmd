//! The gifslim experiment's card-wide tools (`docs/gifslim-experiment.md`).
//!
//! - `recode <card-root> --cap N --delta on|off [--verify]` writes
//!   `<name>.cap<N>[d].gif` beside every original GIF (never over it) with
//!   [`gifslim::recode_gif`], and with `--verify` plays every frame of each
//!   through three decoders at once — the original through `GifSource`, the
//!   copy through `GifSource` (the `gif` crate: "it is a standard GIF") and
//!   the copy through `GifSlim` at the cap — comparing canvases and delays.
//! - `gifscan <card-root>` plays every *original* GIF through `GifSlim` at
//!   caps 9..12 against `GifSource`: how much of the pack the small decoder
//!   plays with no preparation, and the widest code / longest string it met.

use crate::percentile;
use anyhow::{bail, Result};
use clap::ValueEnum;
use opendmd_core::canvas::Canvas;
use opendmd_core::gifslim::{self, GifSlim, RecodeOptions, RecodeStats, SlimStats};
use opendmd_core::player::GifSource;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

/// Is `name` something `recode` wrote (`X.cap10d.gif`, or `X.<suffix>.gif`)?
/// The walkers skip these so a recode never feeds on its own output.
pub fn is_recode_output(name: &str, extra_suffix: Option<&str>) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".gif") else {
        return false;
    };
    let Some((_, tag)) = stem.rsplit_once('.') else {
        return false;
    };
    if extra_suffix.is_some_and(|s| s.eq_ignore_ascii_case(tag)) {
        return true;
    }
    let Some(rest) = tag.strip_prefix("cap") else {
        return false;
    };
    let digits = rest.strip_suffix('d').unwrap_or(rest);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Every original `.gif` under `dir`, sorted.
pub fn find_sources(dir: &Path, suffix: Option<&str>) -> io::Result<Vec<PathBuf>> {
    fn walk(dir: &Path, suffix: Option<&str>, out: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, suffix, out)?;
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
                && !path
                    .file_name()
                    .is_some_and(|n| is_recode_output(&n.to_string_lossy(), suffix))
            {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut v = Vec::new();
    walk(dir, suffix, &mut v)?;
    v.sort();
    Ok(v)
}

/// Per-file results and per-file errors, by path relative to `gifs/`.
type Collected<T> = (Vec<(String, T)>, Vec<(String, String)>);

/// Run `work` over `files` on `jobs` threads, collecting `Ok`s and errors.
pub(crate) fn parallel<T: Send>(
    files: &[PathBuf],
    jobs: usize,
    root: &Path,
    work: impl Fn(&Path, &str) -> Result<T> + Sync,
) -> Collected<T> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(files.len()));
    let errors = Mutex::new(Vec::new());
    let t0 = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = files.get(i) else { break };
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned();
                match work(path, &rel) {
                    Ok(t) => results.lock().unwrap().push((rel, t)),
                    Err(e) => errors.lock().unwrap().push((rel, format!("{e:#}"))),
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(1000) {
                    eprintln!(
                        "  {n}/{} done, {:.1}s",
                        files.len(),
                        t0.elapsed().as_secs_f64()
                    );
                }
            });
        }
    });
    let mut r = results.into_inner().unwrap();
    r.sort_by(|a, b| a.0.cmp(&b.0));
    let mut e = errors.into_inner().unwrap();
    e.sort();
    (r, e)
}

pub(crate) fn jobs_or_all(jobs: Option<usize>) -> usize {
    jobs.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
        .max(1)
}

// ---------------------------------------------------------------------------
// recode
// ---------------------------------------------------------------------------

/// How the copy compared, when verified.
#[derive(Debug)]
pub enum Verdict {
    /// Original, `gif` crate on the copy and gifslim on the copy: every
    /// canvas and delay identical.
    Exact,
    /// gifslim refused the copy at its cap (must never happen).
    NeedsFull(String),
    Fail(String),
}

struct Recoded {
    gif_bytes: u64,
    stats: RecodeStats,
    verdict: Option<Verdict>,
    frames_compared: u64,
    slim: SlimStats,
}

#[allow(clippy::too_many_arguments)]
pub fn recode(
    root: &Path,
    cap: u8,
    delta: OnOff,
    out_suffix: Option<String>,
    jobs: Option<usize>,
    verify: bool,
    hint: bool,
    keep_runs: bool,
    verify_only: bool,
    tsv: Option<PathBuf>,
) -> Result<()> {
    if !(gifslim::MIN_CAP..=gifslim::MAX_CAP).contains(&cap) {
        bail!("--cap must be {}..={}", gifslim::MIN_CAP, gifslim::MAX_CAP);
    }
    let gif_root = root.join("gifs");
    if !gif_root.is_dir() {
        bail!("{}: no gifs/ directory", root.display());
    }
    let delta = delta == OnOff::On;
    let suffix = out_suffix.unwrap_or_else(|| format!("cap{cap}{}", if delta { "d" } else { "" }));
    let files = find_sources(&gif_root, Some(&suffix))?;
    let jobs = jobs_or_all(jobs);
    println!(
        "recode: {} GIFs under {}, cap {cap}, delta {}, -> *.{suffix}.gif ({jobs} jobs{})",
        files.len(),
        gif_root.display(),
        if delta { "on" } else { "off" },
        if verify || verify_only {
            ", verifying"
        } else {
            ""
        }
    );
    let opts = RecodeOptions {
        cap,
        delta,
        hint,
        keep_runs,
    };
    let t0 = Instant::now();
    let (results, errors) = parallel(&files, jobs, &gif_root, |path, _| {
        let gif = fs::read(path)?;
        let dst = out_path(path, &suffix);
        let (out, stats) = if verify_only {
            // The copy on disk, as a previous run wrote it.
            let out = fs::read(&dst)?;
            let stats = RecodeStats {
                gif_bytes: gif.len() as u64,
                out_bytes: out.len() as u64,
                ..Default::default()
            };
            (out, stats)
        } else {
            let mut out = Vec::with_capacity(gif.len());
            let stats = gifslim::recode_gif(&gif[..], &mut out, &opts)?;
            let tmp = dst.with_extension("gif.tmp");
            {
                let mut f = fs::File::create(&tmp)?;
                f.write_all(&out)?;
                if let Ok(t) = fs::metadata(path).and_then(|m| m.modified()) {
                    f.set_modified(t)?;
                }
            }
            fs::rename(&tmp, &dst)?;
            (out, stats)
        };
        let (verdict, frames_compared, slim) = if verify || verify_only {
            let (v, n, s) = compare(&gif, &out, cap);
            (Some(v), n, s)
        } else {
            (None, 0, SlimStats::default())
        };
        Ok(Recoded {
            gif_bytes: gif.len() as u64,
            stats,
            verdict,
            frames_compared,
            slim,
        })
    });
    let secs = t0.elapsed().as_secs_f64();
    report(&results, &errors, cap, delta, jobs, secs);
    if let Some(tsv) = tsv {
        let mut f = io::BufWriter::new(fs::File::create(&tsv)?);
        writeln!(
            f,
            "rel\tgif\tout\tframes\tcolours\topaque\trect_px\tclears\tmax_string\tverdict"
        )?;
        for (rel, r) in &results {
            writeln!(
                f,
                "{rel}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.gif_bytes,
                r.stats.out_bytes,
                r.stats.frames,
                r.stats.colours,
                r.stats.opaque_deltas as u8,
                r.stats.rect_px,
                r.stats.lzw.clears,
                r.stats.lzw.max_string,
                match &r.verdict {
                    None => "-".to_string(),
                    Some(Verdict::Exact) => "exact".into(),
                    Some(Verdict::NeedsFull(e)) => format!("needsfull:{e}"),
                    Some(Verdict::Fail(e)) => format!("FAIL:{e}"),
                }
            )?;
        }
        println!("per-file table: {}", tsv.display());
    }
    let bad = results
        .iter()
        .filter(|(_, r)| matches!(r.verdict, Some(Verdict::Fail(_) | Verdict::NeedsFull(_))))
        .count();
    if bad > 0 {
        bail!("{bad} files failed verification");
    }
    Ok(())
}

fn out_path(gif: &Path, suffix: &str) -> PathBuf {
    let stem = gif.file_stem().unwrap_or_default().to_string_lossy();
    gif.with_file_name(format!("{stem}.{suffix}.gif"))
}

/// Play the original and the copy (twice) in lockstep: the original
/// through `GifSource`, the copy through `GifSource` and through `GifSlim`
/// at `cap`. Returns the verdict, the frames compared and gifslim's stats.
/// Copies are exact or absent: any pixel that differs is a failure.
pub fn compare(orig: &[u8], copy: &[u8], cap: u8) -> (Verdict, u64, SlimStats) {
    let run = || -> Result<(Verdict, u64, SlimStats)> {
        let mut a = GifSource::new(orig)?;
        let mut b = GifSource::new(copy)?;
        let mut c = GifSlim::with_cap(copy, cap)?;
        if a.logical_size() != b.logical_size() || b.logical_size() != c.logical_size() {
            return Ok((Verdict::Fail("logical size".into()), 0, c.stats()));
        }
        let (mut ca, mut cb, mut cc) = (Canvas::new(), Canvas::new(), Canvas::new());
        let mut frames = 0u64;
        let mut max_err = 0u8;
        loop {
            let da = a.next_frame(&mut ca)?;
            let db = b.next_frame(&mut cb)?;
            let dc = match c.next_frame(&mut cc) {
                Ok(d) => d,
                Err(e) if gifslim::is_needs_full_decoder(&e) => {
                    return Ok((
                        Verdict::NeedsFull(format!("frame {frames}")),
                        frames,
                        c.stats(),
                    ))
                }
                Err(e) => {
                    return Ok((
                        Verdict::Fail(format!("gifslim frame {frames}: {e}")),
                        frames,
                        c.stats(),
                    ))
                }
            };
            if da != db || db != dc {
                return Ok((
                    Verdict::Fail(format!("frame {frames}: delays {da:?} / {db:?} / {dc:?}")),
                    frames,
                    c.stats(),
                ));
            }
            if da.is_none() {
                break;
            }
            if cb.as_bytes() != cc.as_bytes() {
                return Ok((
                    Verdict::Fail(format!("frame {frames}: gif crate and gifslim differ")),
                    frames,
                    c.stats(),
                ));
            }
            for (x, y) in ca.as_bytes().iter().zip(cb.as_bytes()) {
                max_err = max_err.max(x.abs_diff(*y));
            }
            frames += 1;
        }
        let v = match max_err {
            0 => Verdict::Exact,
            e => Verdict::Fail(format!("pixels differ from the original by up to {e}")),
        };
        Ok((v, frames, c.stats()))
    };
    run().unwrap_or_else(|e| (Verdict::Fail(format!("{e:#}")), 0, SlimStats::default()))
}

fn report(
    results: &[(String, Recoded)],
    errors: &[(String, String)],
    cap: u8,
    delta: bool,
    jobs: usize,
    secs: f64,
) {
    println!(
        "\n=== RECODE cap {cap} delta {} : {} ok, {} errors, {secs:.1}s on {jobs} jobs ===",
        if delta { "on" } else { "off" },
        results.len(),
        errors.len()
    );
    for (rel, e) in errors {
        println!("  error {rel}: {}", e.lines().next().unwrap_or(""));
    }
    if results.is_empty() {
        return;
    }
    let gif: u64 = results.iter().map(|r| r.1.gif_bytes).sum();
    let out: u64 = results.iter().map(|r| r.1.stats.out_bytes).sum();
    let ratio = |r: &Recoded| r.stats.out_bytes as f64 / r.gif_bytes as f64;
    let mut ratios: Vec<f64> = results.iter().map(|r| ratio(&r.1)).collect();
    ratios.sort_by(f64::total_cmp);
    println!("originals: {gif} B ({:.1} MB)", gif as f64 / 1e6);
    println!(
        "recoded:   {out} B ({:.1} MB)  ratio to originals {:.4}  (per file median {:.4} p95 {:.4})",
        out as f64 / 1e6,
        out as f64 / gif as f64,
        percentile(&ratios, 50.0),
        percentile(&ratios, 95.0)
    );
    let mut worst: Vec<&(String, Recoded)> = results.iter().collect();
    worst.sort_by(|a, b| ratio(&b.1).total_cmp(&ratio(&a.1)));
    println!("worst 10 (recoded / original):");
    for (rel, r) in worst.iter().take(10) {
        println!(
            "  {rel}  gif={} out={} frames={} ratio={:.3}",
            r.gif_bytes,
            r.stats.out_bytes,
            r.stats.frames,
            ratio(r)
        );
    }
    let o = results.iter().filter(|r| r.1.stats.opaque_deltas).count();
    let of: u64 = results.iter().map(|r| r.1.stats.opaque_frames).sum();
    let clears: u64 = results.iter().map(|r| r.1.stats.lzw.clears).sum();
    let codes: u64 = results.iter().map(|r| r.1.stats.lzw.codes).sum();
    let max_string = results
        .iter()
        .map(|r| r.1.stats.lzw.max_string)
        .max()
        .unwrap_or(0);
    let rect: u64 = results.iter().map(|r| r.1.stats.rect_px).sum();
    let px: u64 = results
        .iter()
        .map(|r| r.1.stats.frames as u64 * (r.1.stats.width * r.1.stats.height) as u64)
        .sum();
    println!(
        "exactly 256 colours with deltas: {o} (frames with no spare index: {of})  frame-rect pixels {:.1}% of all  \
         codes {codes}  extra CLEARs {clears}  longest string {max_string}",
        100.0 * rect as f64 / px.max(1) as f64
    );

    let verified: Vec<&(String, Recoded)> =
        results.iter().filter(|r| r.1.verdict.is_some()).collect();
    if verified.is_empty() {
        return;
    }
    let (mut exact, mut needs, mut fails, mut frames) = (0, Vec::new(), Vec::new(), 0u64);
    let mut slim_max_string = 0;
    let mut slim_long = 0u64;
    let mut slim_codes = 0u64;
    let mut slim_width = 0u8;
    for (rel, r) in &verified {
        frames += r.frames_compared;
        slim_max_string = slim_max_string.max(r.slim.max_string);
        slim_long += r.slim.long_strings;
        slim_codes += r.slim.codes;
        slim_width = slim_width.max(r.slim.max_width);
        match r.verdict.as_ref().unwrap() {
            Verdict::Exact => exact += 1,
            Verdict::NeedsFull(e) => needs.push((rel.as_str(), e.as_str())),
            Verdict::Fail(e) => fails.push((rel.as_str(), e.as_str())),
        }
    }
    println!(
        "verify (original/GifSource vs copy/GifSource vs copy/GifSlim cap {cap}): {} files, {frames} frames: \
         exact {exact}, NeedsFullDecoder {}, FAIL {}",
        verified.len(),
        needs.len(),
        fails.len()
    );
    println!(
        "gifslim on the copies: widest code {slim_width} bits, longest string {slim_max_string}, \
         strings past the 96 B walk buffer: {slim_long} of {slim_codes} codes"
    );
    for (rel, e) in needs.iter().chain(fails.iter()).take(30) {
        println!("  BAD {rel}: {e}");
    }
}

// ---------------------------------------------------------------------------
// gifscan: the originals, unprepared
// ---------------------------------------------------------------------------

const SCAN_CAPS: [u8; 4] = [9, 10, 11, 12];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// Every frame identical to GifSource's.
    Plays,
    NeedsFull,
    Unsupported(String),
    /// Both decoders stop with an error at the same frame (the file is
    /// damaged), after identical frames.
    BothFail,
    Mismatch(String),
}

struct Scanned {
    at: [Outcome; 4],
    /// At cap 12: widest code read / longest string.
    stats: SlimStats,
}

fn scan_one(gif: &[u8], cap: u8) -> (Outcome, SlimStats) {
    let mut a = match GifSource::new(gif) {
        Ok(a) => a,
        Err(_) => {
            return match GifSlim::with_cap(gif, cap) {
                Err(_) => (Outcome::BothFail, SlimStats::default()),
                Ok(_) => (
                    Outcome::Mismatch("gif crate cannot open it".into()),
                    SlimStats::default(),
                ),
            }
        }
    };
    let mut b = match GifSlim::with_cap(gif, cap) {
        Ok(b) => b,
        Err(e) => {
            return (
                Outcome::Mismatch(format!("open: {e}")),
                SlimStats::default(),
            )
        }
    };
    let (mut ca, mut cb) = (Canvas::new(), Canvas::new());
    let mut frame = 0u64;
    loop {
        let ra = a.next_frame(&mut ca);
        let rb = b.next_frame(&mut cb);
        let out = match (ra, rb) {
            (_, Err(e)) if gifslim::is_needs_full_decoder(&e) => Some(Outcome::NeedsFull),
            (_, Err(e)) if gifslim::is_unsupported(&e) => Some(Outcome::Unsupported(
                e.get_ref().map(|i| i.to_string()).unwrap_or_default(),
            )),
            (Err(_), Err(_)) => Some(Outcome::BothFail),
            (Err(e), Ok(_)) => Some(Outcome::Mismatch(format!(
                "frame {frame}: only gif crate fails: {e}"
            ))),
            (Ok(_), Err(e)) => Some(Outcome::Mismatch(format!(
                "frame {frame}: only gifslim fails: {e}"
            ))),
            (Ok(da), Ok(db)) if da != db => Some(Outcome::Mismatch(format!(
                "frame {frame}: delay {da:?} vs {db:?}"
            ))),
            (Ok(None), Ok(None)) => Some(Outcome::Plays),
            (Ok(_), Ok(_)) if ca.as_bytes() != cb.as_bytes() => {
                Some(Outcome::Mismatch(format!("frame {frame}: pixels differ")))
            }
            _ => None,
        };
        if let Some(o) = out {
            return (o, b.stats());
        }
        frame += 1;
    }
}

pub fn gifscan(root: &Path, jobs: Option<usize>, tsv: Option<PathBuf>) -> Result<()> {
    let gif_root = root.join("gifs");
    let files = find_sources(&gif_root, None)?;
    let jobs = jobs_or_all(jobs);
    println!(
        "gifscan: {} original GIFs, caps {SCAN_CAPS:?}, {jobs} jobs",
        files.len()
    );
    let t0 = Instant::now();
    let (results, errors) = parallel(&files, jobs, &gif_root, |path, _| {
        let gif = fs::read(path)?;
        let mut at: [Outcome; 4] = std::array::from_fn(|_| Outcome::Plays);
        let mut stats = SlimStats::default();
        for (i, &cap) in SCAN_CAPS.iter().enumerate() {
            let (o, s) = scan_one(&gif, cap);
            at[i] = o;
            if cap == 12 {
                stats = s;
            }
        }
        Ok(Scanned { at, stats })
    });
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "\n=== GIFSCAN: {} files, {} read errors, {secs:.1}s ===",
        results.len(),
        errors.len()
    );
    for (i, &cap) in SCAN_CAPS.iter().enumerate() {
        let count = |f: &dyn Fn(&Outcome) -> bool| results.iter().filter(|r| f(&r.1.at[i])).count();
        println!(
            "cap {cap:>2}: plays identically {:>5}  NeedsFullDecoder {:>5}  unsupported {:>4}  \
             damaged (both fail) {:>3}  MISMATCH {}",
            count(&|o| *o == Outcome::Plays),
            count(&|o| *o == Outcome::NeedsFull),
            count(&|o| matches!(o, Outcome::Unsupported(_))),
            count(&|o| *o == Outcome::BothFail),
            count(&|o| matches!(o, Outcome::Mismatch(_))),
        );
    }
    let mut reasons: Vec<(String, usize)> = Vec::new();
    for r in &results {
        if let Outcome::Unsupported(u) = &r.1.at[3] {
            match reasons.iter_mut().find(|x| &x.0 == u) {
                Some(x) => x.1 += 1,
                None => reasons.push((u.clone(), 1)),
            }
        }
    }
    for (u, n) in &reasons {
        println!("  unsupported at cap 12: {u}: {n}");
    }
    // Consistency: at cap N a file plays iff it plays at 12 and its widest
    // code is <= N.
    let mut widths = [0usize; 13];
    let mut inconsistent = 0;
    for r in &results {
        if r.1.at[3] == Outcome::Plays {
            widths[r.1.stats.max_width as usize] += 1;
            for (i, &cap) in SCAN_CAPS.iter().enumerate() {
                let predicted = r.1.stats.max_width <= cap;
                if predicted != (r.1.at[i] == Outcome::Plays) {
                    inconsistent += 1;
                }
            }
        }
    }
    println!("widest code needed, over files gifslim plays at cap 12:");
    for (w, n) in widths.iter().enumerate().filter(|x| *x.1 > 0) {
        println!("  {w:>2} bits: {n}");
    }
    let longest = results
        .iter()
        .map(|r| r.1.stats.max_string)
        .max()
        .unwrap_or(0);
    println!("longest LZW string in any original: {longest}; cap/width predicate inconsistencies: {inconsistent}");
    for (rel, r) in results
        .iter()
        .filter(|r| matches!(r.1.at[3], Outcome::Mismatch(_)))
        .take(20)
    {
        println!("  MISMATCH {rel}: {:?}", r.at[3]);
    }
    for (rel, e) in &errors {
        println!("  error {rel}: {e}");
    }
    if let Some(tsv) = tsv {
        let mut f = io::BufWriter::new(fs::File::create(&tsv)?);
        writeln!(f, "rel\tmax_width\tmax_string\tcap9\tcap10\tcap11\tcap12")?;
        for (rel, r) in &results {
            let o = |o: &Outcome| match o {
                Outcome::Plays => "plays".to_string(),
                Outcome::NeedsFull => "needsfull".into(),
                Outcome::Unsupported(u) => format!("unsupported:{u}"),
                Outcome::BothFail => "damaged".into(),
                Outcome::Mismatch(m) => format!("MISMATCH:{m}"),
            };
            writeln!(
                f,
                "{rel}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.stats.max_width,
                r.stats.max_string,
                o(&r.at[0]),
                o(&r.at[1]),
                o(&r.at[2]),
                o(&r.at[3])
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recode_outputs_are_recognised_and_originals_are_not() {
        for yes in ["X.cap10d.gif", "X_01..cap9.GIF", "a.b.cap12.gif"] {
            assert!(is_recode_output(yes, None), "{yes}");
        }
        for no in [
            "X.gif",
            "X_cap10.gif",
            "X.capd.gif",
            "X.cap.gif",
            "cap10.gif",
            "X.bin",
        ] {
            assert!(!is_recode_output(no, None), "{no}");
        }
        assert!(is_recode_output("X.slim.gif", Some("slim")));
        assert_eq!(
            out_path(Path::new("/c/gifs/A/X_01..gif"), "cap10d"),
            PathBuf::from("/c/gifs/A/X_01..cap10d.gif")
        );
    }
}
