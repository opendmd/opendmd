//! Build script: ESP-IDF plumbing, the embedded setup page, and the two
//! compile-time conveniences the device reads at boot.
//!
//! The setup page is rendered through Tera and gzipped into `OUT_DIR`, the same
//! trick `~/projects/stripe-dashboard/build.rs` uses: `http.rs` embeds the
//! compressed bytes with `include_bytes!` and serves them with
//! `Content-Encoding: gzip`, which browsers decompress transparently. It costs
//! roughly a third of the flash a raw string would.

use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::path::PathBuf;
use tera::{Context, Tera};

fn main() {
    embuild::espidf::sysenv::output();

    // The Makefile rewrites the partition path into target/sdkconfig.partitions;
    // still re-run when the table itself changes.
    println!("cargo:rerun-if-changed=partitions.csv");
    println!("cargo:rerun-if-changed=sdkconfig.defaults");
    println!("cargo:rerun-if-changed=components/hub75_shim/include/hub75_shim.h");
    println!("cargo:rerun-if-changed=components/hub75_shim/hub75_shim.cpp");
    println!("cargo:rerun-if-changed=web/setup.html");

    // Which flash layout this image is for (the Makefile's FW_VARIANT, which
    // also picks the partition table and the flash size): `/api/status`
    // reports it as `variant`, the boot banner prints it, and a firmware
    // update must be for the same one (`ota.rs`).
    let variant = std::env::var("OPENDMD_FLASH_VARIANT").unwrap_or_else(|_| "4mb".into());
    let variant_name = match variant.as_str() {
        "4mb" => "DevKit (4 MB)",
        "8mb" => "8 MB build",
        v => panic!("OPENDMD_FLASH_VARIANT={v}: the layouts are 4mb and 8mb"),
    };
    println!("cargo:rustc-env=OPENDMD_FLASH_VARIANT={variant}");
    println!("cargo:rerun-if-env-changed=OPENDMD_FLASH_VARIANT");

    // The firmware's version: the Makefile's FW_VERSION (which also goes
    // into the app descriptor), else the crate's. `state::firmware_version`
    // reads the descriptor and falls back to this; the setup page shows it.
    let version = std::env::var("OPENDMD_FW_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());
    println!("cargo:rustc-env=OPENDMD_FW_VERSION={version}");
    println!("cargo:rerun-if-env-changed=OPENDMD_FW_VERSION");

    // Dev convenience for the test rig, `dev-credentials` feature only:
    // `firmware/.env` (gitignored, see `.env.example`) is exported by the
    // Makefile's bench targets, and these become `option_env!` values that seed
    // NVS on a device that has no credentials of its own. Without the feature
    // (every release image) nothing is forwarded even if the variables happen
    // to be set, and `nvs_config` does not compile the `option_env!` reads at
    // all. They are NEVER echoed here — a build log is not a secret store —
    // and `nvs_config` only ever logs the SSID.
    let dev_credentials = std::env::var_os("CARGO_FEATURE_DEV_CREDENTIALS").is_some();
    for key in ["OPENDMD_WIFI_SSID", "OPENDMD_WIFI_PASS"] {
        println!("cargo:rerun-if-env-changed={key}");
        if !dev_credentials {
            continue;
        }
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                println!("cargo:rustc-env={key}={value}");
            }
        }
    }

    // The embedded setup page, rendered then gzipped.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    // The bundled Formulas (`-- bundle` in opendmd-web/formulas/, then the probe's other examples), for
    // psi-probe's opens through the real path (`psi_probe::bundled`): a
    // list of (file stem, `include_str!`), so a Formula bundled later is in
    // it without an edit here.
    let dir = std::fs::canonicalize("../opendmd-web/formulas").expect("opendmd-web/formulas");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut bundled: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .expect("reading opendmd-web/formulas")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "psi"))
        .filter(|p| {
            std::fs::read_to_string(p).is_ok_and(|t| t.lines().any(|l| l.starts_with("-- bundle")))
        })
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    bundled.sort();
    // The bundled Formulas go into the image compiled (`opendmd_psi::psb`:
    // the player never compiles), each a byte string; the probe's builds
    // also get their sources (squeezed: comments and the `about` line out).
    let strip = |text: &str| -> String {
        let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with("about \"")).collect();
        opendmd_psi::squeeze(&kept.join("\n"))
    };
    let mut list = String::from("&[\n");
    let mut sources = String::from("&[\n");
    for (name, path) in &bundled {
        println!("cargo:rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(path).expect("reading a bundled Formula");
        let psb = opendmd_psi::build(&text)
            .unwrap_or_else(|e| panic!("{}: {}", path.display(), e.render(&text)));
        list += &format!("    ({name:?}, &{psb:?}),\n");
        sources += &format!("    ({name:?}, {:?}),\n", strip(&text));
    }
    list += "]\n";
    sources += "]\n";
    std::fs::write(out_dir.join("bundled_formulas.rs"), list).expect("writing bundled_formulas.rs");
    std::fs::write(out_dir.join("bundled_sources.rs"), sources).expect("writing bundled_sources.rs");
    let mut list = String::from("&[\n");
    for name in ["plasma", "fire", "tunnel", "rotozoom", "rotozoom-floor", "wireframe-cube", "corner-clock"] {
        let path = dir.join(format!("{name}.psi"));
        println!("cargo:rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(&path).expect("reading an example Formula");
        list += &format!("    ({name:?}, {:?}),\n", strip(&text));
    }
    list += "]\n";
    std::fs::write(out_dir.join("probe_examples.rs"), list).expect("writing probe_examples.rs");
    // The paint probe's cases (`paint-probe`, src/paint_probe.rs), compiled
    // here (the image has no compiler): (group, name, pixels, psb).
    std::fs::write(out_dir.join("probe_paint.rs"), paint_cases(&dir, &strip)).expect("writing probe_paint.rs");
    let raw = std::fs::read_to_string("web/setup.html").expect("reading web/setup.html");
    let mut tera = Tera::default();
    tera.add_raw_template("setup", &raw)
        .expect("parsing web/setup.html");
    let mut ctx = Context::new();
    ctx.insert("version", &version);
    ctx.insert("variant", &variant);
    ctx.insert("variant_name", variant_name);
    let html = tera
        .render("setup", &ctx)
        .expect("rendering web/setup.html");

    let file =
        std::fs::File::create(out_dir.join("setup.html.gz")).expect("creating setup.html.gz");
    let mut encoder = GzEncoder::new(file, Compression::best());
    encoder.write_all(html.as_bytes()).expect("compressing");
    encoder.finish().expect("finishing setup.html.gz");
}

/// The paint probe's cases (src/paint_probe.rs says what each measures):
/// `step` one primitive 16 times a frame (with the pixels it examines),
/// `overlay` a painter with its plane empty and full and one that only
/// draws, `under` looks with 0-12 `UAT` steps, `formula` every Formula in
/// opendmd-web/formulas/ that paints or reads the picture elsewhere, the
/// three the paint rewrite is for, and the device cases in
/// docs/research/psi-rebuild-device/. As Rust: `&[(group, name, px, psb)]`.
fn paint_cases(dir: &std::path::Path, strip: &dyn Fn(&str) -> String) -> String {
    const STEPS: &[(&str, &str, u32)] = &[
        ("base", "pset(-9, -9, #f00)", 0),
        ("pset", "pset(64, 16, #f00)", 1),
        ("line 1", "line(10, 16, 10, 16, #f00)", 1),
        ("line 32", "line(0, 16, 31, 16, #f00)", 32),
        ("line 128", "line(0, 0, 127, 31, #f00)", 128),
        ("rect off", "rect(-30, -30, -20, -20, #f00)", 0),
        ("rect 64", "rect(60, 12, 67, 19, #f00)", 64),
        ("rect 2048", "rect(32, 0, 95, 31, #f00)", 2048),
        ("rectfill off", "rectfill(-30, -30, -20, -20, #f00)", 0),
        ("rectfill 64", "rectfill(60, 12, 67, 19, #f00)", 64),
        ("rectfill 2048", "rectfill(32, 0, 95, 31, #f00)", 2048),
        ("circ off", "circ(-40, 16, 6, #f00)", 0),
        ("circ 100", "circ(64, 16, 3, #f00)", 100),
        ("circ 1024", "circ(64, 16, 14, #f00)", 1024),
        ("circfill off", "circfill(-40, 16, 6, #f00)", 0),
        ("circfill 100", "circfill(64, 16, 3, #f00)", 100),
        ("circfill 1024", "circfill(64, 16, 14, #f00)", 1024),
        ("trifill off", "trifill(-30, -30, -20, -30, -25, -20, #f00)", 0),
        ("trifill 110", "trifill(60, 12, 68, 12, 64, 20, #f00)", 110),
        ("trifill 4096", "trifill(0, 31, 64, 0, 127, 31, #f00)", 4096),
        ("spr off", "spr(ship, -20, -20)", 20),
        ("spr 20", "spr(ship, 60, 4)", 20),
        ("print 5", "print(\"SCORE\", 2, 1, #ff0)", 175),
        ("print num 4", "print(1234, 30, 1, #ff0, small, 4)", 96),
        ("cls", "cls(#f00)", 4096),
    ];
    let mut cases: Vec<(&str, String, u32, String)> = Vec::new();
    for &(name, body, px) in STEPS {
        let src = format!(
            "sprite ship {{\n  key . 0, W #fff\n  ..W..\n  .WWW.\n  WWWWW\n  W.W.W\n}}\ndraw {{\n  for k < 16 {{\n    {body}\n  }}\n}}\ngray(x)\n"
        );
        cases.push(("step", name.into(), px, src));
    }
    for (name, src) in [
        ("plane empty", "draw {\n  pset(-9, -9, #f00)\n}\ngray(x)\n"),
        ("plane full", "draw {\n  cls(#f00)\n}\ngray(x)\n"),
        ("no plane", "draw {\n  dot(-9, -9, 1)\n}\ngray(x)\n"),
    ] {
        cases.push(("overlay", name.into(), 0, src.into()));
    }
    for (name, src) in [
        ("uat 0", "look\nunder\n"),
        ("uat 3", "look\nunder(px + a, py)\n"),
        ("uat 6", "look\nunder(px + a, py)*0.5 + under(px - a, py)*0.5\n"),
        ("uat 12", "look\nunder(px + a, py)*0.25 + under(px - a, py)*0.25 + under(px, py + a)*0.25 + under(px, py - a)*0.25\n"),
    ] {
        cases.push(("under", name.into(), 0, src.into()));
    }
    let devcases = std::fs::canonicalize("../docs/research/psi-rebuild-device").expect("psi-rebuild-device");
    println!("cargo:rerun-if-changed={}", devcases.display());
    let mut paths: Vec<PathBuf> = Vec::new();
    for d in [dir, devcases.as_path()] {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d)
            .expect("reading a Formula folder")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "psi"))
            .collect();
        v.sort();
        paths.extend(v);
    }
    for path in paths {
        let text = std::fs::read_to_string(&path).expect("reading a Formula");
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let Ok(prog) = opendmd_psi::compile(&text) else { continue };
        let wanted = ["endless-road", "neon-city", "wormhole-run"].contains(&name.as_str())
            || path.starts_with(&devcases);
        if wanted || prog.paints() || prog.uat_steps() > 0 {
            println!("cargo:rerun-if-changed={}", path.display());
            cases.push(("formula", name, 0, strip(&text)));
        }
    }
    let mut list = String::from("&[\n");
    for (group, name, px, src) in cases {
        let psb = opendmd_psi::build(&src).unwrap_or_else(|e| panic!("paint case {name}: {}", e.render(&src)));
        list += &format!("    ({group:?}, {name:?}, {px}, &{psb:?}),\n");
    }
    // P0b-4 and the guards a row (`cmp`, each with `px` the first frame's
    // ms): the fused compares' rows, each fused and not (the same frames),
    // the scenes' guards on, off and by rows against the same steps
    // without scenes, and Endless road V2 at the sweep's frames (seed
    // golden::SEED, 0.3) fused and not.
    let road = std::fs::read_to_string("../opendmd-web/experiments/road-v2/road-v2.psi").expect("road-v2.psi");
    println!("cargo:rerun-if-changed=../opendmd-web/experiments/road-v2/road-v2.psi");
    // (Each reads `t` through `u`: no still cache takes them, so a pair
    // differs only by its fused steps.)
    let base = "loop 8\nu = frac(x + t/8)\n";
    let sel4 = format!("{base}gray(frac((u < y)*u + (y < frac(u*3))*y + (u*2 < y)*0.5 + (y <= u*0.7)*0.25))\n");
    let cnt2 = format!("{base}gray(frac(((u >= y) + (u >= y*0.5))*0.2 + ((y >= u*0.5) + (y >= u*0.3))*0.2))\n");
    let three = |when: &str| {
        let mut f = String::from(base);
        for k in 1..=3 {
            match when {
                "" => f += &format!("b{k}a = sin(u*{k}.7 + y)*0.3\nb{k}b = sin(y*{k}.3 + u)*0.3\n"),
                w => f += &format!("scene s{k} when {w} {{\n  b{k}a = sin(u*{k}.7 + y)*0.3\n  b{k}b = sin(y*{k}.3 + u)*0.3\n}}\n"),
            }
        }
        f + "gray(frac(u*0.1 + b1a + b1b + b2a + b2b + b3a + b3b))\n"
    };
    let mut cmp: Vec<(String, u32, String, bool)> = vec![
        ("sel4 fused".into(), 0, sel4.clone(), true),
        ("sel4 unfused".into(), 0, sel4, false),
        ("cnt2 fused".into(), 0, cnt2.clone(), true),
        ("cnt2 unfused".into(), 0, cnt2, false),
        ("guards none".into(), 0, three(""), true),
        ("guards on".into(), 0, three("t >= 0"), true),
        ("guards off".into(), 0, three("t < 0"), true),
        ("guards rows".into(), 0, three("py < 16"), true),
        ("guards base".into(), 0, format!("{base}gray(frac(u*0.1))\n"), true),
    ];
    // (V2 fused only: it has grown past what its unfused steps fit in, with
    // its lamp posts; P0b-4 measured the pairs.)
    for (place, ms) in [("exit", 2_056_049u32), ("mouth", 733_049), ("forest-desert", 544_049), ("desert", 2_177_549)] {
        cmp.push((format!("road-v2 {place} fused"), ms, road.clone(), true));
    }
    // V2 in the city (its lamp posts), and psb ABI 3: the same table and
    // sprite reads in the store's first numbers and past 900 (the store
    // sized 1,024; the helpers mask by its length), and built-in inputs
    // against the same constants.
    cmp.push(("road-v2 city fused".into(), 128_049, road.clone(), true));
    let reads = "table T = 0.1, 0.5, 0.9, 0.3, 0.7\nsprite s {\nkey . 0, X 1, Y 0.5\nXY.XY.XY.XY.XY.XY.\n.XYXY.XYXY.XYXY.XY\n}\ngray(frac(T[x*5 + t] + s[x*18 + t, y*2] + T[y*5]*0.3 + s[x*9, y*2 + t]))\n";
    let filler: Vec<String> = (0..250).map(|k| format!("{}", k % 9)).collect();
    let fill = format!("table F0 = {f}, 1\ntable F1 = {f}, 2\ntable F2 = {f}, 3\n", f = filler.join(", "));
    cmp.push(("store low".into(), 0, format!("loop 8\n{reads}"), true));
    cmp.push(("store high".into(), 0, format!("loop 8\n{fill}{reads}").replace("gray(frac(", "gray(frac(F2[250]*0.01 + "), true));
    cmp.push(("store low+f".into(), 0, format!("loop 8\n{reads}").replace("gray(frac(", "gray(frac(0.03 + "), true));
    cmp.push(("inputs".into(), 0, format!("{base}gray(frac(u*daylight + sun.alt + y*moon.phase + since(night)*0.001 + was(bright)))\n"), true));
    cmp.push(("inputs const".into(), 0, format!("{base}gray(frac(u*1 + 0.1 + y*0.5 + 32767*0.001 + 1))\n"), true));
    for (name, ms, src, fused) in cmp {
        let psb = opendmd_psi::build_with(&src, fused).unwrap_or_else(|e| panic!("cmp case {name}: {}", e.render(&src)));
        list += &format!("    (\"cmp\", {name:?}, {ms}, &{psb:?}),\n");
    }
    list + "]\n"
}
