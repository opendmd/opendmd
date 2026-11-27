//! A Formula added by a remote controller (`formula-add`, `formula-part`;
//! protocol 5.7): the card's half of `opendmd_core::connect::fadd`.
//!
//! - **Start:** the stem, or the next free `-2` … `-9`: a stem is free when
//!   neither its `.psi` nor its `.psb` opens (an open each, never a stat);
//!   a taken stem whose `.psi` has the signed source's SHA-256 is the same
//!   Formula already there: `done` with nothing written. The bytes go to
//!   `/formulas/.add-<id>.psi.part` and `.add-<id>.psb.part` (dot names:
//!   never listed or played; removed at boot).
//! - **The last part:** both hashes (the core's), the source UTF-8 with no
//!   NUL, the compiled file `PSB` + this player's ABI, and every program in
//!   it loaded and checked by `psb::load_boxed` (`psb::verify`), behind a
//!   heap gate (the bytes, one program and 20 KB for the rest; below it, or
//!   a load short of heap, `busy`, the bytes kept for the same
//!   `formula-add` signed again).
//! - **Landing, never overwriting:** the `.psb` part renamed to
//!   `<stem>.psb` and the `.psi` part to `<stem>.psi` (a rename fails onto
//!   a name that exists), the `.psb` dated 1980 (older than any source: the
//!   next start finds it stale), and the Formula noted as added by remote
//!   (`card_formulas::note_remote`): on the VM only, never the JIT, until
//!   the next start's pass compiles its source here.

use super::client::{core_now, Client};
use crate::config;
use opendmd_core::connect::fadd::{self, AddArgs, Part, Start};
use opendmd_core::connect::keys::KeyId;
use opendmd_core::connect::proto::Refusal;
use std::io::{Read, Write};

type Done = Result<Option<String>, Refusal>;

fn dir() -> String {
    format!("{}/formulas", config::SD_MOUNT)
}

fn part_path(id: u32, p: Part) -> String {
    format!("{}/.add-{id}.{}.part", dir(), if p == Part::Source { "psi" } else { "psb" })
}

fn remove_parts(id: u32) {
    let _ = std::fs::remove_file(part_path(id, Part::Source));
    let _ = std::fs::remove_file(part_path(id, Part::Psb));
}

/// Where `args` lands: `Ok(Err(stem))` the same source is there already
/// (done, nothing written); `Ok(Ok(stem))` a free stem.
fn target(args: &AddArgs) -> Result<Result<String, String>, Refusal> {
    let d = dir();
    for stem in fadd::stem_choices(&args.stem) {
        match std::fs::File::open(format!("{d}/{stem}.psi")) {
            Ok(f) => {
                if fadd::sha256_of(f).ok() == Some(args.src_sha256) {
                    return Ok(Err(stem));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // (A compiled file with no source is taken too.)
                if std::fs::File::open(format!("{d}/{stem}.psb")).is_err() {
                    return Ok(Ok(stem));
                }
            }
            Err(_) => return Err(Refusal::Busy),
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

/// The compiled part: this player's format, every program in it loaded
/// and checked as anything from the network is (`psb::verify`). `Err`:
/// the heap had no room for a program (try again), not a refusal.
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

/// The heap a check needs: the compiled bytes, one program (its box is
/// ~10.5 KB at most) in one block, and what CPU0's other card work leaves
/// for the player and the network (`card_formulas`' 20 KB).
fn room(psb_size: usize) -> bool {
    crate::state::free_heap() as usize >= psb_size + 12 * 1024 + 20 * 1024 && crate::state::largest_free_block() >= 12 * 1024
}

impl Client {
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn formula_add(&mut self, k: KeyId, args: AddArgs) -> Done {
        if !self.link.status.lock().unwrap().sd_usable() {
            return Err(Refusal::NoCard);
        }
        if crate::cache_builder::transfer_in_flight() {
            return Err(Refusal::Busy);
        }
        let now = core_now();
        if let Some(old) = self.adds.expire(now) {
            remove_parts(old);
        }
        // The same add signed again: no new target looked for.
        let again = self.adds.held.as_ref().is_some_and(|h| h.k == k && h.args == args);
        let dest = if again {
            None
        } else {
            match target(&args)? {
                Err(stem) => return Ok(done(&stem)),
                Ok(stem) => Some(stem),
            }
        };
        let id = args.id;
        match self.adds.start(k, args, dest, now)? {
            Start::Landed { stem } => Ok(done(&stem)),
            Start::Resume { next } => {
                if self.adds.whole(&k, id).is_some() {
                    return self.formula_land(k, id);
                }
                Ok(Some(format!(r#"{{"next":{next}}}"#)))
            }
            Start::Fresh { dropped } => {
                if let Some(old) = dropped {
                    remove_parts(old);
                }
                for p in [Part::Source, Part::Psb] {
                    if std::fs::File::create(part_path(id, p)).is_err() {
                        self.adds.drop_held();
                        remove_parts(id);
                        return Err(Refusal::Busy);
                    }
                }
                Ok(Some(r#"{"next":0}"#.into()))
            }
        }
    }

    #[optimize(size)]
    #[inline(never)]
    pub(super) fn formula_part(&mut self, k: KeyId, id: u32, off: u32, data: Vec<u8>) -> Done {
        if !self.link.status.lock().unwrap().sd_usable() {
            return Err(Refusal::NoCard);
        }
        let p = match self.adds.part(k, id, off, &data, core_now()) {
            Ok(p) => p,
            // (Bytes that missed a signed hash: the add is gone, its parts too.)
            Err(Refusal::BadArg) if self.adds.held.is_none() => {
                remove_parts(id);
                return Err(Refusal::BadArg);
            }
            Err(e) => return Err(e),
        };
        for (which, _, range) in &p.runs {
            let w = std::fs::OpenOptions::new().append(true).open(part_path(id, *which)).and_then(|mut f| f.write_all(&data[range.clone()]));
            if w.is_err() {
                self.adds.drop_held();
                remove_parts(id);
                return Err(Refusal::Busy);
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
        if !room(psb_size) {
            // Kept: the same formula-add signed again tries the checks again.
            return Err(Refusal::Busy);
        }
        let mut psb = Vec::new();
        let read = psb.try_reserve_exact(psb_size).is_ok()
            && std::fs::File::open(part_path(id, Part::Psb)).and_then(|f| f.take(psb_size as u64 + 1).read_to_end(&mut psb)).is_ok();
        if !read {
            return Err(Refusal::Busy);
        }
        let checked = if psb.len() == psb_size && source_ok(&part_path(id, Part::Source)) { psb_ok(&psb) } else { Ok(false) };
        if checked.is_err() {
            return Err(Refusal::Busy);
        }
        if checked != Ok(true) {
            log::warn!("connect: a Formula added by remote refused: not one this panel can play");
            self.adds.drop_held();
            remove_parts(id);
            return Err(Refusal::BadArg);
        }
        let crc = opendmd_core::settings_file::crc32(&psb);
        drop(psb);
        let d = dir();
        let (psb_to, psi_to) = (format!("{d}/{stem}.psb"), format!("{d}/{stem}.psi"));
        // (std's rename onto a name that exists fails on FAT: never an overwrite.)
        if std::fs::rename(part_path(id, Part::Psb), &psb_to).is_err() {
            self.adds.drop_held();
            remove_parts(id);
            return Err(Refusal::Busy);
        }
        if std::fs::rename(part_path(id, Part::Source), &psi_to).is_err() {
            let _ = std::fs::remove_file(&psb_to);
            self.adds.drop_held();
            remove_parts(id);
            return Err(Refusal::Busy);
        }
        old_date(&psb_to);
        if let Err(e) = crate::card_formulas::note_remote(&stem, crc) {
            log::warn!("connect: noting a Formula added by remote failed ({e}); it stays on the VM until a restart compiles it");
        }
        log::info!("connect: a Formula added by remote landed as /formulas/{stem}.psi (on the VM until compiled here)");
        self.adds.landed(&stem, core_now());
        Ok(done(&stem))
    }

    /// `k` was removed: its add goes.
    pub(super) fn formula_forget(&mut self, k: &KeyId) {
        if let Some(id) = self.adds.forget_key(k) {
            remove_parts(id);
        }
    }
}

/// Date `path` 1980-01-01 (FAT's first day): older than any source.
fn old_date(path: &str) {
    use esp_idf_sys as sys;
    let card = path.strip_prefix(config::SD_MOUNT).unwrap_or(path);
    let Ok(c) = std::ffi::CString::new(format!("0:{card}")) else {
        return;
    };
    // SAFETY: a zeroed FILINFO with the date fields set; a NUL-terminated path.
    unsafe {
        let mut fi: sys::FILINFO = std::mem::zeroed();
        fi.fdate = (1 << 5) | 1;
        fi.ftime = 0;
        sys::f_utime(c.as_ptr(), &fi);
    }
}
