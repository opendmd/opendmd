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
}

static RECENT: Mutex<([Option<Miss>; 8], usize)> = Mutex::new(([None; 8], 0));

/// The main task, at a deadline miss (rare: a lock taken only then).
pub fn note(m: Miss) {
    if let Ok(mut g) = RECENT.try_lock() {
        let k = g.1 % 8;
        g.0[k] = Some(m);
        g.1 += 1;
    }
}

/// `,"recent":[{late_ms, cause, open, read, decode, blit, log, other, wake,
/// transitioning, advance_us, step_us, at_s, card_wait_us, card_xfer_us,
/// kind}]`, oldest first.
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
            r#"{{"late_ms":{},"cause":"{}","open":{},"read":{},"decode":{},"blit":{},"log":{},"other":{},"wake":{},"transitioning":{},"advance_us":{},"step_us":{},"at_s":{},"card_wait_us":{},"card_xfer_us":{},"kind":"{}"}}"#,
            m.late_ms,
            opendmd_core::frametime::CAUSES[m.cause.min(7)],
            p[0], p[1], p[2], p[3], p[4], p[5], p[6],
            m.transitioning, m.advance_us, m.step_us, m.at_s, m.card_wait_us, m.card_xfer_us, m.kind
        ));
    }
    out.push(']');
    out
}
