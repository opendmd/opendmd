//! Admission for the device's heavy HTTP work: whether a request that is
//! about to take a block of the heap may start now, or is answered
//! `503 Service Unavailable` with a `Retry-After` instead.
//!
//! The firmware's handlers run on two tasks (httpd, one handler at a time,
//! and the stream task, one transfer at a time), so how many heavy requests
//! run at once is already bounded by construction. What a count cannot see
//! is the heap they share with the player: a GIF opening, a Formula's
//! still cache or JIT kernel, the WiFi driver's receive buffers (taken
//! from the same heap as frames arrive, up to ~13.6 KB with eight of them
//! held). Two numbers decide: the free heap, and the **largest free
//! block** — a heap with 39 KB free can still have no 9 KB block (the
//! rig's restart ring: `last failed alloc: 9069 B at 1381 s, 39148 B
//! free`), and an allocation Rust cannot take aborts the board.
//!
//! A request names the biggest single block it will take ([`Ask::block`])
//! and what it will hold in all at its peak ([`Ask::total`]); [`admit`]
//! keeps [`FLOOR`] free beyond that for the player and the radio, more
//! while a transfer is under way on the stream task (its receive buffers
//! come and go with the traffic: [`TRANSFER_SWING`] each), and asks for a
//! block with [`BLOCK_SLACK`] to spare (the heap's own headers, and the
//! next small allocation landing in it first). Pure: the firmware passes
//! the numbers in, the host tests them.

/// The free heap kept beyond what a request will hold, bytes: the stream
/// task's start threshold (`config::STREAM_START_HEAP`, 5 KB of reserve +
/// 8 KB) — below it lwIP and the WiFi driver stop delivering frames on
/// this board (status reads lost, replies 10 s late: net-wedge2).
pub const FLOOR: u32 = 13 * 1024;

/// What a transfer under way on the stream task may still take on top of
/// what the free heap shows now: its receive frames come and go with the
/// traffic (up to eight WiFi buffers of ~1.7 KB, some already held when
/// the door reads the heap), its chunk buffer and file handle are already
/// counted.
pub const TRANSFER_SWING: u32 = 4 * 1024;

/// The largest free block must exceed the block asked for by this much.
pub const BLOCK_SLACK: u32 = 1024;

/// What a refused request is told to wait, seconds (`Retry-After`): the
/// time a 300 KB upload takes at the card's pace, or a GIF's switch.
pub const RETRY_AFTER_S: u8 = 2;

/// A request's ask of the heap, bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ask {
    /// The largest single allocation it will make (a body's buffer, a
    /// chunk buffer).
    pub block: u32,
    /// All it will hold at its peak (the buffer, a parse of it, a reply).
    pub total: u32,
}

impl Ask {
    /// A request that reads a body of `len` bytes whole and parses it (a
    /// JSON body: the bytes, then about as much again in the values made
    /// from them, and a reply).
    pub fn body(len: u32) -> Ask {
        Ask {
            block: len,
            total: len.saturating_mul(2).saturating_add(1024),
        }
    }

    /// A transfer that streams through one chunk buffer of `chunk` bytes
    /// (an upload or download on the stream task): the block for the
    /// buffer, and [`FLOOR`] is its start threshold as it always was (the
    /// buffer, a file handle and the radio's frames come out of that).
    pub fn transfer(chunk: u32) -> Ask {
        Ask {
            block: chunk,
            total: 0,
        }
    }

    /// A compiled Formula of `len` bytes held for the player (`POST
    /// /api/play/formula`): the bytes alone here (the player's open takes
    /// its own memory fallibly and refuses on a short heap).
    pub fn held(len: u32) -> Ask {
        Ask {
            block: len,
            total: len,
        }
    }
}

/// The heap as the firmware reads it at the door.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heap {
    pub free: u32,
    pub largest: u32,
    /// Transfers under way on the stream task (0 or 1).
    pub transfers: u32,
}

/// Why a request was refused (for the log line; the client hears a 503
/// and the [`RETRY_AFTER_S`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Too little free heap for it and the floor.
    Short { need: u32 },
    /// Free heap enough, but no block that big (fragmented).
    NoBlock { need: u32 },
}

impl Refusal {
    pub fn why(&self) -> &'static str {
        match self {
            Refusal::Short { .. } => "short of heap",
            Refusal::NoBlock { .. } => "no block that size",
        }
    }
    pub fn need(&self) -> u32 {
        match *self {
            Refusal::Short { need } | Refusal::NoBlock { need } => need,
        }
    }
}

/// May a request asking `ask` start with the heap at `heap`?
pub fn admit(ask: Ask, heap: Heap) -> Result<(), Refusal> {
    let need = ask
        .total
        .saturating_add(FLOOR)
        .saturating_add(heap.transfers.saturating_mul(TRANSFER_SWING));
    if heap.free < need {
        return Err(Refusal::Short { need });
    }
    let block = ask.block.saturating_add(BLOCK_SLACK);
    if ask.block > 0 && heap.largest < block {
        return Err(Refusal::NoBlock { need: block });
    }
    Ok(())
}

/// The sentence a refused client reads (no word the UI may not show).
pub const BUSY: &str = "the player needs the memory right now; retry shortly";

#[cfg(test)]
mod tests {
    use super::*;

    fn heap(free: u32, largest: u32) -> Heap {
        Heap {
            free,
            largest,
            transfers: 0,
        }
    }

    #[test]
    fn a_roomy_heap_admits() {
        assert_eq!(admit(Ask::body(2048), heap(60_000, 30_000)), Ok(()));
        assert_eq!(admit(Ask::transfer(2048), heap(20_000, 8_000)), Ok(()));
        assert_eq!(admit(Ask::held(9069), heap(40_000, 16_384)), Ok(()));
    }

    #[test]
    fn the_rigs_fragmented_heap_refuses_a_9k_block() {
        // The restart ring's failure: 39 KB free, the largest block under
        // the 9,069 B asked for.
        let r = admit(Ask::held(9069), heap(39_148, 8_192));
        assert_eq!(
            r,
            Err(Refusal::NoBlock {
                need: 9069 + BLOCK_SLACK
            })
        );
        assert_eq!(r.unwrap_err().why(), "no block that size");
    }

    #[test]
    fn near_exhaustion_refuses_even_a_small_body() {
        // The ring's other: 2,332 B asked with 8,864 B free.
        let r = admit(Ask::body(2332), heap(8_864, 4_000));
        assert!(matches!(r, Err(Refusal::Short { .. })));
    }

    #[test]
    fn the_floor_is_kept_beyond_the_ask() {
        let ask = Ask::transfer(2048);
        let edge = ask.total + FLOOR;
        assert_eq!(admit(ask, heap(edge, 64_000)), Ok(()));
        assert_eq!(
            admit(ask, heap(edge - 1, 64_000)),
            Err(Refusal::Short { need: edge })
        );
    }

    #[test]
    fn a_transfer_under_way_counts_against_the_rest() {
        let ask = Ask::body(4096);
        let free = ask.total + FLOOR + 1000;
        assert_eq!(admit(ask, heap(free, 64_000)), Ok(()));
        let busy = Heap {
            transfers: 1,
            ..heap(free, 64_000)
        };
        assert_eq!(
            admit(ask, busy),
            Err(Refusal::Short {
                need: ask.total + FLOOR + TRANSFER_SWING
            })
        );
    }

    #[test]
    fn the_block_needs_its_slack() {
        let ask = Ask::held(9000);
        assert_eq!(admit(ask, heap(64_000, 9000 + BLOCK_SLACK)), Ok(()));
        assert!(matches!(
            admit(ask, heap(64_000, 9000 + BLOCK_SLACK - 1)),
            Err(Refusal::NoBlock { .. })
        ));
    }

    #[test]
    fn no_overflow_at_the_edges() {
        let r = admit(Ask::body(u32::MAX), heap(u32::MAX - 1, u32::MAX - 1));
        assert!(r.is_err());
        assert_eq!(admit(Ask { block: 0, total: 0 }, heap(FLOOR, 0)), Ok(()));
    }

    #[test]
    fn the_busy_sentence_says_no_hidden_word() {
        for w in ["cache", "copy", "bucket", "odm"] {
            assert!(!BUSY.contains(w), "{w}");
        }
    }
}
