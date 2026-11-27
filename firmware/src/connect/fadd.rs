//! A Formula added by a remote controller (`formula-add`, `formula-part`;
//! protocol 5.7): the card's half of `opendmd_core::connect::fadd`. Built
//! only with `connect-formula-add` (test images: never a release).
//!
//! - **Before any card read:** the add's own rate, the same add signed
//!   again (no card read at all), another key's add under way, the day's
//!   tally (`/formulas/.add-day`, kept across restarts), a LAN transfer or
//!   update under way (`busy`).
//! - **One directory pass** (no lookup per name): the Formulas on the card
//!   (at most [`fadd::MAX_FORMULAS`] sources, else `quota`) and the taken
//!   stems (a `.psi`, `.psb` or `.psr`); where it lands is the stem or the
//!   next free `-2` … `-9`. A taken stem whose `.psi` has the signed
//!   source's length is hashed, and if it is the signed source it is the
//!   same Formula: `done`, nothing written.
//! - **The bytes** go to `/formulas/.add-<id>.psi.part` and `.psr.part`
//!   (dot names: never listed or played; gone when the add is dropped or
//!   stale, and at boot).
//! - **The last part:** both hashes (the core's), the source UTF-8 with no
//!   NUL, the program `PSB` + this ABI and every program in it loaded and
//!   checked (`psb::verify`), behind a heap gate (the bytes, one program, 20
//!   KB for the rest; below it, or a load short of heap, `busy`: the same
//!   `formula-add` signed again tries again).
//! - **Landing, fail closed:** the program renamed to `<stem>.psr` first,
//!   then the source to `<stem>.psi` (a rename never replaces a file). It is
//!   never a `.psb`: whatever is read from a `.psr` plays on the VM only
//!   (`fx_psi::card_program`), until the panel's own compiler writes the
//!   `.psb` from the source and removes the `.psr` (`card_formulas`). A
//!   restart between the two renames leaves a `.psr` with no source, which
//!   the next pass removes; a failed second rename removes the first.

use super::client::{core_now, Client};
use crate::config;
use opendmd_core::connect::fadd::{self, AddArgs, Part, Pre, Tally};
use opendmd_core::connect::keys::KeyId;
use opendmd_core::connect::proto::Refusal;
use opendmd_core::synth::REMOTE_EXT;
use std::io::{Read, Write};

type Done = Result<Option<String>, Refusal>;

fn dir() -> String {
    format!("{}/formulas", config::SD_MOUNT)
}

const TALLY_FILE: &str = ".add-day";

fn part_path(id: u32, p: Part) -> String {
    format!("{}/.add-{id}.{}.part", dir(), if p == Part::Source { "psi" } else { "psr" })
}

fn remove_parts(id: u32) {
    let _ = std::fs::remove_file(part_path(id, Part::Source));
    let _ = std::fs::remove_file(part_path(id, Part::Psb));
}

/// A LAN transfer (an upload, a stream, an update's) shares the card.
fn lan_busy() -> bool {
    crate::cache_builder::transfer_in_flight() || crate::stream::uploads() > 0
}

/// Today by the panel's clock (Unix days), when it has one.
fn today() -> Option<u64> {
    crate::clock::unix_valid().and_then(|t| u64::try_from(t).ok()).map(|t| t / 86_400)
}

/// The tally as the card keeps it (none yet: a fresh day; unreadable: a full one).
pub(super) fn load_tally() -> Tally {
    match std::fs::read_to_string(format!("{}/{TALLY_FILE}", dir())) {
        Ok(t) => Tally::parse(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Tally::default(),
        Err(_) => Tally::parse("x"),
    }
}

fn save_tally(t: &Tally) -> bool {
    std::fs::write(format!("{}/{TALLY_FILE}", dir()), t.text()).is_ok()
}

/// The add's directory pass: the stems taken (lower case) and how many
/// Formulas there are, with each taken `.psi`'s size.
fn scan() -> Option<(Vec<(String, u32)>, usize)> {
    use esp_idf_sys as sys;
    let path = std::ffi::CString::new("0:/formulas").ok()?;
    let mut taken = Vec::new();
    let mut sources = 0usize;
    // SAFETY: plain data FatFs fills, the directory closed before return.
    unsafe {
        let mut d: sys::FF_DIR = std::mem::zeroed();
        let r = sys::f_opendir(&mut d, path.as_ptr());
        if r == sys::FRESULT_FR_NO_PATH {
            return Some((taken, 0));
        }
        if r != sys::FRESULT_FR_OK {
            return None;
        }
        let mut fi: sys::FILINFO = std::mem::zeroed();
        while sys::f_readdir(&mut d, &mut fi) == sys::FRESULT_FR_OK && fi.fname[0] != 0 {
            let Ok(name) = std::ffi::CStr::from_ptr(fi.fname.as_ptr()).to_str() else { continue };
            let l = name.to_ascii_lowercase();
            for ext in [".psi", ".psb", REMOTE_EXT] {
                if let Some(st) = l.strip_suffix(ext) {
                    // (Only a `.psi`'s size matters: 0 for the others.)
                    taken.push((st.to_string(), if ext == ".psi" { fi.fsize as u32 } else { u32::MAX }));
                    if ext == ".psi" {
                        sources += 1;
                    }
                }
            }
            // (Bounded: past the cap the add is refused anyway.)
            if taken.len() > 3 * fadd::MAX_FORMULAS + 64 {
                break;
            }
        }
        sys::f_closedir(&mut d);
    }
    Some((taken, sources))
}

/// Where `args` lands: `Ok(Err(stem))` the same source is there already
/// (done, nothing written); `Ok(Ok(stem))` a free stem.
fn target(args: &AddArgs) -> Result<Result<String, String>, Refusal> {
    let (taken, sources) = scan().ok_or(Refusal::Busy)?;
    if sources >= fadd::MAX_FORMULAS {
        return Err(Refusal::Quota);
    }
    for stem in fadd::stem_choices(&args.stem) {
        let here: Vec<u32> = taken.iter().filter(|(s, _)| *s == stem).map(|&(_, n)| n).collect();
        if here.is_empty() {
            return Ok(Ok(stem));
        }
        // Hashed only when its length is the signed source's.
        if here.contains(&args.src_size) {
            let f = std::fs::File::open(format!("{}/{stem}.psi", dir())).map_err(|_| Refusal::Busy)?;
            if fadd::sha256_of(f).ok() == Some(args.src_sha256) {
                return Ok(Err(stem));
            }
        }
    }
    Err(Refusal::Quota)
}

fn done(stem: &str) -> Option<String> {
    Some(format!(r#"{{"done":true,"stem":{}}}"#, opendmd_core::connect::json::quote(stem)))
}

/// The source part: UTF-8, no NUL (read in pieces).
fn source_ok(path: &str) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut b = [0u8; 516];
    let mut carry = 0usize;
    loop {
        let n = match f.read(&mut b[carry..512]) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let len = carry + n;
        if n == 0 {
            return carry == 0;
        }
        if b[carry..len].contains(&0) {
            return false;
        }
        match std::str::from_utf8(&b[..len]) {
            Ok(_) => carry = 0,
            Err(e) if e.error_len().is_none() => {
                let ok = e.valid_up_to();
                carry = len - ok;
                b.copy_within(ok..len, 0);
            }
            Err(_) => return false,
        }
    }
}

/// The program: this player's format, every program in it loaded and
/// checked as anything from the network is (`psb::verify`). `Err`: the
/// heap had no room for a program (try again), not a refusal.
fn psb_ok(psb: &[u8]) -> Result<bool, ()> {
    use opendmd_core::psi::psb::{load_boxed, Which, ABI, NO_HEAP};
    if psb.len() < 6 || &psb[..3] != b"PSB" || psb[3] != ABI {
        return Ok(false);
    }
    for w in [Which::Play, Which::Look] {
        match load_boxed(psb, w) {
            Ok(p) => drop(p),
            Err(e) if e == NO_HEAP => return Err(()),
            Err(_) => return Ok(false),
        }
    }
    Ok(true)
}

/// The heap a check needs: the bytes, one program (its box is ~10.5 KB at
/// most) in one block, and what CPU0's other card work leaves for the
/// player and the network (`card_formulas`' 20 KB).
fn room(psb_size: usize) -> bool {
    crate::state::free_heap() as usize >= psb_size + 12 * 1024 + 20 * 1024 && crate::state::largest_free_block() >= 12 * 1024
}

impl Client {
    /// A turn's housekeeping: a stale add's part files go now.
    pub(super) fn formula_tick(&mut self) {
        if let Some(id) = self.adds.expire(core_now()) {
            log::info!("formula-add: an add left 10 minutes without progress: dropped");
            remove_parts(id);
        }
    }

    #[optimize(size)]
    #[inline(never)]
    pub(super) fn formula_add(&mut self, k: KeyId, args: AddArgs) -> Done {
        if !self.link.status.lock().unwrap().sd_usable() {
            return Err(Refusal::NoCard);
        }
        if lan_busy() {
            return Err(Refusal::Busy);
        }
        self.formula_tick();
        if !self.tally_loaded {
            self.adds.tally = load_tally();
            self.tally_loaded = true;
        }
        let now = core_now();
        let id = args.id;
        match self.adds.pre(k, &args, now, today())? {
            Pre::Landed { stem } => return Ok(done(&stem)),
            Pre::Resume { next } => {
                if self.adds.whole(&k, id).is_some() {
                    return self.formula_land(k, id);
                }
                return Ok(Some(format!(r#"{{"next":{next}}}"#)));
            }
            Pre::New => {}
        }
        let stem = match target(&args)? {
            Err(stem) => return Ok(done(&stem)),
            Ok(stem) => stem,
        };
        if let Some(old) = self.adds.begin(k, args, stem, now) {
            remove_parts(old);
        }
        // The day's tally first (fails closed: no part file without it).
        if !save_tally(&self.adds.tally) {
            self.adds.drop_held();
            return Err(Refusal::Busy);
        }
        let _busy = crate::cache_builder::busy();
        for p in [Part::Source, Part::Psb] {
            if std::fs::File::create(part_path(id, p)).is_err() {
                self.adds.drop_held();
                remove_parts(id);
                return Err(Refusal::Busy);
            }
        }
        Ok(Some(r#"{"next":0}"#.into()))
    }

    #[optimize(size)]
    #[inline(never)]
    pub(super) fn formula_part(&mut self, k: KeyId, id: u32, off: u32, data: Vec<u8>) -> Done {
        if !self.link.status.lock().unwrap().sd_usable() {
            return Err(Refusal::NoCard);
        }
        // (A LAN transfer goes first: the part is sent again.)
        if lan_busy() {
            return Err(Refusal::Busy);
        }
        self.formula_tick();
        let p = match self.adds.part(k, id, off, &data, core_now()) {
            Ok(p) => p,
            Err(Refusal::BadArg) if self.adds.held.is_none() => {
                remove_parts(id);
                return Err(Refusal::BadArg);
            }
            Err(e) => return Err(e),
        };
        {
            let _busy = crate::cache_builder::busy();
            for (which, _, range) in &p.runs {
                let w = std::fs::OpenOptions::new().append(true).open(part_path(id, *which)).and_then(|mut f| f.write_all(&data[range.clone()]));
                if w.is_err() {
                    self.adds.drop_held();
                    remove_parts(id);
                    return Err(Refusal::Busy);
                }
            }
        }
        if p.complete {
            return self.formula_land(k, id);
        }
        Ok(Some(format!(r#"{{"next":{}}}"#, p.next)))
    }

    /// The bytes are in and their hashes match: check, then land.
    #[optimize(size)]
    #[inline(never)]
    fn formula_land(&mut self, k: KeyId, id: u32) -> Done {
        let Some(h) = self.adds.whole(&k, id) else {
            return Err(Refusal::BadArg);
        };
        let (stem, psb_size) = (h.target.clone(), h.args.psb_size as usize);
        if lan_busy() || !room(psb_size) {
            // Kept: the same formula-add signed again tries again.
            return Err(Refusal::Busy);
        }
        let mut psb = Vec::new();
        let read = psb.try_reserve_exact(psb_size).is_ok()
            && std::fs::File::open(part_path(id, Part::Psb)).and_then(|f| f.take(psb_size as u64 + 1).read_to_end(&mut psb)).is_ok();
        if !read {
            return Err(Refusal::Busy);
        }
        let checked = if psb.len() == psb_size && source_ok(&part_path(id, Part::Source)) { psb_ok(&psb) } else { Ok(false) };
        drop(psb);
        if checked.is_err() {
            return Err(Refusal::Busy);
        }
        if checked != Ok(true) {
            log::warn!("formula-add: refused: not a Formula this panel can play");
            self.adds.drop_held();
            remove_parts(id);
            return Err(Refusal::BadArg);
        }
        let d = dir();
        let (psr_to, psi_to) = (format!("{d}/{stem}{REMOTE_EXT}"), format!("{d}/{stem}.psi"));
        let _busy = crate::cache_builder::busy();
        // The program first, as a `.psr` (VM only), then its source; a rename
        // never replaces a file. Either step failing leaves nothing that plays
        // on the JIT, and nothing at all from this add.
        if std::fs::rename(part_path(id, Part::Psb), &psr_to).is_err() {
            self.adds.drop_held();
            remove_parts(id);
            return Err(Refusal::Busy);
        }
        if std::fs::rename(part_path(id, Part::Source), &psi_to).is_err() {
            let _ = std::fs::remove_file(&psr_to);
            self.adds.drop_held();
            remove_parts(id);
            return Err(Refusal::Busy);
        }
        crate::fatidx::note_written(&psi_to);
        log::info!("formula-add: landed as /formulas/{stem}.psi, its program a .psr (on the VM until compiled here)");
        self.adds.landed(&stem, core_now());
        Ok(done(&stem))
    }

    /// `k` was removed (remotely or on the LAN): its add goes, part files too.
    pub(super) fn formula_forget(&mut self, k: &KeyId) {
        if let Some(id) = self.adds.forget_key(k) {
            remove_parts(id);
        }
    }
}
