//! OpenDMD Connect: remote control through a relay (`docs/connect-protocol.md`).
//!
//! - [`ws`]: the session half from spike S0 (RFC 6455 framing, the upgrade,
//!   backoff, the measuring streams); re-exported here as it was.
//! - [`proto`]: protocol v1: versions, the Connect id, the signed command
//!   envelope, sessions, counters and rates, the remote allow-list, the
//!   text messages.
//! - [`keys`]: the controller list and its NVS blob.
//! - [`pair`]: pairing windows, MACs and invites.
//! - [`xfer`]: a GIF send's chunks, credit, hashes and quota.
//! - [`codec`], [`json`]: hex, base32, HMAC; a strict JSON reader.
//!
//! Host-only for now: the firmware wires it in later (P1-PLAN F2).

pub mod codec;
pub mod json;
pub mod keys;
pub mod pair;
pub mod proto;
pub mod ws;
pub mod xfer;

pub use ws::*;
