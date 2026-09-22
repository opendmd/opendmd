//! Integration test against a real DMDoS card dump.
//!
//! Ignored by default — it needs the (third-party, not-redistributable) GIF
//! pack on disk:
//!
//! ```sh
//! DMD_CARD_ROOT=~/scratch/dmdos-card cargo test -- --ignored --nocapture
//! ```

use opendmd_core::canvas::Canvas;
use opendmd_core::engine::{Catalog, DwellPolicy, Engine, FileCatalog, Source, TickOutcome};
use opendmd_core::filter::Filter;
use opendmd_core::index::{write_index, IndexHeader, IndexedCatalog};
use opendmd_core::manifest::{parse_manifest, render_manifest};
use opendmd_core::scheduler::{Mode, Rng, Scheduler};
use std::collections::BTreeMap;
use std::io::{self, BufReader, Read};
use std::path::PathBuf;
use std::time::Duration;

/// Joins card-absolute manifest paths (`/gifs/...`) onto a root directory.
struct DirSource {
    root: PathBuf,
}

impl Source for DirSource {
    fn open(&self, path: &str) -> io::Result<Box<dyn Read>> {
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
        assert!(header.is_valid_for(manifest_len, &filter));
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
