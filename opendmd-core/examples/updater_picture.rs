//! Writes the updater's picture header (`ota/updater/main/picture.h`) to
//! stdout or to the path given: `make updater-picture`.
//! See `opendmd_core::updater_picture`.

fn main() {
    let h = opendmd_core::updater_picture::c_header();
    match std::env::args().nth(1) {
        Some(path) => std::fs::write(&path, h).unwrap_or_else(|e| panic!("{path}: {e}")),
        None => print!("{h}"),
    }
}
