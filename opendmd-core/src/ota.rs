//! The signed firmware bundle (`docs/ota.md`): its 140-byte header, the
//! Ed25519 check of that header against the public key compiled into the
//! firmware, and a streaming SHA-256 so the device hashes the image as it
//! writes it to the card. `tools/fw-sign.py` is the other implementation
//! (it signs); the known-answer test below holds a bundle it made.
//!
//! ```text
//! offset  size  field
//!      0     8  magic "ODFW1\0\0\0"
//!      8    32  version, UTF-8, NUL-padded (at least one byte, no NUL inside)
//!     40     4  length of the image, u32 LE
//!     44    32  SHA-256 of the image
//!     76    64  Ed25519 signature over bytes [0, 76)
//!    140        the app image (ESP image magic 0xE9 first)
//! ```
//!
//! Verify-only: no key generation, no `rand`. The order on the device is
//! [`Header::parse`] on the first 140 B of the body, [`verify_header`]
//! (the signature), then the image streamed through an [`ImageHasher`] and
//! checked with [`Meta::check_image`] at the end.

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fmt;

/// `ODFW1` and three NULs.
pub const MAGIC: [u8; 8] = *b"ODFW1\0\0\0";
/// The version field's size.
pub const VERSION_LEN: usize = 32;
/// The signed part of the header: magic, version, length, digest.
pub const SIGNED_LEN: usize = 8 + VERSION_LEN + 4 + 32;
/// The whole header.
pub const HEADER_LEN: usize = SIGNED_LEN + 64;
/// The first byte of every ESP-IDF app image.
pub const ESP_IMAGE_MAGIC: u8 = 0xE9;
/// An Ed25519 public key.
pub const PUBLIC_KEY_LEN: usize = 32;

/// Why a bundle is refused. Each says which check failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OtaError {
    /// Fewer bytes than the header.
    Truncated { have: usize },
    BadMagic,
    /// Empty, not UTF-8, or a NUL before the padding.
    BadVersion,
    /// The compiled-in public key is not a valid point.
    BadKey,
    /// The signature does not verify: wrong key, or the header was changed.
    BadSignature,
    /// The image's length is not the header's.
    LengthMismatch { header: u32, actual: u64 },
    /// The image is larger than the partition it goes into.
    TooLarge { length: u32, partition: u32 },
    /// The image does not start with 0xE9.
    NotAnImage,
    /// The image's SHA-256 is not the header's.
    HashMismatch,
}

impl fmt::Display for OtaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OtaError::Truncated { have } => {
                write!(f, "truncated: {have} B, the bundle header is {HEADER_LEN} B")
            }
            OtaError::BadMagic => f.write_str("not an OpenDMD firmware bundle (bad magic)"),
            OtaError::BadVersion => f.write_str("the bundle's version field is malformed"),
            OtaError::BadKey => f.write_str("the firmware's public key is invalid"),
            OtaError::BadSignature => f.write_str(
                "bad signature: not signed by this firmware's key, or the header was changed",
            ),
            OtaError::LengthMismatch { header, actual } => write!(
                f,
                "length mismatch: the header says {header} B, the image is {actual} B"
            ),
            OtaError::TooLarge { length, partition } => write!(
                f,
                "the image is {length} B, the firmware partition is {partition} B"
            ),
            OtaError::NotAnImage => {
                f.write_str("not an ESP32 app image (the first byte is not 0xE9)")
            }
            OtaError::HashMismatch => f.write_str("SHA-256 mismatch: the image is damaged"),
        }
    }
}

impl std::error::Error for OtaError {}

/// The header, parsed but not yet trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    version: [u8; VERSION_LEN],
    pub length: u32,
    pub sha256: [u8; 32],
    pub sig: [u8; 64],
}

impl Header {
    /// A header for `version` over an image of `length` bytes with digest
    /// `sha256`, signature to be filled in (the tests sign with the
    /// Python tool's vector; the device never signs).
    pub fn new(version: &str, length: u32, sha256: [u8; 32], sig: [u8; 64]) -> Option<Header> {
        let v = version.as_bytes();
        if v.is_empty() || v.len() > VERSION_LEN || v.contains(&0) {
            return None;
        }
        let mut field = [0u8; VERSION_LEN];
        field[..v.len()].copy_from_slice(v);
        Some(Header {
            version: field,
            length,
            sha256,
            sig,
        })
    }

    /// Parse the first [`HEADER_LEN`] bytes of `bytes`. Checks the magic
    /// and the version field's shape; trusts nothing else.
    pub fn parse(bytes: &[u8]) -> Result<Header, OtaError> {
        if bytes.len() < HEADER_LEN {
            return Err(OtaError::Truncated { have: bytes.len() });
        }
        if bytes[..8] != MAGIC {
            return Err(OtaError::BadMagic);
        }
        let mut version = [0u8; VERSION_LEN];
        version.copy_from_slice(&bytes[8..8 + VERSION_LEN]);
        let used = version.iter().position(|&b| b == 0).unwrap_or(VERSION_LEN);
        if used == 0
            || version[used..].iter().any(|&b| b != 0)
            || std::str::from_utf8(&version[..used]).is_err()
        {
            return Err(OtaError::BadVersion);
        }
        let length = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(&bytes[44..76]);
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&bytes[76..HEADER_LEN]);
        Ok(Header {
            version,
            length,
            sha256,
            sig,
        })
    }

    /// The version string (`"0.4.0"`).
    pub fn version(&self) -> &str {
        let used = self
            .version
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VERSION_LEN);
        std::str::from_utf8(&self.version[..used]).unwrap_or("")
    }

    /// The bytes the signature covers: magic, version, length, digest.
    pub fn signed_bytes(&self) -> [u8; SIGNED_LEN] {
        let mut out = [0u8; SIGNED_LEN];
        out[..8].copy_from_slice(&MAGIC);
        out[8..40].copy_from_slice(&self.version);
        out[40..44].copy_from_slice(&self.length.to_le_bytes());
        out[44..76].copy_from_slice(&self.sha256);
        out
    }

    /// The header as it is on the wire.
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..SIGNED_LEN].copy_from_slice(&self.signed_bytes());
        out[SIGNED_LEN..].copy_from_slice(&self.sig);
        out
    }
}

/// A header whose signature verified: what the bundle claims, now trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub version: String,
    pub length: u32,
    pub sha256: [u8; 32],
}

impl Meta {
    /// The image fits a partition of `partition` bytes.
    pub fn check_fits(&self, partition: u32) -> Result<(), OtaError> {
        if self.length > partition {
            return Err(OtaError::TooLarge {
                length: self.length,
                partition,
            });
        }
        Ok(())
    }

    /// The image that arrived (`actual` bytes, first byte `first`, digest
    /// `digest`) is the one the header describes.
    pub fn check_image(&self, actual: u64, first: u8, digest: &[u8; 32]) -> Result<(), OtaError> {
        if u64::from(self.length) != actual {
            return Err(OtaError::LengthMismatch {
                header: self.length,
                actual,
            });
        }
        if first != ESP_IMAGE_MAGIC {
            return Err(OtaError::NotAnImage);
        }
        if digest != &self.sha256 {
            return Err(OtaError::HashMismatch);
        }
        Ok(())
    }
}

/// Check `header`'s signature against `public_key` (the firmware's
/// `keys/opendmd.pub`). Strict verification: no malleable signatures, no
/// small-order keys.
pub fn verify_header(header: &Header, public_key: &[u8; PUBLIC_KEY_LEN]) -> Result<Meta, OtaError> {
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| OtaError::BadKey)?;
    let sig = Signature::from_bytes(&header.sig);
    key.verify_strict(&header.signed_bytes(), &sig)
        .map_err(|_| OtaError::BadSignature)?;
    Ok(Meta {
        version: header.version().to_string(),
        length: header.length,
        sha256: header.sha256,
    })
}

/// SHA-256 of an image fed in pieces, with a byte count and the first byte
/// kept for [`Meta::check_image`]. About 100 B of state.
#[derive(Clone, Default)]
pub struct ImageHasher {
    sha: Sha256,
    len: u64,
    first: Option<u8>,
}

impl ImageHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, data: &[u8]) {
        if self.first.is_none() {
            self.first = data.first().copied();
        }
        self.sha.update(data);
        self.len += data.len() as u64;
    }

    /// Bytes hashed so far.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The first byte hashed (0 if none).
    pub fn first(&self) -> u8 {
        self.first.unwrap_or(0)
    }

    pub fn finish(self) -> [u8; 32] {
        self.sha.finalize().into()
    }
}

/// SHA-256 of `data` in one go.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Made by `tools/fw-sign.py sign` with the seed 00 01 02 .. 1f, version
    /// `0.4.0-test`, over the 64-byte image E9 01 02 .. 3F.
    const KAT_PUB: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
    const KAT_HEADER: &str = "4f44465731000000302e342e302d74657374000000000000000000000000000000000000000000004000\
        0000ac4a16884561961261469a1a0d7dd650eb0beb0e7dd9ade9d85d0af56bcdb061df55c5618b5c325cd0e72681\
        09c0a77487878d20e1ff27c43424aa29215a9064d3e9b9a5797614360f5793eff7f2d2d0e37dd5f71de9241f79b6\
        f3c9f1bf410b";

    fn kat_image() -> Vec<u8> {
        let mut v = vec![0xE9u8];
        v.extend(1u8..64);
        v
    }

    fn kat_header() -> Vec<u8> {
        let h = unhex(KAT_HEADER);
        assert_eq!(h.len(), HEADER_LEN);
        h
    }

    fn kat_pub() -> [u8; 32] {
        unhex(KAT_PUB).try_into().unwrap()
    }

    fn hashed(image: &[u8]) -> ImageHasher {
        let mut h = ImageHasher::new();
        // In odd pieces, as the network delivers them.
        for piece in image.chunks(7) {
            h.update(piece);
        }
        h
    }

    #[test]
    fn known_answer_vector_verifies() {
        let header = Header::parse(&kat_header()).unwrap();
        assert_eq!(header.version(), "0.4.0-test");
        assert_eq!(header.length, 64);
        let meta = verify_header(&header, &kat_pub()).unwrap();
        assert_eq!(meta.version, "0.4.0-test");
        let image = kat_image();
        let h = hashed(&image);
        assert_eq!(h.len(), 64);
        let (len, first) = (h.len(), h.first());
        let digest = h.finish();
        assert_eq!(digest, sha256(&image));
        meta.check_image(len, first, &digest).unwrap();
        meta.check_fits(0x240000).unwrap();
    }

    #[test]
    fn round_trip() {
        let bytes = kat_header();
        let header = Header::parse(&bytes).unwrap();
        assert_eq!(header.to_bytes().to_vec(), bytes);
        let made = Header::new("0.4.0-test", header.length, header.sha256, header.sig).unwrap();
        assert_eq!(made, header);
        assert!(Header::new("", 0, [0; 32], [0; 64]).is_none());
        assert!(Header::new(&"x".repeat(33), 0, [0; 32], [0; 64]).is_none());
        assert!(Header::new(&"x".repeat(32), 0, [0; 32], [0; 64]).is_some());
    }

    #[test]
    fn truncated() {
        let bytes = kat_header();
        assert_eq!(
            Header::parse(&bytes[..139]),
            Err(OtaError::Truncated { have: 139 })
        );
    }

    #[test]
    fn tampered_magic() {
        let mut b = kat_header();
        b[3] ^= 1;
        assert_eq!(Header::parse(&b), Err(OtaError::BadMagic));
    }

    #[test]
    fn tampered_version() {
        // Still well-formed, so it parses; the signature catches it.
        let mut b = kat_header();
        b[8] = b'1';
        let h = Header::parse(&b).unwrap();
        assert_eq!(h.version(), "1.4.0-test");
        assert_eq!(verify_header(&h, &kat_pub()), Err(OtaError::BadSignature));
        // Malformed: empty, a NUL inside, not UTF-8.
        let mut b = kat_header();
        b[8] = 0;
        assert_eq!(Header::parse(&b), Err(OtaError::BadVersion));
        let mut b = kat_header();
        b[10] = 0;
        assert_eq!(Header::parse(&b), Err(OtaError::BadVersion));
        let mut b = kat_header();
        b[8] = 0xFF;
        assert_eq!(Header::parse(&b), Err(OtaError::BadVersion));
    }

    #[test]
    fn tampered_length() {
        let mut b = kat_header();
        b[40] = 65;
        let h = Header::parse(&b).unwrap();
        assert_eq!(verify_header(&h, &kat_pub()), Err(OtaError::BadSignature));
    }

    #[test]
    fn tampered_digest() {
        let mut b = kat_header();
        b[50] ^= 0x80;
        let h = Header::parse(&b).unwrap();
        assert_eq!(verify_header(&h, &kat_pub()), Err(OtaError::BadSignature));
    }

    #[test]
    fn tampered_signature() {
        for at in [76, 100, 139] {
            let mut b = kat_header();
            b[at] ^= 0x01;
            let h = Header::parse(&b).unwrap();
            assert_eq!(verify_header(&h, &kat_pub()), Err(OtaError::BadSignature));
        }
    }

    #[test]
    fn tampered_image() {
        let meta = verify_header(&Header::parse(&kat_header()).unwrap(), &kat_pub()).unwrap();
        let mut image = kat_image();
        image[33] ^= 4;
        let h = hashed(&image);
        let (len, first) = (h.len(), h.first());
        assert_eq!(
            meta.check_image(len, first, &h.finish()),
            Err(OtaError::HashMismatch)
        );
    }

    #[test]
    fn length_mismatch() {
        let meta = verify_header(&Header::parse(&kat_header()).unwrap(), &kat_pub()).unwrap();
        let image = kat_image();
        let h = hashed(&image[..63]);
        let (len, first) = (h.len(), h.first());
        assert_eq!(
            meta.check_image(len, first, &h.finish()),
            Err(OtaError::LengthMismatch {
                header: 64,
                actual: 63
            })
        );
        assert_eq!(
            meta.check_fits(63),
            Err(OtaError::TooLarge {
                length: 64,
                partition: 63
            })
        );
    }

    #[test]
    fn not_an_image() {
        let meta = verify_header(&Header::parse(&kat_header()).unwrap(), &kat_pub()).unwrap();
        let mut image = kat_image();
        image[0] = 0xE8;
        let h = hashed(&image);
        let (len, first) = (h.len(), h.first());
        assert_eq!(
            meta.check_image(len, first, &h.finish()),
            Err(OtaError::NotAnImage)
        );
    }

    #[test]
    fn wrong_key() {
        let h = Header::parse(&kat_header()).unwrap();
        let mut other = kat_pub();
        // Another valid key: the RFC 8032 test 1 public key.
        other.copy_from_slice(&unhex(
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        ));
        assert_eq!(verify_header(&h, &other), Err(OtaError::BadSignature));
        // Not a point at all.
        let mut bad = [0xFFu8; 32];
        bad[31] = 0x7F;
        let r = verify_header(&h, &bad);
        assert!(
            matches!(r, Err(OtaError::BadKey) | Err(OtaError::BadSignature)),
            "{r:?}"
        );
    }

    #[test]
    fn the_committed_public_key_is_a_key() {
        let key: &[u8] = include_bytes!("../../firmware/keys/opendmd.pub");
        let key: [u8; 32] = key.try_into().expect("firmware/keys/opendmd.pub is 32 B");
        assert!(VerifyingKey::from_bytes(&key).is_ok());
    }

    #[test]
    fn sha256_standard_vector() {
        assert_eq!(
            sha256(b"abc").to_vec(),
            unhex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }
}
