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
