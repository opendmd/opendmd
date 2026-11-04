//! The paint probe (`--features paint-probe`, psi-finish 2026-11-04):
//! what the cost model's paint numbers (`Program::paint_ns`,
//! `PAINT_ROWS_US`) and `UAT`'s (`value_ns`) were guessed or fitted from
//! two points, measured on the device. No compiler in the image (the
//! psi-probe's does not fit the 4mb layout beside its own rows): build.rs
//! compiles every case (`paint_cases`). `make emu-image-paint` builds it
//! for the spare's 4mb layout; `make emu-paint-probe` boots it in the
//! emulator (correctness only: its timings mean nothing).
//!
//! Run once a boot on the main task (CPU1, as the psi-probe), when the boot
//! task hands the card over, with the httpd and the network up. Each line
//! is logged and appended to [`REPORT_NAME`] on the card at once
//! (`GET /filelist-paint-probe.txt`); a report without its `done` line is a
//! probe that did not finish, and the next boot does not run it again
//! (delete the file to probe again). ~1-2 minutes.
//!
//! One summary line a case, each starting `psi-probe: paint:`:
//!
//! - `step NAME`: one primitive drawn 16 times a frame (`for k < 16`):
//!   the frame steps and the draw stage in µs a frame (the VM's stamps; the
//!   kernel's laps), p50 over [`N`] frames; less the `base` case's (the same
//!   loop, `pset` off the panel), a 16th of it the primitive's own
//!   (`ns/prim`); `px` the pixels it examines (`draw::paint_prim`'s walk);
//!   the model's `draw_us` and `paint_us` beside it.
//! - `overlay`: the paint plane laid over a frame alone
//!   (`vm::stats::paint_rows`), nothing painted and every pixel painted,
//!   and the VM's rows of a formula with no plane, an empty and a full one.
//! - `under NAME`: a look over the golden picture with 0, 3, 6 and 12
//!   `UAT` steps: the VM's rows, the kernel's when it takes it (else why
//!   not), the model's estimates.
//! - `formula NAME`: on the real path (an effect through `Fx::parse`, or a
//!   look through `FormulaLook` over a picture) JIT on and off; its frame
//!   steps and draw stage, and its rows, directly (VM, kernel); the overlay
//!   alone; every estimate (`esp32_us`, `_jit`, `draw_us`, `paint_us`,
//!   `overlay_us`, the `UAT` steps).
//!
//! Each case's kernel frames are compared with the VM's (FNV-1a of the
//! first frames); the last line before `done`:
//! `psi-probe: paint: N cases, M differ`.
use opendmd_core::canvas::Canvas;
use opendmd_core::fx::Fx;
use opendmd_core::fx_psi;
use opendmd_core::psi;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The report's file name at the card's root (`filelist…` so the read-only
/// `GET /filelist*` route serves it; not `filelist.`, so nothing takes it
/// for a playlist).
const REPORT_NAME: &str = "filelist-paint-probe.txt";
const DONE: &str = "psi-probe: done";
static REPORT: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// A report line: logged, and appended to the report at once (synced).
#[inline(never)]
fn say(line: String) {
    log::info!("{line}");
    if let Some(f) = REPORT.lock().unwrap().as_mut() {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
        let _ = f.sync_data();
    }
}

/// Open the report: `false` when last boot's is there without its `done`.
fn open_report(root: &str) -> bool {
    let path = format!("{root}/{REPORT_NAME}");
    if let Ok(old) = std::fs::read_to_string(&path) {
        if !old.contains(DONE) {
            log::warn!("psi-probe: /{REPORT_NAME} has no `{DONE}` line: last boot's paint probe did not finish; not running it again (delete the file to)");
            return false;
        }
    }
    match std::fs::File::create(&path) {
        Ok(mut f) => {
            use std::io::Write;
            let _ = writeln!(f, "# OpenDMD {} paint probe report", crate::state::firmware_version());
            let _ = f.sync_data();
            *REPORT.lock().unwrap() = Some(f);
            log::info!("psi-probe: writing /{REPORT_NAME} as it goes: GET /{REPORT_NAME}");
            true
        }
        Err(e) => {
            log::error!("psi-probe: cannot write {path} ({e}); not running");
            false
        }
    }
}

/// Frames timed a case (each a p50).
const N: u32 = 64;

/// Cycles to µs (240 MHz).
const MHZ: u32 = 240;

/// The cases (build.rs `paint_cases`): (group, name, pixels, psb).
const CASES: &[(&str, &str, u32, &[u8])] = include!(concat!(env!("OUT_DIR"), "/probe_paint.rs"));

fn ccount() -> u32 {
    // SAFETY: reads the CCOUNT special register.
    unsafe { esp_idf_sys::xthal_get_ccount() }
}

fn heap() -> u32 {
    crate::state::free_heap()
}

/// A zeroed `Box<T>` for plain-integer `T` (psi's `Program`, `Scratch`).
fn zeroed<T>() -> Box<T> {
    // SAFETY: only used for psi's `Program` and `Scratch`, integers and
    // arrays of integers: all-zero is a valid value.
    unsafe { Box::<T>::new_zeroed().assume_init() }
}

fn p50(v: &mut [u32]) -> u32 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

/// What one direct run measured, µs a frame (p50 each).
struct Direct {
    /// The whole call.
    frame: u32,
    /// The frame's steps and the draw stage (the VM's stamps 1-2; the
    /// kernel's `frame steps` and `draw stage and frame words` laps).
    draw: u32,
    /// The rows (the VM's stamps 2-3; the kernel's other laps).
    rows: u32,
    /// The first frames' FNV-1a, folded.
    hash: u64,
}

/// Inputs for frame `n` of `prog` (20 ms apart within its loop).
fn inputs_at(prog: &psi::Program, n: u32) -> psi::Inputs {
    let mut i = psi::golden::inputs(0);
    let lp = prog.loop_ms().max(1);
    i.set_ms(prog, u64::from(n * 20 % lp));
    i
}

/// `prog` drawn directly by the VM, [`N`] frames after 4 untimed.
fn vm_direct(prog: &psi::Program, under: Option<&[u8]>, out: &mut [u8]) -> Direct {
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut ink = psi::Ink::boxed();
    let (mut fr, mut dr, mut rw) = (Vec::with_capacity(N as usize), Vec::with_capacity(N as usize), Vec::with_capacity(N as usize));
    let mut hash = 0u64;
    for n in 0..N + 4 {
        let inputs = inputs_at(prog, n);
        let c0 = ccount();
        psi::render_rows_ink::<128>(prog, &mut s, Some(&mut ink), &inputs, &t, under, u32::MAX, out);
        let c1 = ccount();
        let st = |k: usize| psi::stats::STAMPS[k].load(Ordering::Relaxed);
        if n < 4 {
            hash = hash.rotate_left(7) ^ psi::golden::fnv(out);
        } else {
            fr.push(c1.wrapping_sub(c0) / MHZ);
            dr.push(st(2).wrapping_sub(st(1)) / MHZ);
            rw.push(st(3).wrapping_sub(st(2)) / MHZ);
        }
        if n % 16 == 15 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Direct { frame: p50(&mut fr), draw: p50(&mut dr), rows: p50(&mut rw), hash }
}

/// `prog` drawn directly by its kernel, or why the JIT refuses it.
#[cfg(feature = "jit")]
fn jit_direct(prog: &psi::Program, under: Option<&[u8]>, out: &mut [u8]) -> Result<Direct, &'static str> {
    use psi::jit::stats as js;
    let mut k = psi::jit::emit(prog)?;
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut ink = psi::Ink::boxed();
    let (mut fr, mut dr, mut rw) = (Vec::with_capacity(N as usize), Vec::with_capacity(N as usize), Vec::with_capacity(N as usize));
    let mut hash = 0u64;
    let _ = js::take();
    for n in 0..N + 4 {
        let inputs = inputs_at(prog, n);
        let c0 = ccount();
        k.render(prog, &mut s, Some(&mut ink), None, &inputs, &t, under, u32::MAX, out, None);
        let c1 = ccount();
        let (cyc, _) = js::take();
        if n < 4 {
            hash = hash.rotate_left(7) ^ psi::golden::fnv(out);
        } else {
            fr.push(c1.wrapping_sub(c0) / MHZ);
            dr.push((cyc[js::DRAW] + cyc[js::FRAME]) / MHZ);
            rw.push((cyc[js::ROWS] + cyc[js::PRE] + cyc[js::FILL] + cyc[js::KERNEL]) / MHZ);
        }
        if n % 16 == 15 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok(Direct { frame: p50(&mut fr), draw: p50(&mut dr), rows: p50(&mut rw), hash })
}

#[cfg(not(feature = "jit"))]
fn jit_direct(_: &psi::Program, _: Option<&[u8]>, _: &mut [u8]) -> Result<Direct, &'static str> {
    Err("no JIT in this build")
}

/// The overlay alone over the plane `prog` last drew into `ink`, µs.
fn overlay_alone(prog: &psi::Program, ink: &psi::Ink, out: &mut [u8]) -> u32 {
    let mut v = Vec::with_capacity(N as usize);
    for n in 0..N {
        let c0 = ccount();
        psi::stats::paint_rows(prog, ink, out);
        v.push(ccount().wrapping_sub(c0) / MHZ);
        if n % 16 == 15 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    p50(&mut v)
}

/// The overlay alone after one frame of `prog`.
fn overlay_of(prog: &psi::Program, under: Option<&[u8]>, out: &mut [u8]) -> u32 {
    let t = fx_psi::tables();
    let mut s = zeroed::<psi::Scratch<128>>();
    s.reset();
    let mut ink = psi::Ink::boxed();
    psi::render_rows_ink::<128>(prog, &mut s, Some(&mut ink), &psi::golden::inputs(0), &t, under, u32::MAX, out);
    overlay_alone(prog, &ink, out)
}

/// The cases so far, and those whose kernel frames differ from the VM's.
#[derive(Default)]
struct Tally {
    cases: u32,
    differ: u32,
}

impl Tally {
    /// The JIT's part of a line, and whether its frames are the VM's.
    fn jit(&mut self, vm: &Direct, jit: &Result<Direct, &'static str>) -> String {
        self.cases += 1;
        match jit {
            Ok(j) => {
                let same = j.hash == vm.hash;
                if !same {
                    self.differ += 1;
                }
                format!(
                    "jit frame {} draw {} rows {} ({})",
                    j.frame,
                    j.draw,
                    j.rows,
                    if same { "the VM's frames" } else { "FRAMES DIFFER" }
                )
            }
            Err(why) => format!("jit refuses: {why}"),
        }
    }
}

fn loaded(name: &str, psb: &[u8]) -> Option<Box<psi::Program>> {
    let mut prog = zeroed::<psi::Program>();
    match psi::psb::load_into(psb, psi::psb::Which::Play, &mut prog) {
        Ok(()) => Some(prog),
        Err(e) => {
            say(format!("psi-probe: paint: {name}: refused: {e:?}"));
            None
        }
    }
}

/// A Formula on the real path, JIT on then off: p50 µs a frame.
fn real_path(name: &str, psb: &[u8], over: bool, canvas: &mut Canvas) -> [Option<u32>; 2] {
    let mut got = [None; 2];
    let path = fx_psi::path_of(psb, "");
    for (i, jit) in [true, false].into_iter().enumerate() {
        fx_psi::set_jit(jit);
        let mut v = Vec::with_capacity(N as usize);
        if over {
            let Ok(mut look) = fx_psi::FormulaLook::from_path(&path) else {
                say(format!("psi-probe: paint: formula {name}: the look did not open"));
                break;
            };
            if !look.take() {
                say(format!("psi-probe: paint: formula {name}: no room for the look's buffers ({} B free)", heap()));
                break;
            }
            let mut pic = vec![0u8; psi::W * psi::H * 3];
            psi::golden::picture(&mut pic);
            canvas.as_mut_bytes().copy_from_slice(&pic);
            drop(pic);
            for k in 0..N + 12 {
                look.redraw();
                canvas.dirty_mut().mark_all();
                let t = Instant::now();
                look.apply(Duration::from_millis(u64::from(k) * 20), canvas, 0).dirty_mut().clear();
                if k >= 12 {
                    v.push(t.elapsed().as_micros() as u32);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        } else {
            let Some(mut fx) = Fx::parse(&path) else {
                say(format!("psi-probe: paint: formula {name}: did not open ({} B free)", heap()));
                break;
            };
            let delay = fx.delay();
            let n = fx.loop_frames().max(1);
            for k in 0..N + 12 {
                let t = Instant::now();
                fx.frame(delay * (k % n), canvas);
                if k >= 12 {
                    v.push(t.elapsed().as_micros() as u32);
                }
                canvas.dirty_mut().clear();
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        got[i] = Some(p50(&mut v));
    }
    fx_psi::set_jit(true);
    got
}

/// The cases in order: the steps, the overlay, `under`, the Formulas.
fn rows(tally: &mut Tally) {
    let mut out = vec![0u8; psi::W * psi::H * 3];
    let mut pic = vec![0u8; psi::W * psi::H * 3];
    psi::golden::picture(&mut pic);
    let mut canvas = Canvas::new();
    let mut base: Option<(u32, u32)> = None;
    // The overlay's three: alone (empty, full), the rows (empty, full, none).
    let (mut alone, mut ov_rows, mut ov_est) = ([0u32; 2], [0u32; 3], 0);
    for &(group, name, px, psb) in CASES {
        if heap() < 40 * 1024 {
            say(format!("psi-probe: paint: stopped before {name}: heap {} B free", heap()));
            break;
        }
        let Some(prog) = loaded(name, psb) else { continue };
        let under = (group == "under" || (group == "formula" && prog.over())).then_some(pic.as_slice());
        let vm = vm_direct(&prog, under, &mut out);
        let jit = jit_direct(&prog, under, &mut out);
        let j = tally.jit(&vm, &jit);
        match group {
            "step" => {
                let jd = jit.as_ref().map_or(vm.draw, |j| j.draw);
                let (bv, bj) = *base.get_or_insert((vm.draw, jd));
                let each = |d: u32, b: u32| (d.saturating_sub(b) * 1000) / 16;
                say(format!(
                    "psi-probe: paint: step {name}: px {px}, vm draw {} us ({} ns/prim over base), {j} ({} ns/prim over base); est draw_us {} paint_us {}",
                    vm.draw, each(vm.draw, bv), each(jd, bj), prog.draw_us(), prog.paint_us()
                ));
            }
            "overlay" => {
                let k = match name {
                    "plane empty" => 0,
                    "plane full" => 1,
                    _ => 2,
                };
                ov_rows[k] = vm.rows;
                if k < 2 {
                    alone[k] = overlay_of(&prog, None, &mut out);
                    ov_est = prog.overlay_us();
                }
                if k == 2 {
                    say(format!(
                        "psi-probe: paint: overlay: alone {} us a frame with nothing painted, {} us with every pixel painted; vm rows {} us with no plane, {} with it empty, {} with it full; est overlay_us {ov_est}",
                        alone[0], alone[1], ov_rows[2], ov_rows[0], ov_rows[1]
                    ));
                }
            }
            "under" => say(format!(
                "psi-probe: paint: under {name}: {} UAT steps, vm rows {} us frame {} us, {j}; est vm {} jit {}",
                prog.uat_steps(), vm.rows, vm.frame, prog.esp32_us(), prog.esp32_us_jit()
            )),
            _ => {
                let ov = if prog.paints() { overlay_of(&prog, under, &mut out) } else { 0 };
                let over = prog.over();
                let [rj, rv] = real_path(name, psb, over, &mut canvas);
                let show = |v: Option<u32>| v.map_or("-".to_string(), |v| v.to_string());
                say(format!(
                    "psi-probe: paint: formula {name}: {} jit {} vm {} us; direct vm frame {} draw {} rows {}, {j}; overlay alone {ov} us; est vm {} jit {} draw_us {} paint_us {} overlay_us {} uat {} steps",
                    if over { "look" } else { "effect" },
                    show(rj), show(rv), vm.frame, vm.draw, vm.rows,
                    prog.esp32_us(), prog.esp32_us_jit(), prog.draw_us(), prog.paint_us(), prog.overlay_us(), prog.uat_steps()
                ));
            }
        }
    }
}

pub fn run(root: &str) {
    if !open_report(root) {
        return;
    }
    let start = Instant::now();
    psi::stats::CLOCK.store(ccount as fn() -> u32 as usize, Ordering::Relaxed);
    fx_psi::set_still_gate(Some(crate::still_gate));
    say(format!("psi-probe: paint: start, {} cases, heap {} B free", CASES.len(), heap()));
    let mut tally = Tally::default();
    rows(&mut tally);
    say(format!(
        "psi-probe: paint: {} cases, {} differ{}; heap {} B free",
        tally.cases, tally.differ, if tally.differ == 0 { " (bit-identical)" } else { "" }, heap()
    ));
    say(format!("{DONE}: paint, heap {} B free, {} ms in all", heap(), start.elapsed().as_millis()));
    *REPORT.lock().unwrap() = None;
}
