//! The heap gate: whether the panel has the memory to start a TLS handshake
//! now, and if not, which condition is unmet (`GET /api/connect`
//! `waiting`).
//!
//! What a handshake takes (spike S0, `S0-RESULTS.md` in the maintainers'
//! notes): mbedTLS holds 12.0 KB at its peak, for about a second (the 4 KB
//! input record and the 2 KB output one with their overheads, the ECDHE
//! and ECDSA working numbers, the chain's certificates parsed one at a
//! time), the same on the classic and the S3. Around it, in internal RAM
//! whatever the build: the socket and lwIP's segments, the WiFi driver's
//! frames (unless SPIRAM_TRY_ALLOCATE_WIFI_LWIP), the worker's own Rust
//! buffers (the WebSocket decoder, the upgrade's request and answer).
//!
//! - [`CLASSIC`]: 48 KB free for a full handshake, 44 KB for a resumed one,
//!   a 16 KB block for either (S0's recommendation: leaves about 36 KB
//!   during one, above where the player starts shedding).
//! - [`S3_INTERNAL_TLS`]: an S3 whose mbedTLS allocates internal RAM:
//!   100 KB, open in practice (its internal heap is three times the
//!   classic's with a session open).
//! - [`S3_PSRAM_TLS`]: an S3 whose mbedTLS allocates from PSRAM (feature
//!   `psram-tls`, `CONFIG_MBEDTLS_EXTERNAL_MEM_ALLOC`). The 12 KB is no
//!   longer internal, so 100 KB of internal free is the wrong question
//!   (the carrier image's internal heap sits near 79 KB and never reaches
//!   it). PSRAM must have a 64 KB block, five times the handshake's whole
//!   peak in one piece (every one of its allocations fits in it with room;
//!   an 8 MB PSRAM always has it, so in practice this only stops a
//!   handshake when PSRAM is missing or broken). There is no internal
//!   fallback to budget for: IDF's `EXTERNAL_MEM_ALLOC` allocator asks
//!   `MALLOC_CAP_SPIRAM` only, so with no PSRAM block mbedTLS's allocation
//!   fails and the handshake with it. Only a `connect-measure` build
//!   installs the firmware's own allocator (`tls_calloc`: PSRAM, then
//!   internal RAM, counted); a shipping build uses it for one struct, the
//!   pinned root the verify callback parses (a few hundred bytes).
//!
//!   The internal free gates (48 KB full, 44 KB resumed) are the
//!   classic's, borrowed until the S3 is measured on its own: everything
//!   but mbedTLS still lands in internal RAM. The rig's evidence so far
//!   (carrier-connect, 2026-11-16): a handshake dipped internal free by
//!   about 22 KB, from 79 KB to a floor of 57 KB, so 48 KB leaves about
//!   26 KB through one. The internal largest-block gate is 8 KB, not the
//!   classic's 16 KB: the 16 KB was for mbedTLS's own 4 KB input record
//!   and working numbers, and no internal allocation on the psram-tls
//!   path is near it. The largest are a WiFi or lwIP frame (about 1.6 KB),
//!   the worker's binary message buffer (`BIN_KEEP`, about 1 KB), the
//!   upgrade's answer head (up to 1 KB) and the mbedTLS context structs
//!   the worker boxes (under 1 KB); 8 KB is five times the largest of
//!   them, as the PSRAM gate is to its peak.

/// The heap as the gate reads it, bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heap {
    /// Internal free (`heap_caps_get_free_size(INTERNAL | 8BIT)` on the
    /// S3; the whole heap on the classic, which has no PSRAM).
    pub free: u32,
    /// The largest internal block.
    pub largest: u32,
    /// PSRAM's largest block (0 without PSRAM, or where it is not read).
    pub psram_largest: u32,
}

/// The thresholds, bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gates {
    /// Internal free for a full handshake.
    pub full: u32,
    /// Internal free for a resumed one (a TLS ticket held).
    pub resumed: u32,
    /// The largest internal block, either kind.
    pub largest: u32,
    /// PSRAM's largest block; 0: not gated (mbedTLS allocates internal).
    pub psram_largest: u32,
}

const KB: u32 = 1024;

/// The classic ESP32 (mbedTLS in internal RAM; no PSRAM).
pub const CLASSIC: Gates = Gates { full: 48 * KB, resumed: 44 * KB, largest: 16 * KB, psram_largest: 0 };
/// An ESP32-S3 with mbedTLS in internal RAM.
pub const S3_INTERNAL_TLS: Gates = Gates { full: 100 * KB, resumed: 100 * KB, largest: 16 * KB, psram_largest: 0 };
/// An ESP32-S3 with mbedTLS in PSRAM (`psram-tls`).
/// Internal free borrowed from [`CLASSIC`] pending an S3 measurement;
/// largest block 8 KB (the module's notes: what the internal side holds).
pub const S3_PSRAM_TLS: Gates = Gates { full: 48 * KB, resumed: 44 * KB, largest: 8 * KB, psram_largest: 64 * KB };

/// The first condition a handshake does not meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmet {
    /// Internal free under the full or resumed gate.
    Free { have: u32, need: u32 },
    /// The largest internal block under its gate.
    Largest { have: u32, need: u32 },
    /// PSRAM's largest block under its gate.
    PsramLargest { have: u32, need: u32 },
}

impl Unmet {
    /// For `GET /api/connect` `waiting`: "internal heap 79 KB < 100 KB".
    /// KB rounded down, so the two sides never print equal.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn describe(&self) -> String {
        let (what, have, need) = match *self {
            Unmet::Free { have, need } => ("internal heap", have, need),
            Unmet::Largest { have, need } => ("internal largest block", have, need),
            Unmet::PsramLargest { have, need } => ("PSRAM largest block", have, need),
        };
        // `have < need`: show bytes where whole KB would read the same.
        if have / KB < need / KB {
            format!("{what} {} KB < {} KB", have / KB, need / KB)
        } else {
            format!("{what} {have} B < {need} B")
        }
    }
}

/// Whether a handshake (`resumed`: with a ticket) may start on `h`.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn check(g: &Gates, h: &Heap, resumed: bool) -> Result<(), Unmet> {
    let need = if resumed { g.resumed } else { g.full };
    if h.free < need {
        return Err(Unmet::Free { have: h.free, need });
    }
    if h.largest < g.largest {
        return Err(Unmet::Largest { have: h.largest, need: g.largest });
    }
    if g.psram_largest > 0 && h.psram_largest < g.psram_largest {
        return Err(Unmet::PsramLargest { have: h.psram_largest, need: g.psram_largest });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The S3 rig on carrier-connect, `/api/status` heap (2026-11-16):
    /// free 79196, internal_largest 31744, psram largest 8257536.
    const RIG: Heap = Heap { free: 79_196, largest: 31_744, psram_largest: 8_257_536 };

    #[test]
    fn the_rigs_heap_passes_the_psram_tls_gates() {
        assert_eq!(check(&S3_PSRAM_TLS, &RIG, false), Ok(()));
        assert_eq!(check(&S3_PSRAM_TLS, &RIG, true), Ok(()));
    }

    #[test]
    fn the_rigs_heap_fails_the_internal_tls_gate_and_says_why() {
        let e = check(&S3_INTERNAL_TLS, &RIG, false).unwrap_err();
        assert_eq!(e, Unmet::Free { have: 79_196, need: 100 * 1024 });
        assert_eq!(e.describe(), "internal heap 77 KB < 100 KB");
    }

    #[test]
    fn the_classic_is_gated_as_before() {
        // S0's GIF-play median with a session open: about 41 KB.
        let busy = Heap { free: 41_000, largest: 20_000, psram_largest: 0 };
        assert_eq!(check(&CLASSIC, &busy, false), Err(Unmet::Free { have: 41_000, need: 48 * 1024 }));
        assert_eq!(check(&CLASSIC, &busy, true), Err(Unmet::Free { have: 41_000, need: 44 * 1024 }));
        // 46 KB: a resumed handshake only.
        let mid = Heap { free: 46 * 1024, largest: 20_000, psram_largest: 0 };
        assert!(check(&CLASSIC, &mid, false).is_err());
        assert_eq!(check(&CLASSIC, &mid, true), Ok(()));
        // Enough free, fragmented.
        let frag = Heap { free: 60 * 1024, largest: 12 * 1024, psram_largest: 0 };
        assert_eq!(check(&CLASSIC, &frag, false), Err(Unmet::Largest { have: 12 * 1024, need: 16 * 1024 }));
        // Exactly at the gates passes; no PSRAM is never asked about.
        let edge = Heap { free: 48 * 1024, largest: 16 * 1024, psram_largest: 0 };
        assert_eq!(check(&CLASSIC, &edge, false), Ok(()));
    }

    #[test]
    fn psram_tls_still_needs_internal_room() {
        let low = Heap { free: 40 * 1024, ..RIG };
        assert_eq!(check(&S3_PSRAM_TLS, &low, false), Err(Unmet::Free { have: 40 * 1024, need: 48 * 1024 }));
        let frag = Heap { largest: 6 * 1024, ..RIG };
        assert_eq!(check(&S3_PSRAM_TLS, &frag, true).unwrap_err().describe(), "internal largest block 6 KB < 8 KB");
        // 8 KB passes here; the classic still asks 16 KB (mbedTLS internal).
        let eight = Heap { largest: 8 * 1024, ..RIG };
        assert_eq!(check(&S3_PSRAM_TLS, &eight, false), Ok(()));
        assert!(check(&CLASSIC, &Heap { psram_largest: 0, ..eight }, false).is_err());
    }

    #[test]
    fn psram_tls_without_psram_waits() {
        let none = Heap { psram_largest: 0, ..RIG };
        let e = check(&S3_PSRAM_TLS, &none, false).unwrap_err();
        assert_eq!(e, Unmet::PsramLargest { have: 0, need: 64 * 1024 });
        assert_eq!(e.describe(), "PSRAM largest block 0 KB < 64 KB");
    }

    #[test]
    fn a_shortfall_inside_one_kilobyte_prints_bytes() {
        // A measuring build's gate need not be whole KB.
        let e = Unmet::Free { have: 48 * 1024 + 100, need: 48 * 1024 + 500 };
        assert_eq!(e.describe(), "internal heap 49252 B < 49652 B");
        let e = Unmet::Free { have: 48 * 1024 - 10, need: 48 * 1024 };
        assert_eq!(e.describe(), "internal heap 47 KB < 48 KB");
    }
}
