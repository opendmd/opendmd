//! Integration test against a real DMDoS card dump.
//!
//! Ignored by default — it needs the (third-party, not-redistributable) GIF
//! pack on disk:
//!
//! ```sh
//! DMD_CARD_ROOT=~/scratch/dmdos-card cargo test -- --ignored --nocapture
//! ```

use opendmd_core::canvas::Canvas;
use opendmd_core::engine::{
    Catalog, DwellPolicy, Engine, FileCatalog, ReadSeek, Source, TickOutcome,
};
use opendmd_core::filter::Filter;
use opendmd_core::index::{write_index, IndexHeader, IndexedCatalog};
use opendmd_core::manifest::{parse_manifest, render_manifest};
use opendmd_core::scheduler::{Mode, Rng, Scheduler};
use std::collections::BTreeMap;
use std::io::{self, BufReader};
use std::path::PathBuf;
use std::time::Duration;

/// Joins card-absolute manifest paths (`/gifs/...`) onto a root directory.
struct DirSource {
    root: PathBuf,
}

impl Source for DirSource {
    fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>> {
        let rel = path.trim_start_matches('/');
        Ok(Box::new(std::fs::File::open(self.root.join(rel))?))
    }
}

#[test]
#[ignore = "needs DMD_CARD_ROOT pointing at a real card dump"]
fn real_card_manifest() {
    let Ok(root) = std::env::var("DMD_CARD_ROOT") else {
        eprintln!("DMD_CARD_ROOT not set, nothing to do");
        return;
    };
    let root = PathBuf::from(root);
    let text = std::fs::read_to_string(root.join("filelist.txt")).expect("read filelist.txt");
    let entries = parse_manifest(&text);
    println!("manifest entries: {}", entries.len());
    assert!(entries.len() > 1000, "suspiciously small manifest");

    // Round-trip must be byte-exact against the shipped file.
    assert_eq!(render_manifest(&entries), text);

    for e in &entries {
        assert!(!e.title.trim().is_empty(), "empty title for {}", e.path);
        assert!(e.path.starts_with('/'), "path lost its slash: {}", e.path);
    }

    let mut by_platform: BTreeMap<String, usize> = BTreeMap::new();
    for e in &entries {
        *by_platform
            .entry(e.platform.clone().unwrap_or_else(|| "(none)".into()))
            .or_default() += 1;
    }
    println!("\nper-platform counts:");
    let mut rows: Vec<_> = by_platform.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (platform, count) in &rows {
        println!("  {count:>6}  {platform}");
    }

    let consoles: Vec<_> = entries
        .iter()
        .filter(|e| e.category.as_deref() == Some("Consoles"))
        .collect();
    let with_platform = consoles.iter().filter(|e| e.platform.is_some()).count();
    let pct = 100.0 * with_platform as f64 / consoles.len() as f64;
    println!(
        "\ngifs/Consoles: {with_platform}/{} got a platform ({pct:.2}%)",
        consoles.len()
    );
    for e in consoles.iter().filter(|e| e.platform.is_none()).take(10) {
        println!("  no platform: {}", e.path);
    }
    assert!(
        pct >= 95.0,
        "only {pct:.2}% of Consoles entries got a platform"
    );

    // The streaming index must agree exactly with parse + filter, and read
    // back the same entries. This is the path the firmware actually uses.
    let tmp = std::env::temp_dir().join(format!("opendmd-real-card-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("temp dir");
    for (label, filter) in [
        ("empty filter", Filter::new()),
        ("MD,SNES", Filter::new().platforms(["MD", "SNES"])),
    ] {
        let want: Vec<&opendmd_core::Entry> = filter
            .apply(&entries)
            .iter()
            .map(|i| &entries[*i])
            .collect();
        let file = std::fs::File::open(root.join("filelist.txt")).expect("open filelist.txt");
        let mut catalog = FileCatalog::build(file, &filter).expect("build catalog");
        println!(
            "\nFileCatalog[{label}]: {} entries, {} index bytes ({} for parse_manifest's Vec<Entry>)",
            catalog.len(),
            catalog.offsets().len() * 4,
            want.len() * 590,
        );
        assert_eq!(catalog.len(), want.len(), "catalog length for {label}");
        assert!(!want.is_empty(), "{label} matched nothing");

        // Spot-check 20 random offsets against the parsed vector.
        let mut rng = Rng::new(0x4F50_454E_444D_4400);
        for _ in 0..20 {
            let i = (rng.next_u64() % catalog.len() as u64) as usize;
            assert_eq!(
                &catalog.entry(i).expect("entry_at"),
                want[i],
                "entry {i} of {label}"
            );
        }

        // The on-card index (what the firmware plays from now): built into a
        // temp dir with `write_index`, read back through `IndexedCatalog`, and
        // held against the in-RAM catalog it replaces.
        let manifest_path = root.join("filelist.txt");
        let manifest_len = std::fs::metadata(&manifest_path).unwrap().len() as u32;
        let idx_path = tmp.join(format!("filelist.{}.idx", label.replace([' ', ','], "_")));
        let started = std::time::Instant::now();
        let header = {
            let manifest = std::fs::File::open(&manifest_path).unwrap();
            let out = std::fs::File::create(&idx_path).expect("create .idx");
            write_index(
                BufReader::with_capacity(1024, manifest),
                manifest_len,
                &filter,
                out,
            )
            .expect("write_index")
        };
        let idx_bytes = std::fs::metadata(&idx_path).unwrap().len();
        println!(
            "IndexedCatalog[{label}]: {} entries, {idx_bytes} B on disk, built in {:?}",
            header.count,
            started.elapsed()
        );
        assert_eq!(idx_bytes, header.file_len());
        assert_eq!(
            IndexHeader::read(&mut std::fs::File::open(&idx_path).unwrap()).unwrap(),
            header
        );
        assert!(header.is_valid_for(manifest_len, &filter, opendmd_core::overrides::NO_META));
        let mut indexed = IndexedCatalog::open(
            std::fs::File::open(&manifest_path).unwrap(),
            std::fs::File::open(&idx_path).unwrap(),
        )
        .expect("open IndexedCatalog");
        assert_eq!(indexed.len(), catalog.len(), "indexed length for {label}");
        for (i, off) in catalog.offsets().iter().enumerate() {
            assert_eq!(indexed.offset(i).unwrap(), *off, "offset {i} of {label}");
        }
        let mut rng = Rng::new(0x4F44_4D58_0000_0001);
        for _ in 0..50 {
            let i = (rng.next_u64() % catalog.len() as u64) as usize;
            assert_eq!(
                indexed.entry(i).expect("indexed entry"),
                catalog.entry(i).expect("in-RAM entry"),
                "indexed entry {i} of {label}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);

    // Open the first 50 GIFs through the engine and pull one frame each.
    let file = std::fs::File::open(root.join("filelist.txt")).expect("open filelist.txt");
    let catalog = FileCatalog::build(file, &Filter::new()).expect("build catalog");
    let scheduler = Scheduler::new(catalog.len().min(50), Mode::Sequential, 0);
    let dwell = DwellPolicy {
        min: Duration::ZERO,
        max: Duration::from_secs(30),
        min_loops: 1,
    };
    let mut engine = Engine::new(
        Box::new(catalog),
        scheduler,
        dwell,
        Box::new(DirSource { root }),
    );

    let mut canvas = Canvas::new();
    let mut decoded = 0usize;
    let mut seen = std::collections::HashSet::new();
    let t0 = Duration::ZERO;
    // Each GIF yields one frame then is skipped, so 50 skips == 50 files.
    for step in 0..50 {
        let now = t0 + Duration::from_secs(step as u64 * 60);
        match engine.tick(now, &mut canvas).expect("engine tick") {
            TickOutcome::Frame { playing, .. } => {
                decoded += 1;
                seen.insert(playing.path.clone());
            }
            TickOutcome::Idle => {}
            TickOutcome::Exhausted => panic!("ran out of playable entries at step {step}"),
        }
        engine.skip();
    }
    println!(
        "\ndecoded a frame from {decoded} ticks, {} distinct files",
        seen.len()
    );
    assert_eq!(decoded, 50);
    assert_eq!(seen.len(), 50);
}

/// Cross-implementation check: `.odm` files written by the Python prototype
/// (`tools/odm-proto.py`, output under `DMD_ODM_ROOT` mirroring `gifs/`)
/// decode through the Rust `OdmSource` to exactly the canvases `GifSource`
/// produces from the source GIF.
///
/// Twenty files: the seven the heap budget is measured on, the two strays
/// (132x32 and 135x32), and eleven more picked by a fixed-seed draw. Files
/// the prototype had to quantise are lossy by design and skipped by the draw.
/// Delays are compared after the GIF player's rules (0 -> 100 ms, floor
/// 20 ms), because the prototype wrote Pillow's raw `duration` where the Rust
/// encoder writes the final delay.
/// The one known disagreement. This 135x32 GIF's frames never cover source
/// columns 128..=130 (canvas 125..=127 after the -3 crop offset); `GifSource`
/// leaves never-drawn pixels black, as it always has, while Pillow starts the
/// screen at the GIF's background colour, so the prototype encoded that
/// strip as (107,41,57). The Rust encoder follows playback and differs from
/// the prototype here by 216 bytes.
const WARIO: &str = "Consoles/NINTENDO_DS_Wario_WareDIY_03";

fn assert_uncovered_only(g: &Canvas, o: &Canvas, rel: &str, frame: u64) {
    for y in 0..32 {
        for x in 0..128 {
            if g.get(x, y) != o.get(x, y) {
                assert!(
                    x >= 125 && g.get(x, y) == opendmd_core::Rgb::BLACK,
                    "{rel}: frame {frame} differs at ({x},{y}) outside the uncovered strip"
                );
            }
        }
    }
}

#[test]
#[ignore = "needs DMD_CARD_ROOT and DMD_ODM_ROOT (the prototype's output)"]
fn prototype_odm_files_decode_like_the_gifs() {
    use opendmd_core::odm::OdmSource;
    use opendmd_core::player::GifSource;

    let (Ok(card), Ok(odm_root)) = (
        std::env::var("DMD_CARD_ROOT"),
        std::env::var("DMD_ODM_ROOT"),
    ) else {
        eprintln!("DMD_CARD_ROOT / DMD_ODM_ROOT not set, nothing to do");
        return;
    };
    let gifs = PathBuf::from(card).join("gifs");
    let odm_root = PathBuf::from(odm_root);

    let mut picks: Vec<String> = [
        "Pinball_Short/PINBALL_SHORT_Corvette_01",
        "Pinball_Short/PINBALL_SHORT_WWF_40",
        "Consoles/SNES_Inspector_Gadget_02",
        "Arcade/ARCADE_Arkanoid_03",
        "Arcade/ARCADE_NEOGEO_MetalSlugFire05_Shabazz",
        "Consoles/SNES_Donkey_Kong_Country_04",
        "Consoles/SNES_Yoshi_Island_01_01_Wolfsoft",
        "Consoles/NINTENDO_DS_Wario_WareDIY_03",
        "NEW 2021/2a",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    // Every prototype output, sorted, for a reproducible draw.
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if p.extension().is_some_and(|x| x == "odm") {
                let rel = p.strip_prefix(base).unwrap().with_extension("");
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    let mut all = Vec::new();
    walk(&odm_root, &odm_root, &mut all);
    all.sort();
    println!(
        "prototype .odm files under {}: {}",
        odm_root.display(),
        all.len()
    );
    let mut rng = Rng::new(0x4F44_4D31_0000_0020);
    while picks.len() < 20 {
        let rel = &all[(rng.next_u64() % all.len() as u64) as usize];
        if picks.contains(rel) {
            continue;
        }
        // Skip the prototype's quantised files: those are lossy by design.
        let odm = std::fs::File::open(odm_root.join(format!("{rel}.odm"))).unwrap();
        if OdmSource::new(BufReader::new(odm)).unwrap().palette_len() == 256 {
            continue;
        }
        picks.push(rel.clone());
    }

    let rule = |ms: u128| match ms {
        0 => 100,
        n => n.max(20),
    };
    let mut total_frames = 0u64;
    for rel in &picks {
        let gif = std::fs::read(gifs.join(format!("{rel}.gif"))).unwrap();
        let odm = std::fs::File::open(odm_root.join(format!("{rel}.odm"))).unwrap();
        let mut g = GifSource::new(&gif[..]).unwrap();
        let mut o = OdmSource::new(BufReader::new(odm)).unwrap();
        assert_eq!(g.logical_size(), o.logical_size(), "{rel}: size");
        let (mut cg, mut co) = (Canvas::new(), Canvas::new());
        let mut frames = 0u64;
        let mut delay_diffs = 0u64;
        loop {
            let dg = g.next_frame(&mut cg).unwrap();
            let d_o = o.next_frame(&mut co).unwrap();
            match (dg, d_o) {
                (None, None) => break,
                (Some(dg), Some(d_o)) => {
                    if rel == WARIO {
                        assert_uncovered_only(&cg, &co, rel, frames);
                    } else {
                        assert!(cg == co, "{rel}: frame {frames} differs");
                    }
                    if dg.as_millis() != rule(d_o.as_millis()) {
                        delay_diffs += 1;
                    }
                }
                _ => panic!("{rel}: frame count differs at {frames}"),
            }
            frames += 1;
        }
        println!(
            "  ok  {rel}: {frames} frames, {}x{}, palette {}, delay mismatches {delay_diffs}",
            o.logical_size().0,
            o.logical_size().1,
            o.palette_len()
        );
        assert_eq!(delay_diffs, 0, "{rel}: delays");
        total_frames += frames;
    }
    println!(
        "{} prototype files, {total_frames} frames, pixel-identical to GifSource (bar {WARIO}'s uncovered strip)",
        picks.len()
    );
}

/// What "Follow RePlayOS" would build for a few games on this card: the
/// platform's entries, how many score as this game (strong) or related,
/// the weights those counts earn (`follow::ContextWeights`), and the logo it
/// would show. For Sonic 3 and Super Mario World it asserts that the
/// matches (strong + related) get 60% of the draws, by the weights' model
/// and by 60,000 draws of the scheduler over the index read back with the
/// context's no-repeat window (`follow::context_window`) — what the panel
/// plays. For every game, the draws match the model within 3 points.
#[test]
#[ignore = "needs DMD_CARD_ROOT pointing at a real card dump"]
fn follow_context_on_the_real_card() {
    use opendmd_core::follow::{self, Match};
    use opendmd_core::index::write_index_resolving_then;
    let Ok(root) = std::env::var("DMD_CARD_ROOT") else {
        eprintln!("DMD_CARD_ROOT not set, nothing to do");
        return;
    };
    let path = PathBuf::from(root).join("filelist.txt");
    let len = std::fs::metadata(&path).unwrap().len() as u32;
    let near = |got: f32, want: f32, what: &str| {
        assert!(
            (got - want).abs() <= 0.03,
            "{what}: {got:.3}, want {want:.3} ± 0.03"
        );
    };
    for (system, game, check) in [
        ("sega_smd", "Sonic the Hedgehog 3 (USA)", true),
        ("nintendo_snes", "Super Mario World (USA)", true),
        ("nintendo_snes", "Donkey Kong Country (USA)", false),
        ("sega_smd", "Streets of Rage 2 (USA)", false),
    ] {
        let platform = follow::platform_for_system(system).unwrap();
        let filter = Filter::new().platforms([platform]);
        let game_tokens = follow::tokens(game);
        let (mut strong, mut weak, mut logos) = (Vec::new(), Vec::new(), Vec::new());
        let mut weights = None;
        let mut out = io::Cursor::new(Vec::new());
        let file = std::fs::File::open(&path).unwrap();
        let (header, ov) = write_index_resolving_then(
            BufReader::new(file),
            len,
            &filter,
            follow::context_fingerprint(platform, game),
            |e| {
                if follow::is_logo(e) {
                    logos.push((e.title.clone(), e.path.clone()));
                }
                let m = follow::classify(follow::score_tokens(
                    &follow::tokens(&e.title),
                    &game_tokens,
                ))?;
                match m {
                    Match::Strong => strong.push(e.title.clone()),
                    Match::Related => weak.push(e.title.clone()),
                }
                Some(m.mark())
            },
            |count, ov| weights = Some(follow::weigh_context(count, ov)),
            &mut out,
        )
        .unwrap();
        let w = weights.unwrap();
        let logo = follow::choose_logo(&logos, game);
        weak.truncate(20);
        let (s, r, o) = w.shares();
        println!(
            "{game} ({platform}): {} entries, {} weighted, {} strong {:?}, related e.g. {weak:?}, logo {logo:?} of {}\n  \
             window {}, weights strong {} related {}{}; shares by the model {:.1}% / {:.1}% / {:.1}%",
            header.count,
            ov.len(),
            strong.len(),
            strong,
            logos.len(),
            w.window,
            w.strong,
            w.related,
            if w.capped { " (capped: 60% out of reach)" } else { "" },
            s * 100.0,
            r * 100.0,
            o * 100.0,
        );
        assert_eq!(w.n_strong as usize, strong.len());
        let file = std::fs::File::open(&path).unwrap();
        let mut cat = IndexedCatalog::open(file, io::Cursor::new(out.into_inner())).unwrap();
        assert_eq!(cat.len(), header.count as usize);
        let read_back = cat.overrides().unwrap();
        assert_eq!(read_back.len(), ov.len());
        assert!(cat.header().is_valid_for(
            len,
            &filter,
            follow::context_fingerprint(platform, game)
        ));
        let class: Vec<Option<Match>> = (0..cat.len())
            .map(|i| follow::classify(follow::title_score(&cat.entry(i).unwrap().title, game)))
            .collect();
        let draw = |window: usize| {
            let mut sched = Scheduler::new(cat.len(), Mode::Shuffle, 0x5eed).with_no_repeat(window);
            for (i, o) in read_back.iter() {
                sched.set_weight(*i as usize, o.scheduler_weight().unwrap());
            }
            let n = 60_000;
            let mut c = [0usize; 3];
            for _ in 0..n {
                c[match class[sched.next_index().unwrap()] {
                    Some(Match::Strong) => 0,
                    Some(Match::Related) => 1,
                    None => 2,
                }] += 1;
            }
            (
                c[0] as f32 / n as f32,
                c[1] as f32 / n as f32,
                c[2] as f32 / n as f32,
            )
        };
        let (ds, dr, d_o) = draw(w.window as usize);
        let (gs, gr, go) = draw(32);
        println!(
            "  drawn {:.1}% / {:.1}% / {:.1}% (with the global window of 32 instead: {:.1}% / {:.1}% / {:.1}%)",
            ds * 100.0,
            dr * 100.0,
            d_o * 100.0,
            gs * 100.0,
            gr * 100.0,
            go * 100.0
        );
        near(ds, s, &format!("{game}: drawn strong vs the model"));
        near(dr, r, &format!("{game}: drawn related vs the model"));
        near(d_o, o, &format!("{game}: drawn rest vs the model"));
        if !check {
            continue;
        }
        assert!(!w.capped, "{game}: {w:?}");
        let target = follow::MATCH_PCT as f32 / 100.0;
        near(s + r, target, &format!("{game}: matches by the model"));
        near(ds + dr, target, &format!("{game}: matches drawn"));
    }
}
