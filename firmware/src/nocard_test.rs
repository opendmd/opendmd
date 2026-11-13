//! The no-card lane's self-test (`nocard-test` feature, `make emu-nocard`:
//! esp32sim boots the firmware with no card). With no card the settings
//! live in NVS: the test checks the store says so, applies a change through
//! the path `PUT /api/config` takes (its checks, the main task's apply, the
//! card save that does nothing without a card), restarts, and on the next
//! boot finds the change kept. Lines `nocard-test: …` for tools/emu/run.py.

use crate::http::{apply_patch, ConfigPatch, Ctx};
use crate::state::Command;
use std::sync::Arc;
use std::time::Duration;

/// The brightness the test sets: on the boot after the restart it is the
/// sign the change was kept.
const MARK: u8 = 37;

pub fn spawn(ctx: Arc<Ctx>) {
    // The S3's power-safe lane boots this image with `ps_test` (s3board):
    // its restarts are its own.
    #[cfg(esp32s3)]
    if crate::s3board::ps_test() {
        log::info!("nocard-test: the power-safe lane's boot (ps_test): not run");
        return;
    }
    let _ = crate::task::spawn_core0(b"nocard-test\0", 8 * 1024, 2, Box::new(move || run(&ctx)));
}

fn run(ctx: &Ctx) {
    std::thread::sleep(Duration::from_secs(4));
    if crate::CARD_READY.load(std::sync::atomic::Ordering::Relaxed) {
        log::info!("nocard-test: a card is mounted; nothing to test");
        return;
    }
    log::info!("nocard-test: store{}", crate::settings_card::status_json());
    let now = ctx.settings.lock().unwrap().brightness;
    if now == MARK {
        log::info!("nocard-test: after the restart: brightness {now}, kept");
        return;
    }
    let mut next = ctx.settings.lock().unwrap().clone();
    let mut patch = ConfigPatch::default();
    patch.brightness = Some(MARK);
    if let Err(e) = apply_patch(&mut next, &patch) {
        log::error!("nocard-test: refused: {}", e.msg);
        return;
    }
    let shared = next.clone();
    let (tx, rx) = crate::chan::channel();
    if ctx.cmd.send(Command::ApplyConfig { settings: Box::new(next), panel: None, look_named: false, boot_named: false, reply: tx }).is_err() {
        log::error!("nocard-test: no main task");
        return;
    }
    let ok = matches!(rx.recv_timeout(Duration::from_secs(10)), Ok(Ok(_)));
    crate::state::drop_spent();
    *ctx.settings.lock().unwrap() = shared;
    crate::settings_card::save_shared(&ctx.settings);
    log::info!("nocard-test: applied brightness {MARK}: {}", if ok { "ok" } else { "FAILED" });
    std::thread::sleep(Duration::from_millis(500));
    crate::restart(opendmd_core::restarts::Reason::ApiReboot);
}
