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

/// The prepared copies on the card (`opendmd-pack convert --verify <card>`
/// makes them): twenty files — the seven the heap budget is measured on,
/// the two strays (132x32 and 135x32) and eleven drawn with a fixed seed —
/// must each be exactly `recode_gif`'s bytes for its GIF, open through
/// `FrameSource` as gifslim at the hint's cap 9, and play every canvas and
/// delay exactly as `GifSource` plays the GIF (a GIF past 256 colours has
/// no copy: it plays as it is, and a copy found for one is an error). Then the engine plays 50 entries of
/// `filelist.txt` through a source that resolves each GIF's copy first, as
/// the firmware's `SdSource` does.
#[test]
#[ignore = "needs DMD_CARD_ROOT pointing at a real card dump with cache/"]
fn prepared_copies_on_the_real_card() {
    use opendmd_core::gifslim::{recode_gif, PREPARED};
    use opendmd_core::layout;
    use opendmd_core::player::{FrameSource, GifSource};

    let Ok(card) = std::env::var("DMD_CARD_ROOT") else {
        eprintln!("DMD_CARD_ROOT not set, nothing to do");
        return;
    };
    let card = PathBuf::from(card);
    assert!(
        card.join("cache").is_dir(),
        "no cache/ on the card: run opendmd-pack convert first"
    );
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
    .map(|s| format!("/gifs/{s}.gif"))
    .collect();
    let text = std::fs::read_to_string(card.join("filelist.txt")).unwrap();
    let all: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    let mut rng = Rng::new(0x4341_4348_4500_0020);
    let mut total_frames = 0u64;
    let mut checked = 0;
    while checked < 20 {
        let gif_path = if checked < picks.len() {
            picks[checked].clone()
        } else {
            let p = all[(rng.next_u64() % all.len() as u64) as usize].to_string();
            if picks.contains(&p) {
                continue;
            }
            picks.push(p.clone());
            p
        };
        checked += 1;
        let gif = std::fs::read(card.join(gif_path.trim_start_matches('/'))).unwrap();
        let mut want = Vec::new();
        let copy_path = layout::cache_path(&gif_path).unwrap();
        let stats = match recode_gif(&gif[..], &mut want, &PREPARED) {
            Ok(s) => s,
            Err(e) if opendmd_core::gifslim::is_too_many_colours(&e) => {
                println!("  ==  {gif_path}: more than 256 colours, plays as it is");
                assert!(
                    !card.join(copy_path.trim_start_matches('/')).exists(),
                    "{copy_path}: a lossy copy (opendmd-pack convert removes it)"
                );
                continue;
            }
            Err(_) => {
                println!("  --  {gif_path}: does not recode (damaged), no copy expected");
                continue;
            }
        };
        let copy = std::fs::read(card.join(copy_path.trim_start_matches('/')))
            .unwrap_or_else(|e| panic!("{copy_path}: {e}"));
        assert!(copy == want, "{copy_path}: not recode_gif's bytes");
        let mut c = FrameSource::open(io::Cursor::new(copy)).unwrap();
        assert_eq!((c.kind(), c.cap()), ("slim", Some(9)), "{copy_path}");
        let mut g = GifSource::new(&gif[..]).unwrap();
        let (mut cg, mut cc) = (Canvas::new(), Canvas::new());
        let mut frames = 0u64;
        loop {
            let dg = g.next_frame(&mut cg).unwrap();
            let dc = c.next_frame(&mut cc).unwrap();
            assert_eq!(dg, dc, "{copy_path}: delay at frame {frames}");
            if dg.is_none() {
                break;
            }
            assert!(cg == cc, "{copy_path}: frame {frames} differs");
            frames += 1;
        }
        assert!(c.take_fallback().is_none(), "{copy_path}: fell back");
        println!(
            "  ok  {copy_path}: {frames} frames, {}x{}, {} colours, {} -> {} B",
            stats.width,
            stats.height,
            stats.colours,
            gif.len(),
            stats.out_bytes
        );
        total_frames += frames;
    }
    println!("{checked} copies, {total_frames} frames identical to their GIFs");

    /// The firmware's resolution rule: the copy if it opens, else the GIF.
    struct Resolving(PathBuf, std::cell::Cell<(u32, u32)>);
    impl Source for Resolving {
        fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>> {
            let (hits, misses) = self.1.get();
            if let Some(copy) = layout::cache_path(path) {
                if let Ok(f) = std::fs::File::open(self.0.join(copy.trim_start_matches('/'))) {
                    self.1.set((hits + 1, misses));
                    return Ok(Box::new(f));
                }
            }
            self.1.set((hits, misses + 1));
            Ok(Box::new(std::fs::File::open(
                self.0.join(path.trim_start_matches('/')),
            )?))
        }
    }
    let file = std::fs::File::open(card.join("filelist.txt")).unwrap();
    let catalog = FileCatalog::build(file, &Filter::new()).unwrap();
    let n = catalog.len();
    let mut engine = Engine::new(
        Box::new(catalog),
        Scheduler::new(n, Mode::Shuffle, 7),
        DwellPolicy {
            min: Duration::ZERO,
            max: Duration::from_secs(30),
            min_loops: 1,
        },
        Box::new(Resolving(card.clone(), Default::default())),
    );
    let mut canvas = Canvas::new();
    let mut frames = 0;
    for step in 0..50u64 {
        match engine
            .tick(Duration::from_secs(step * 100), &mut canvas)
            .unwrap()
        {
            TickOutcome::Frame { .. } => frames += 1,
            TickOutcome::Exhausted => panic!("ran out at step {step}"),
            TickOutcome::Idle => {}
        }
        engine.skip();
    }
    println!("engine over resolved copies: {frames} of 50 ticks drew a frame");
    assert_eq!(frames, 50);
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

/// Every name on the card read as fields, built back into a name and read
/// again gives the same fields (`manifest::NameFields`, what the web app's
/// "Edit details" writes). The few that cannot be built are printed with
/// why; there must be very few.
#[cfg(feature = "names")]
#[test]
#[ignore = "needs DMD_CARD_ROOT pointing at a real card dump"]
fn real_card_names_round_trip_through_the_builder() {
    use opendmd_core::manifest::NameFields;
    let Ok(root) = std::env::var("DMD_CARD_ROOT") else {
        eprintln!("DMD_CARD_ROOT not set, nothing to do");
        return;
    };
    let text = std::fs::read_to_string(PathBuf::from(root).join("filelist.txt"))
        .expect("read filelist.txt");
    let (mut ok, mut same, mut refused) = (0usize, 0usize, BTreeMap::<&str, Vec<String>>::new());
    for e in parse_manifest(&text) {
        let f = NameFields::from_path(&e.path);
        match f.path() {
            Ok(p) => {
                assert_eq!(NameFields::from_path(&p), f, "{} -> {p}", e.path);
                ok += 1;
                same += usize::from(p == e.path);
            }
            Err(why) => refused.entry(why.code()).or_default().push(e.path.clone()),
        }
    }
    let n_refused: usize = refused.values().map(Vec::len).sum();
    println!("names: {ok} rebuilt ({same} byte-identical), {n_refused} refused");
    for (why, paths) in &refused {
        println!(
            "  {why}: {} e.g. {:?}",
            paths.len(),
            &paths[..paths.len().min(5)]
        );
    }
    assert!(
        n_refused * 100 <= ok,
        "more than 1% of the card cannot be rebuilt"
    );
}
