//! The last late frames' details (`/api/status` `misses.recent`): each
//! late frame's phases as the miss attribution saw them (`frametime`), and
//! the tick's split (the file's advance, the transition's step:
//! `opendmd_core::engine::tick_split`), the last 8. What a cause alone does
//! not say (a "transition" miss: the incoming file's open, its decoding, or
//! the dissolve's compositing).

use std::sync::Mutex;

#[derive(Clone, Copy)]
pub struct Miss {
    pub late_ms: u64,
    pub cause: usize,
    pub phases: [u32; 7],
    pub transitioning: bool,
    pub advance_us: u32,
    pub step_us: u32,
    pub at_s: u64,
    /// The display core's card reads this frame: µs waiting for the card's
    /// I/O lock (another task's transfer), µs transferring (`cardspeed`).
    pub card_wait_us: u32,
    pub card_xfer_us: u32,
    /// The decoder of the file on screen (`Engine::playing_kind`).
    pub kind: &'static str,
    /// Its opens by step: through the FAT index, by path (FatFs, behind
    /// the volume lock), and waiting for the cache builder to let go of
    /// its heap (µs).
    pub open_index_us: u32,
    pub open_path_us: u32,
    pub open_builder_us: u32,
    /// The playlist's lookup of the next file (its index and manifest).
    pub open_catalog_us: u32,
    /// What the network side was doing (net-wedge2): an upload in hand
    /// (1), a transfer sharing the card in flight (2: the cache builder
    /// stopped, nothing opened ahead), the read-ahead ring held (4).
    pub net: u8,
    /// The free heap at the miss.
    pub heap: u32,
}

/// Late frames while a transfer shared the card (`net` & 3), by cause
/// (`frametime::CAUSES` order), and the file switches made meanwhile (each
/// opened in its frame: nothing is opened ahead during a transfer).
static IN_TRANSFER: [std::sync::atomic::AtomicU32; 8] = [const { std::sync::atomic::AtomicU32::new(0) }; 8];
static SWITCHES_IN_TRANSFER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// What [`Miss::net`] says now (the main task; atomics only).
pub fn net_now() -> u8 {
    (crate::stream::uploads() > 0) as u8
        | ((crate::cache_builder::transfer_in_flight() as u8) << 1)
        | ((crate::readahead::active() as u8) << 2)
}

/// A file switch: counted if a transfer is in flight.
pub fn switched() {
    if crate::cache_builder::transfer_in_flight() {
        SWITCHES_IN_TRANSFER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// `,"transfer":{"open":N,…,"switches":N}`: late frames during a transfer.
pub fn transfer_json() -> String {
    use std::sync::atomic::Ordering::Relaxed;
    let mut out = String::from(r#","transfer":{"#);
    for (name, n) in opendmd_core::frametime::CAUSES.iter().zip(&IN_TRANSFER) {
        out.push_str(&format!(r#""{name}":{},"#, n.load(Relaxed)));
    }
    out.push_str(&format!(r#""switches":{}}}"#, SWITCHES_IN_TRANSFER.load(Relaxed)));
    out
}

static RECENT: Mutex<([Option<Miss>; 8], usize)> = Mutex::new(([None; 8], 0));

/// The main task, at a deadline miss (rare: a lock taken only then).
pub fn note(m: Miss) {
    if m.net & 3 != 0 {
        IN_TRANSFER[m.cause.min(7)].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if let Ok(mut g) = RECENT.try_lock() {
        let k = g.1 % 8;
        g.0[k] = Some(m);
        g.1 += 1;
    }
}

/// `,"recent":[{late_ms, cause, open, read, decode, blit, log, other, wake,
/// transitioning, advance_us, step_us, at_s, card_wait_us, card_xfer_us,
/// kind, open_index_us, open_path_us, open_builder_us, open_catalog_us}]`, oldest first.
pub fn json() -> String {
    let g = RECENT.lock().unwrap_or_else(|p| p.into_inner());
    let n = g.1;
    let mut out = String::from(r#","recent":["#);
    let mut first = true;
    for i in n.saturating_sub(8)..n {
        let Some(m) = g.0[i % 8] else { continue };
        if !first {
            out.push(',');
        }
        first = false;
        let p = m.phases;
        out.push_str(&format!(
            r#"{{"late_ms":{},"cause":"{}","open":{},"read":{},"decode":{},"blit":{},"log":{},"other":{},"wake":{},"transitioning":{},"advance_us":{},"step_us":{},"at_s":{},"card_wait_us":{},"card_xfer_us":{},"kind":"{}","open_index_us":{},"open_path_us":{},"open_builder_us":{},"open_catalog_us":{},"net":{},"heap":{}}}"#,
            m.late_ms,
            opendmd_core::frametime::CAUSES[m.cause.min(7)],
            p[0], p[1], p[2], p[3], p[4], p[5], p[6],
            m.transitioning, m.advance_us, m.step_us, m.at_s, m.card_wait_us, m.card_xfer_us, m.kind, m.open_index_us, m.open_path_us, m.open_builder_us, m.open_catalog_us, m.net, m.heap
        ));
    }
    out.push(']');
    out
}
