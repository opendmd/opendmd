//! OpenDMD Connect: remote control through a relay (`docs/connect-protocol.md`).
//!
//! - [`ws`]: the session half from spike S0 (RFC 6455 framing, the upgrade
//!   and its subprotocols, the register request, backoff, the measuring
//!   streams); re-exported here as it was.
//! - [`proto`]: protocol v1: versions, the Connect id, the signed command
//!   envelope, sessions, counters and rates, the remote allow-list, the
//!   text messages.
//! - [`keys`]: the controller list and its NVS blob.
//! - [`pair`]: pairing windows, MACs and invites.
//! - [`qr`], [`screen`]: the pairing QR code (version 3-L) and the panel's
//!   two pairing screens.
//! - [`xfer`]: a GIF send's chunks, credit, hashes and quota.
//! - [`codec`], [`json`]: hex, base32, HMAC; a strict JSON reader.
//!
//! The firmware's client is `firmware/src/connect/` (feature `connect`).

pub mod codec;
pub mod json;
pub mod keys;
pub mod pair;
pub mod proto;
pub mod qr;
pub mod screen;
pub mod ws;
pub mod xfer;

pub use ws::*;
