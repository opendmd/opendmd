//! The signed firmware bundle (`docs/ota.md`): its 140-byte header, the
//! Ed25519 check of that header against the public key compiled into the
//! firmware, and a streaming SHA-256 so the device hashes the image as it
//! writes it to the card. `tools/fw-sign.py` is the other implementation
//! (it signs); the known-answer test below holds a bundle it made.
//!
//! ```text
//! offset  size  field
//!      0     8  magic: the flash layout and its table version (below)
//!      8    32  version, UTF-8, NUL-padded (at least one byte, no NUL inside)
//!     40     4  length of the image, u32 LE
//!     44    32  SHA-256 of the image
//!     76    64  Ed25519 signature over bytes [0, 76)
//!    140        the app image (ESP image magic 0xE9 first)
//! ```
//!
//! **The magic names the flash layout** the image is built for: its
//! `variant` (`4mb`, the one this tree builds) and the version of that
//! variant's partition table ([`LAYOUTS`]). `"ODFW1"` + three NULs is the
//! DevKit's table v2 (every 4mb bundle up to 1.0.0-beta.15), `"ODFW34mb"`
//! the DevKit's table v3 (a bigger `factory`, `firmware/partitions.csv`).
//! The signature covers it. A player installs a bundle only for its own
//! variant *and* table ([`Header::check_layout`]): an update over WiFi never
//! changes the table, so a bundle built for another one needs a USB install.
//! Any other magic that starts `"ODFW"` (`"ODFW18mb"`, the 8 MB layout
//! releases up to 1.0.0-beta.16 also carried) is a bundle for a layout this
//! tree does not build: [`OtaError::OtherLayout`], refused in words. A firmware or
//! updater from before a layout knows nothing of its magic and refuses it
//! as not a bundle at all (1.0.0-beta.15 and earlier: "bad magic" for
//! `ODFW34mb`), which is what keeps a v3 image out of a v2 board's smaller
//! `factory`.
//!
//! **The key id** (1.0.0-beta.15 on): the image's last 16 bytes are a key
//! trailer, `"ODFWKEY1"`, the id of the key that signed it, 7 zero bytes
//! ([`KeyTail`], [`key_id_of`]). They are part of the image as the header
//! counts it (its length, its SHA-256), so a firmware from before knows
//! nothing of them and still verifies the bundle, and the updater writes
//! them after the app image, where `esp_image_verify` (which sizes the image
//! from its own segment headers) never reads. A firmware that knows the
//! trailer checks the id against the key that verified the header; a bundle
//! without one (earlier releases) counts as key 0, the first release key.
//!
//! **The web app pack's signature** is [`www`].
//!
//! Verify-only: no key generation, no `rand`. The order on the device is
//! [`Header::parse`] on the first 140 B of the body, [`Header::check_variant`]
//! against its own layout, [`verify_header`] (the signature), then the image
//! streamed through an [`ImageHasher`] and checked with
//! [`Meta::check_image`] and [`check_app_desc`] at the end.

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fmt;

/// `ODFW1` and three NULs: a 4mb bundle for the DevKit's table v2 (every
/// 4mb bundle up to 1.0.0-beta.15). Refused by a v3 player.
pub const MAGIC: [u8; 8] = *b"ODFW1\0\0\0";
/// What every bundle magic starts with: one that does, but names no
/// table in [`LAYOUTS`], is a bundle for another flash layout.
pub const MAGIC_PREFIX: &[u8; 4] = b"ODFW";
/// A 4mb bundle for the DevKit's table v3: `ODFW`, the table version, the
/// variant. A later table gets the next digit (`ODFW44mb`, `ODFW38mb`).
pub const MAGIC_4MB_V3: [u8; 8] = *b"ODFW34mb";
/// The flash layouts, as the firmware names them (`/api/status`
/// `variant`, the app descriptor's `"<version> <variant>"`).
pub const VARIANTS: [&str; 1] = ["4mb"];

/// One partition table a bundle can be built for: the variant, the
/// table's version (`firmware/partitions*.csv`'s header line), the bundle
/// magic that names it, and its `factory` partition's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub variant: &'static str,
    pub table: u8,
    pub magic: [u8; 8],
    pub factory: u32,
}

/// Every table a bundle may name, the current one of each variant first
/// (what this tree builds: [`layout_of`]); the older ones after, so their
/// bundles are recognised and refused in words.
pub const LAYOUTS: [Layout; 2] = [
    Layout {
        variant: "4mb",
        table: 3,
        magic: MAGIC_4MB_V3,
        factory: 0x28_0000,
    },
    Layout {
        variant: "4mb",
        table: 2,
        magic: MAGIC,
        factory: 0x24_0000,
    },
];

/// The table this tree builds for `variant` (`/api/status` `layout`).
pub fn layout_of(variant: &str) -> Option<&'static Layout> {
    LAYOUTS.iter().find(|l| l.variant == variant)
}

/// The table `variant` at version `table`, if there is one.
pub fn layout(variant: &str, table: u8) -> Option<&'static Layout> {
    LAYOUTS
        .iter()
        .find(|l| l.variant == variant && l.table == table)
}

/// The magic of a bundle for `variant`'s current table.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn magic_for(variant: &str) -> Option<[u8; 8]> {
    layout_of(variant).map(|l| l.magic)
}

/// The table a magic names, if it is one.
pub fn layout_of_magic(magic: &[u8]) -> Option<&'static Layout> {
    LAYOUTS.iter().find(|l| l.magic[..] == *magic)
}

/// The variant a magic names, if it is one (any table).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn variant_of_magic(magic: &[u8]) -> Option<&'static str> {
    layout_of_magic(magic).map(|l| l.variant)
}

/// What a person calls the board a variant is for (the web app, the
/// installer and the refusals use the same words).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn variant_name(variant: &str) -> &str {
    match variant {
        "4mb" => "DevKit (4 MB)",
        other => other,
    }
}

/// Where the app descriptor (`esp_app_desc_t`) starts in an app image: after
/// the 24-byte image header and the first segment's 8-byte header.
pub const APP_DESC_OFFSET: usize = 0x20;
/// `esp_app_desc_t.magic_word`.
pub const APP_DESC_MAGIC: u32 = 0xABCD_5432;
/// `esp_app_desc_t.version`: 32 bytes at 0x30 of the image.
pub const APP_DESC_VERSION_OFFSET: usize = 0x30;
/// The image bytes [`check_app_desc`] reads (and [`ImageHasher`] keeps).
pub const APP_PREFIX_LEN: usize = APP_DESC_VERSION_OFFSET + 32;
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
    Truncated {
        have: usize,
    },
    BadMagic,
    /// An OpenDMD bundle (its magic starts `ODFW`) for a flash layout this
    /// firmware does not build (e.g. `ODFW18mb`).
    OtherLayout,
    /// Empty, not UTF-8, or a NUL before the padding.
    BadVersion,
    /// The compiled-in public key is not a valid point.
    BadKey,
    /// The signature does not verify: wrong key, or the header was changed.
    BadSignature,
    /// The image's length is not the header's.
    LengthMismatch {
        header: u32,
        actual: u64,
    },
    /// The image is larger than the partition it goes into.
    TooLarge {
        length: u32,
        partition: u32,
    },
    /// The image does not start with 0xE9.
    NotAnImage,
    /// The image's SHA-256 is not the header's.
    HashMismatch,
    /// The bundle is for another flash layout than this player's.
    WrongVariant {
        bundle: &'static str,
        device: String,
    },
    /// The bundle is for this player's variant but another partition
    /// table (an update over WiFi never changes the table).
    WrongLayout {
        variant: &'static str,
        bundle: u8,
        device: u8,
    },
    /// The image's app descriptor is not an OpenDMD build for the
    /// bundle's layout (`"<version> <variant>"`).
    WrongBuild {
        variant: String,
        found: String,
    },
    /// The image's key trailer names another key than the one that
    /// verified the header.
    WrongKeyId {
        named: u8,
        signed_by: u8,
    },
    /// Older than the rollback floor this firmware was built with.
    BelowFloor {
        version: String,
        floor: String,
    },
    /// A web app pack without a signature (or with a damaged trailer).
    Unsigned,
    /// Signed with a key this firmware does not know.
    UnknownKey(u8),
}

impl fmt::Display for OtaError {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OtaError::Truncated { have } => {
                write!(
                    f,
                    "truncated: {have} B, the bundle header is {HEADER_LEN} B"
                )
            }
            OtaError::BadMagic => f.write_str("not an OpenDMD firmware bundle (bad magic)"),
            OtaError::OtherLayout => f.write_str(
                "this update is for another flash layout than this player's: use the DevKit (4 MB) \
                 update file (opendmd.dev/install has the latest)",
            ),
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
            OtaError::WrongVariant { bundle, device } => write!(
                f,
                "this update is for the {} and this player runs the {} firmware: use the {} update file",
                variant_name(bundle),
                variant_name(device),
                variant_name(device)
            ),
            OtaError::WrongLayout {
                variant,
                bundle,
                device,
            } if bundle > device => write!(
                f,
                "this update needs flash layout v{bundle} and this {} has layout v{device}: \
                 reinstall it once over USB from opendmd.dev/install (an update over WiFi \
                 cannot change the layout)",
                variant_name(variant)
            ),
            OtaError::WrongLayout {
                variant,
                bundle,
                device,
            } => write!(
                f,
                "this update is for the older flash layout v{bundle} and this {} has layout \
                 v{device}: use a release made for layout v{device} (opendmd.dev/install has \
                 the latest)",
                variant_name(variant)
            ),
            OtaError::WrongBuild { variant, found } => write!(
                f,
                "the image inside is not an OpenDMD {variant} build (its app descriptor says '{found}')"
            ),
            OtaError::WrongKeyId { named, signed_by } => write!(
                f,
                "the image names signing key {named}, but key {signed_by} signed it"
            ),
            OtaError::BelowFloor { version, floor } => write!(
                f,
                "firmware {version} is older than {floor}, the oldest this player installs"
            ),
            OtaError::Unsigned => f.write_str(
                "not signed: this player takes only a release's web app pack (signed)",
            ),
            OtaError::UnknownKey(id) => {
                write!(f, "signed with key {id}, which this player does not know")
            }
        }
    }
}

impl std::error::Error for OtaError {}

/// The header, parsed but not yet trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    variant: &'static str,
    table: u8,
    version: [u8; VERSION_LEN],
    pub length: u32,
    pub sha256: [u8; 32],
    pub sig: [u8; 64],
}

impl Header {
    /// A header for a `variant` image of `version` (for the variant's
    /// current table), `length` bytes with digest `sha256`, signature to be
    /// filled in (the tests sign with the Python tool's vector; the device
    /// never signs).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn new(
        variant: &str,
        version: &str,
        length: u32,
        sha256: [u8; 32],
        sig: [u8; 64],
    ) -> Option<Header> {
        Header::for_table(layout_of(variant)?, version, length, sha256, sig)
    }

    /// [`Header::new`] for a given table.
    pub fn for_table(
        layout: &'static Layout,
        version: &str,
        length: u32,
        sha256: [u8; 32],
        sig: [u8; 64],
    ) -> Option<Header> {
        let v = version.as_bytes();
        if v.is_empty() || v.len() > VERSION_LEN || v.contains(&0) {
            return None;
        }
        let mut field = [0u8; VERSION_LEN];
        field[..v.len()].copy_from_slice(v);
        Some(Header {
            variant: layout.variant,
            table: layout.table,
            version: field,
            length,
            sha256,
            sig,
        })
    }

    /// Parse the first [`HEADER_LEN`] bytes of `bytes`. Checks the magic
    /// (one of the known variants) and the version field's shape; trusts
    /// nothing else.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn parse(bytes: &[u8]) -> Result<Header, OtaError> {
        if bytes.len() < HEADER_LEN {
            return Err(OtaError::Truncated { have: bytes.len() });
        }
        let layout = layout_of_magic(&bytes[..8]).ok_or(if bytes[..4] == MAGIC_PREFIX[..] {
            OtaError::OtherLayout
        } else {
            OtaError::BadMagic
        })?;
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
            variant: layout.variant,
            table: layout.table,
            version,
            length,
            sha256,
            sig,
        })
    }

    /// The flash layout's variant the bundle is for (`"4mb"`).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn variant(&self) -> &'static str {
        self.variant
    }

    /// The version of the variant's partition table the bundle is for.
    pub fn table(&self) -> u8 {
        self.table
    }

    /// The bundle is for `device`'s variant and its table `table` (the
    /// player's `FLASH_VARIANT` and the table it runs). Needs no trust: a
    /// refusal, before the signature is checked. The variant first, then
    /// the table.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn check_layout(&self, device: &str, table: u8) -> Result<(), OtaError> {
        if self.variant != device {
            return Err(OtaError::WrongVariant {
                bundle: self.variant,
                device: device.to_string(),
            });
        }
        if self.table != table {
            return Err(OtaError::WrongLayout {
                variant: self.variant,
                bundle: self.table,
                device: table,
            });
        }
        Ok(())
    }

    /// [`Header::check_layout`] against `device`'s current table (what this
    /// tree builds for it): the check a player of this tree makes.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn check_variant(&self, device: &str) -> Result<(), OtaError> {
        self.check_layout(device, layout_of(device).map_or(0, |l| l.table))
    }

    /// The version string (`"0.4.0"`).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn version(&self) -> &str {
        let used = self
            .version
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VERSION_LEN);
        std::str::from_utf8(&self.version[..used]).unwrap_or("")
    }

    /// The bytes the signature covers: magic, version, length, digest.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn signed_bytes(&self) -> [u8; SIGNED_LEN] {
        let mut out = [0u8; SIGNED_LEN];
        out[..8].copy_from_slice(&layout(self.variant, self.table).map_or(MAGIC, |l| l.magic));
        out[8..40].copy_from_slice(&self.version);
        out[40..44].copy_from_slice(&self.length.to_le_bytes());
        out[44..76].copy_from_slice(&self.sha256);
        out
    }

    /// The header as it is on the wire.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
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
    pub variant: &'static str,
    pub version: String,
    pub length: u32,
    pub sha256: [u8; 32],
}

impl Meta {
    /// The image fits a partition of `partition` bytes.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
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
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
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

/// The image's first [`APP_PREFIX_LEN`] bytes hold the app descriptor of an
/// OpenDMD build for `variant`: its magic word, and a version that ends in
/// `" <variant>"` (what `make fw-release` writes, and checks). The bundle's
/// magic names the layout; this checks the image inside agrees, so a 4mb
/// image signed for another layout by mistake is refused too. Call it
/// after [`Meta::check_image`], when the bytes are known to be the signed ones.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn check_app_desc(prefix: &[u8], variant: &str) -> Result<(), OtaError> {
    let found = app_desc_version(prefix);
    let ok = found
        .as_deref()
        .and_then(|d| d.strip_suffix(variant))
        .is_some_and(|v| v.ends_with(' ') && v.len() > 1);
    if ok {
        Ok(())
    } else {
        Err(OtaError::WrongBuild {
            variant: variant.to_string(),
            found: found.unwrap_or_else(|| "nothing: no app descriptor".into()),
        })
    }
}

/// The app descriptor's version string, from an image's first
/// [`APP_PREFIX_LEN`] bytes; `None` without the descriptor's magic word.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn app_desc_version(prefix: &[u8]) -> Option<String> {
    let at = APP_DESC_OFFSET;
    let magic = prefix.get(at..at + 4)?;
    if u32::from_le_bytes([magic[0], magic[1], magic[2], magic[3]]) != APP_DESC_MAGIC {
        return None;
    }
    let raw = prefix.get(APP_DESC_VERSION_OFFSET..APP_PREFIX_LEN)?;
    let used = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Some(String::from_utf8_lossy(&raw[..used]).into_owned())
}

/// Check `header`'s signature against `public_key` (the firmware's
/// `keys/opendmd.pub`). Strict verification: no malleable signatures, no
/// small-order keys.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn verify_header(header: &Header, public_key: &[u8; PUBLIC_KEY_LEN]) -> Result<Meta, OtaError> {
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| OtaError::BadKey)?;
    let sig = Signature::from_bytes(&header.sig);
    key.verify_strict(&header.signed_bytes(), &sig)
        .map_err(|_| OtaError::BadSignature)?;
    Ok(Meta {
        variant: header.variant,
        version: header.version().to_string(),
        length: header.length,
        sha256: header.sha256,
    })
}

/// SHA-256 of an image fed in pieces, with a byte count, the first byte
/// kept for [`Meta::check_image`] and the first [`APP_PREFIX_LEN`] bytes for
/// [`check_app_desc`]. About 170 B of state.
#[derive(Clone)]
pub struct ImageHasher {
    sha: Sha256,
    len: u64,
    first: Option<u8>,
    prefix: [u8; APP_PREFIX_LEN],
}

impl Default for ImageHasher {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn default() -> Self {
        ImageHasher {
            sha: Sha256::default(),
            len: 0,
            first: None,
            prefix: [0; APP_PREFIX_LEN],
        }
    }
}

impl ImageHasher {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn update(&mut self, data: &[u8]) {
        if self.first.is_none() {
            self.first = data.first().copied();
        }
        if self.len < APP_PREFIX_LEN as u64 {
            let at = self.len as usize;
            let n = (APP_PREFIX_LEN - at).min(data.len());
            self.prefix[at..at + n].copy_from_slice(&data[..n]);
        }
        self.sha.update(data);
        self.len += data.len() as u64;
    }

    /// Bytes hashed so far.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn len(&self) -> u64 {
        self.len
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The first byte hashed (0 if none).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn first(&self) -> u8 {
        self.first.unwrap_or(0)
    }

    /// The image's first [`APP_PREFIX_LEN`] bytes (zeros past the end of a
    /// shorter one): for [`check_app_desc`].
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn prefix(&self) -> &[u8; APP_PREFIX_LEN] {
        &self.prefix
    }

    #[inline(never)]
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn finish(self) -> [u8; 32] {
        self.sha.finalize().into()
    }
}

/// SHA-256 of `data` in one go.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// The firmware version from the running image's app descriptor
/// (`esp_app_desc_t.version`), which the build sets to `"<version>
/// <variant>"` (`make fw-build` / `fw-release`, `FW_VERSION`, by default
/// `firmware/Cargo.toml`'s): the version a bundle for this image carries in
/// its header, and the updater names `done-<version>.bin` after. `None`
/// for a descriptor in any other form (IDF's default, `git describe`), so
/// the caller falls back to the crate's version.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn version_from_app_desc<'a>(desc: &'a str, variant: &str) -> Option<&'a str> {
    let v = desc.strip_suffix(variant)?.strip_suffix(' ')?;
    (!v.is_empty() && v.len() <= VERSION_LEN && !v.contains(char::is_whitespace)).then_some(v)
}

/// The key trailer's length, at the end of a bundle's image.
pub const KEY_TRAILER_LEN: usize = 16;
/// Its first 8 bytes.
pub const KEY_TRAILER_MAGIC: [u8; 8] = *b"ODFWKEY1";

/// The id of the key a bundle's image says signed it, from the image's last
/// [`KEY_TRAILER_LEN`] bytes; `None` without a trailer (a bundle from before
/// 1.0.0-beta.15, signed by key 0).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn key_id_of(tail: &[u8]) -> Option<u8> {
    let t = tail.get(tail.len().checked_sub(KEY_TRAILER_LEN)?..)?;
    (t[..8] == KEY_TRAILER_MAGIC && t[9..].iter().all(|&b| b == 0)).then_some(t[8])
}

/// The last [`KEY_TRAILER_LEN`] bytes of a stream fed in pieces (the image
/// as it arrives), for [`key_id_of`].
#[derive(Clone, Copy, Default)]
pub struct KeyTail {
    buf: [u8; KEY_TRAILER_LEN],
    len: usize,
}

impl KeyTail {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn update(&mut self, data: &[u8]) {
        // The last `n` bytes of `data` go in at the end, the rest move up.
        let n = data.len().min(KEY_TRAILER_LEN);
        self.buf.copy_within(n.., 0);
        self.buf[KEY_TRAILER_LEN - n..].copy_from_slice(&data[data.len() - n..]);
        self.len = (self.len + n).min(KEY_TRAILER_LEN);
    }

    /// The key id the trailer names, `None` without one.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn key_id(&self) -> Option<u8> {
        let t = &self.buf;
        (self.len == KEY_TRAILER_LEN && t[..8] == KEY_TRAILER_MAGIC && t[9..] == [0; 7]).then_some(t[8])
    }
}

/// The key that verified the header is the one the image names: `signed_by`
/// (the id of the key that verified) against the trailer's id, a bundle
/// with no trailer counting as key 0.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn check_key_id(signed_by: u8, trailer: Option<u8>) -> Result<(), OtaError> {
    let named = trailer.unwrap_or(0);
    if named != signed_by {
        return Err(OtaError::WrongKeyId {
            named,
            signed_by,
        });
    }
    Ok(())
}

/// Check `header` against each `(id, key)` this firmware trusts, in turn:
/// the id of the first that verifies, with the trusted [`Meta`].
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn verify_header_any(header: &Header, keys: &[(u8, &[u8; PUBLIC_KEY_LEN])]) -> Result<(Meta, u8), OtaError> {
    let mut last = OtaError::BadSignature;
    for &(id, key) in keys {
        match verify_header(header, key) {
            Ok(m) => return Ok((m, id)),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Whether version `a` is older than `b`: dot-separated numbers compared as
/// numbers (a leading `v` ignored, a missing part 0), and a pre-release
/// (`-beta.3`) before its release, its dot-separated parts compared as
/// numbers where both are (`beta.9` < `beta.10`). The web app's `older` in
/// `firmware.js` is the same rule. Build metadata (`+…`) is ignored.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn version_older(a: &str, b: &str) -> bool {
    use std::cmp::Ordering;
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn split(s: &str) -> (&str, Option<&str>) {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split('+').next().unwrap_or(s);
        match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        }
    }
    let ((ac, ap), (bc, bp)) = (split(a), split(b));
    let (an, bn): (Vec<u64>, Vec<u64>) = (
        ac.split('.').map(|x| x.parse().unwrap_or(0)).collect(),
        bc.split('.').map(|x| x.parse().unwrap_or(0)).collect(),
    );
    for i in 0..an.len().max(bn.len()) {
        let (x, y) = (an.get(i).copied().unwrap_or(0), bn.get(i).copied().unwrap_or(0));
        if x != y {
            return x < y;
        }
    }
    match (ap, bp) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(p), Some(q)) => {
            let (mut pi, mut qi) = (p.split('.'), q.split('.'));
            loop {
                match (pi.next(), qi.next()) {
                    (None, None) => return false,
                    (None, Some(_)) => return true,
                    (Some(_), None) => return false,
                    (Some(x), Some(y)) => {
                        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                            (Ok(m), Ok(n)) => m.cmp(&n),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            _ => x.cmp(y),
                        };
                        if o != Ordering::Equal {
                            return o == Ordering::Less;
                        }
                    }
                }
            }
        }
    }
}

/// The rollback floor, if the firmware was built with one
/// (`OPENDMD_OTA_FLOOR`, off by default): a bundle older than `floor` is
/// refused.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn check_floor(version: &str, floor: Option<&str>) -> Result<(), OtaError> {
    match floor {
        Some(f) if !f.is_empty() && version_older(version, f) => Err(OtaError::BelowFloor {
            version: version.to_string(),
            floor: f.to_string(),
        }),
        _ => Ok(()),
    }
}

/// The web app pack's signature (1.0.0-beta.15 on): `www_pack`'s header
/// flag [`crate::www_pack::FLAG_SIGNED`] and a 72-byte trailer at the end of
/// the pack, inside its total length and its CRC:
///
/// ```text
/// offset  size  field
///      0     4  "ODWS"
///      4     1  key id (0: the first release key, as for firmware bundles)
///      5     3  zero
///      8    64  Ed25519 signature over "ODWPSIG1" | key id | SHA-256 of the
///               pack's bytes before the trailer, the header's CRC field
///               (bytes 12..16) taken as zeros
/// ```
///
/// The CRC is computed last, over the trailer too, so the CRC still covers
/// everything after the header and the signature covers everything but the
/// CRC. `tools/fw-sign.py sign-www` signs; the firmware verifies with the
/// same keys as bundles before a `PUT /api/www` touches flash.
pub mod www {
    use super::{ImageHasher, OtaError, PUBLIC_KEY_LEN};
    use crate::www_pack::SIG_TRAILER_LEN;
    use ed25519_dalek::{Signature, VerifyingKey};

    /// The trailer's magic.
    pub const TRAILER_MAGIC: [u8; 4] = *b"ODWS";
    /// The signed message's first 8 bytes (nothing else is signed with them).
    pub const DOMAIN: [u8; 8] = *b"ODWPSIG1";
    /// The signed message: the domain, the key id, the digest.
    pub const MESSAGE_LEN: usize = 8 + 1 + 32;

    /// What a pack's signature check needs, once the whole pack has gone
    /// through a [`PackHasher`].
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Signed {
        pub key_id: u8,
        pub message: [u8; MESSAGE_LEN],
        pub sig: [u8; 64],
    }

    /// A signed pack fed in order from its first byte, in pieces of any
    /// size: the SHA-256 of what the signature covers, and the trailer.
    /// ~250 B of state.
    #[derive(Clone)]
    pub struct PackHasher {
        // (An image's hasher, for its SHA-256: one copy of the hash's
        // finishing code in the firmware.)
        sha: ImageHasher,
        total: usize,
        at: usize,
        trailer: [u8; SIG_TRAILER_LEN],
    }

    impl PackHasher {
        /// For a pack of `total` bytes (its header's total length).
        #[cfg_attr(target_arch = "xtensa", optimize(size))]
        pub fn new(total: usize) -> PackHasher {
            PackHasher {
                sha: ImageHasher::new(),
                total,
                at: 0,
                trailer: [0; SIG_TRAILER_LEN],
            }
        }

        #[cfg_attr(target_arch = "xtensa", optimize(size))]
        pub fn update(&mut self, mut data: &[u8]) {
            let body = self.total.saturating_sub(SIG_TRAILER_LEN);
            while !data.is_empty() && self.at < self.total {
                let at = self.at;
                let n = if at < 12 {
                    (12 - at).min(data.len())
                } else if at < 16 {
                    (16 - at).min(data.len())
                } else if at < body {
                    (body - at).min(data.len())
                } else {
                    (self.total - at).min(data.len())
                };
                let (piece, rest) = data.split_at(n);
                if (12..16).contains(&at) {
                    self.sha.update(&[0u8; 4][..n]);
                } else if at < body {
                    self.sha.update(piece);
                } else {
                    let t = at - body;
                    self.trailer[t..t + n].copy_from_slice(piece);
                }
                self.at += n;
                data = rest;
            }
        }

        /// The message and signature, once all `total` bytes are in.
        #[cfg_attr(target_arch = "xtensa", optimize(size))]
        pub fn finish(self) -> Result<Signed, OtaError> {
            if self.at != self.total || self.total < SIG_TRAILER_LEN {
                return Err(OtaError::Unsigned);
            }
            let t = &self.trailer;
            if t[..4] != TRAILER_MAGIC || t[5..8] != [0, 0, 0] {
                return Err(OtaError::Unsigned);
            }
            let key_id = t[4];
            let mut message = [0u8; MESSAGE_LEN];
            message[..8].copy_from_slice(&DOMAIN);
            message[8] = key_id;
            message[9..].copy_from_slice(&self.sha.finish());
            let mut sig = [0u8; 64];
            sig.copy_from_slice(&t[8..]);
            Ok(Signed {
                key_id,
                message,
                sig,
            })
        }
    }

    /// Check a pack's signature against the key its trailer names, among
    /// the `(id, key)` this firmware trusts. Strict verification.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn verify(signed: &Signed, keys: &[(u8, &[u8; PUBLIC_KEY_LEN])]) -> Result<(), OtaError> {
        let Some(&(_, key)) = keys.iter().find(|(id, _)| *id == signed.key_id) else {
            return Err(OtaError::UnknownKey(signed.key_id));
        };
        let key = VerifyingKey::from_bytes(key).map_err(|_| OtaError::BadKey)?;
        key.verify_strict(&signed.message, &Signature::from_bytes(&signed.sig))
            .map_err(|_| OtaError::BadSignature)
    }

    /// The whole check on a pack in memory (the tests, `opendmd-pack`).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn verify_bytes(pack: &[u8], keys: &[(u8, &[u8; PUBLIC_KEY_LEN])]) -> Result<u8, OtaError> {
        let header = crate::www_pack::Header::parse(pack).map_err(|_| OtaError::BadMagic)?;
        if !header.signed() || header.total_len as usize != pack.len() {
            return Err(OtaError::Unsigned);
        }
        let mut h = PackHasher::new(pack.len());
        h.update(pack);
        let s = h.finish()?;
        verify(&s, keys)?;
        Ok(s.key_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_the_app_descriptor() {
        assert_eq!(version_from_app_desc("0.3.1 4mb", "4mb"), Some("0.3.1"));
        assert_eq!(
            version_from_app_desc("0.4.0-rc.1 4mb", "4mb"),
            Some("0.4.0-rc.1")
        );
        assert_eq!(version_from_app_desc("0.3.1 8mb", "4mb"), None);
        assert_eq!(version_from_app_desc("0.3.1", "4mb"), None);
        assert_eq!(version_from_app_desc(" 4mb", "4mb"), None);
        assert_eq!(version_from_app_desc("4mb", "4mb"), None);
        assert_eq!(version_from_app_desc("v0.3-12-gabc", "4mb"), None);
        assert_eq!(version_from_app_desc("a b 4mb", "4mb"), None);
        let long = format!("{} 4mb", "9".repeat(VERSION_LEN + 1));
        assert_eq!(version_from_app_desc(&long, "4mb"), None);
    }

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

    /// The same seed, version and image, signed for the DevKit's table v3
    /// (`build_header(..., "4mb", 3)`): magic `ODFW34mb`.
    const KAT_HEADER_V3: &str = "4f44465733346d62302e342e302d746573740000000000000000000000000000000000000000\
        000040000000ac4a16884561961261469a1a0d7dd650eb0beb0e7dd9ade9d85d0af56bcdb061aada858cfabd9932\
        376f2b24d7382d4f08799c67f4e31eb4c756d689720e0521f298ed3e75cec3c69ed431039b4cff6ff9fe463197e4\
        74f35b7abc746731e208";

    /// The same seed, version and image, signed as an 8mb bundle by
    /// `tools/fw-sign.py` up to 1.0.0-beta.16 (magic `ODFW18mb`): a layout
    /// this tree no longer builds, refused.
    const KAT_HEADER_8MB: &str = "4f44465731386d62302e342e302d746573740000000000000000000000000000000000000000\
        000040000000ac4a16884561961261469a1a0d7dd650eb0beb0e7dd9ade9d85d0af56bcdb0611ffb5c5e448858c0\
        ed1a8bacc86df740f6c49763a7c4e95532ff98157ce3cfdb97f063bb78c4858e80c0b2d264b4cfdc1d6c12965a53\
        c45bacb56306d43c5809";

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
        let made = Header::for_table(
            layout("4mb", 2).unwrap(),
            "0.4.0-test",
            header.length,
            header.sha256,
            header.sig,
        )
        .unwrap();
        assert_eq!(made, header);
        // `new` is for the variant's current table: v3 for 4mb.
        let bytes3 = unhex(KAT_HEADER_V3);
        let h3 = Header::parse(&bytes3).unwrap();
        assert_eq!(h3.to_bytes().to_vec(), bytes3);
        assert_eq!(
            Header::new("4mb", "0.4.0-test", h3.length, h3.sha256, h3.sig).unwrap(),
            h3
        );
        assert!(Header::new("4mb", "", 0, [0; 32], [0; 64]).is_none());
        assert!(Header::new("4mb", &"x".repeat(33), 0, [0; 32], [0; 64]).is_none());
        assert!(Header::new("4mb", &"x".repeat(32), 0, [0; 32], [0; 64]).is_some());
        assert!(Header::new("16mb", "1.0.0", 0, [0; 32], [0; 64]).is_none());
        assert!(Header::new("8mb", "1.0.0", 0, [0; 32], [0; 64]).is_none());
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
    fn variants_and_their_magics() {
        assert_eq!(magic_for("4mb"), Some(*b"ODFW34mb"));
        assert_eq!(magic_for("8mb"), None);
        assert_eq!(magic_for("16mb"), None);
        for v in VARIANTS {
            assert_eq!(variant_of_magic(&magic_for(v).unwrap()), Some(v));
        }
        assert_eq!(variant_of_magic(&MAGIC), Some("4mb"));
        assert_eq!(layout_of_magic(&MAGIC).map(|l| l.table), Some(2));
        assert_eq!(layout_of("4mb").map(|l| l.table), Some(3));
        assert_eq!(layout_of("8mb"), None);
        assert_eq!(variant_of_magic(b"ODFW18mb"), None);
        assert_eq!(variant_of_magic(b"ODFW116m"), None);
        // Every magic is a different table; and the v3 one does not even
        // start like the ones before it ("ODFW1": what 1.0.0-beta.15's web
        // app, firmware and updater compare whole, and tools/fw-ota.sh
        // compared the first 5 bytes of), so nothing from before takes it.
        for (i, a) in LAYOUTS.iter().enumerate() {
            for b in &LAYOUTS[i + 1..] {
                assert_ne!(a.magic, b.magic);
                assert_ne!((a.variant, a.table), (b.variant, b.table));
            }
        }
        assert_ne!(&MAGIC_4MB_V3[..5], b"ODFW1");
        assert_eq!(variant_name("4mb"), "DevKit (4 MB)");
        assert_eq!(variant_name("8mb"), "8mb");
    }

    #[test]
    fn the_layout_is_the_variant_and_its_table() {
        // A v3 bundle on a v3 player: taken. The same bundle on a v2 player
        // (a firmware that knew both, checking against its own table): a
        // USB reinstall, in words.
        let h3 = Header::parse(&unhex(KAT_HEADER_V3)).unwrap();
        assert_eq!((h3.variant(), h3.table()), ("4mb", 3));
        h3.check_variant("4mb").unwrap();
        h3.check_layout("4mb", 3).unwrap();
        let e = h3.check_layout("4mb", 2).unwrap_err();
        assert_eq!(
            e,
            OtaError::WrongLayout {
                variant: "4mb",
                bundle: 3,
                device: 2
            }
        );
        assert_eq!(
            e.to_string(),
            "this update needs flash layout v3 and this DevKit (4 MB) has layout v2: reinstall it \
             once over USB from opendmd.dev/install (an update over WiFi cannot change the layout)"
        );
        // A v2 bundle (every DevKit release up to 1.0.0-beta.15) on a v3
        // player: refused before the signature, in words.
        let h2 = Header::parse(&kat_header()).unwrap();
        assert_eq!((h2.variant(), h2.table()), ("4mb", 2));
        let e = h2.check_variant("4mb").unwrap_err();
        assert_eq!(
            e,
            OtaError::WrongLayout {
                variant: "4mb",
                bundle: 2,
                device: 3
            }
        );
        assert_eq!(
            e.to_string(),
            "this update is for the older flash layout v2 and this DevKit (4 MB) has layout v3: \
             use a release made for layout v3 (opendmd.dev/install has the latest)"
        );
        // The variant is checked first.
        assert!(matches!(
            h3.check_layout("16mb", 3),
            Err(OtaError::WrongVariant { bundle: "4mb", .. })
        ));
        // It verifies as any bundle does (the layout check is the refusal).
        let meta = verify_header(&h3, &kat_pub()).unwrap();
        meta.check_fits(layout_of("4mb").unwrap().factory).unwrap();
    }

    /// `firmware/partitions.csv` is the table
    /// [`LAYOUTS`] says this tree builds: the version in the header comment,
    /// `factory`'s size, NVS where every table has it (a USB install that
    /// changes the table keeps the settings), apps on 64 KB boundaries,
    /// nothing overlapping, the last partition inside the flash.
    #[test]
    fn the_partition_tables_are_the_layouts() {
        let tables = [
            ("4mb", include_str!("../../firmware/partitions.csv"), 4u32 << 20),
        ];
        for (variant, csv, flash) in tables {
            let l = layout_of(variant).unwrap();
            let first = csv.lines().next().unwrap();
            assert!(
                first.starts_with(&format!("# OpenDMD partition table v{},", l.table)),
                "{variant}: {first}"
            );
            let num = |s: &str| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap();
            let rows: Vec<Vec<&str>> = csv
                .lines()
                .filter(|r| !r.trim_start().starts_with('#') && !r.trim().is_empty())
                .map(|r| r.split(',').map(str::trim).collect())
                .collect();
            let mut end = 0x9000;
            for r in &rows {
                let (name, kind, off, size) = (r[0], r[1], num(r[3]), num(r[4]));
                assert!(off >= end, "{variant}: {name} at {off:#x} overlaps (the previous ends at {end:#x})");
                if kind == "app" {
                    assert_eq!(off % 0x1_0000, 0, "{variant}: {name} at {off:#x}: apps start on 64 KB");
                }
                assert_eq!(off % 0x1000, 0, "{variant}: {name}");
                end = off + size;
            }
            assert!(end <= flash, "{variant}: the table ends at {end:#x}, past the flash");
            let find = |n: &str| rows.iter().find(|r| r[0] == n).unwrap();
            assert_eq!(num(find("factory")[4]), l.factory, "{variant}: factory");
            assert_eq!((num(find("nvs")[3]), num(find("nvs")[4])), (0x9000, 0x6000), "{variant}: nvs");
            assert_eq!(num(find("otadata")[3]), 0xF000, "{variant}: otadata");
        }
    }

    #[test]
    fn the_4mb_vector_is_4mb() {
        let h = Header::parse(&kat_header()).unwrap();
        assert_eq!(h.variant(), "4mb");
        h.check_layout("4mb", 2).unwrap();
    }

    /// An 8mb bundle from 1.0.0-beta.16 or earlier (and any other `ODFW`
    /// magic this tree has no table for) is refused at the header, in
    /// words, before its signature or anything else is looked at: the
    /// player answers 422 and writes nothing.
    #[test]
    fn a_bundle_for_another_layout_is_refused() {
        let b = unhex(KAT_HEADER_8MB);
        assert_eq!(&b[..8], b"ODFW18mb");
        let e = Header::parse(&b).unwrap_err();
        assert_eq!(e, OtaError::OtherLayout);
        assert_eq!(
            e.to_string(),
            "this update is for another flash layout than this player's: use the DevKit (4 MB) \
             update file (opendmd.dev/install has the latest)"
        );
        for magic in [b"ODFW116m", b"ODFW44mb", b"ODFW\0\0\0\0"] {
            let mut b = kat_header();
            b[..8].copy_from_slice(magic);
            assert_eq!(Header::parse(&b), Err(OtaError::OtherLayout), "{magic:?}");
        }
        // Not an OpenDMD bundle at all.
        let mut b = kat_header();
        b[..4].copy_from_slice(b"XXXX");
        assert_eq!(Header::parse(&b), Err(OtaError::BadMagic));
    }

    #[test]
    fn the_variant_is_signed() {
        // An old 8mb bundle relabelled 4mb: well-formed, so it parses as
        // the 4mb layout, and the signature catches it.
        let mut b = unhex(KAT_HEADER_8MB);
        b[..8].copy_from_slice(&MAGIC);
        let h = Header::parse(&b).unwrap();
        assert_eq!(h.variant(), "4mb");
        assert_eq!(verify_header(&h, &kat_pub()), Err(OtaError::BadSignature));
        // A v3 bundle relabelled v2, and the reverse.
        let mut b = unhex(KAT_HEADER_V3);
        b[..8].copy_from_slice(&MAGIC);
        let h = Header::parse(&b).unwrap();
        assert_eq!(h.table(), 2);
        assert_eq!(verify_header(&h, &kat_pub()), Err(OtaError::BadSignature));
        let mut b = kat_header();
        b[..8].copy_from_slice(&MAGIC_4MB_V3);
        let h = Header::parse(&b).unwrap();
        assert_eq!(verify_header(&h, &kat_pub()), Err(OtaError::BadSignature));
        // A layout nobody knows is another layout's bundle.
        let mut b = kat_header();
        b[5..8].copy_from_slice(b"16m");
        assert_eq!(Header::parse(&b), Err(OtaError::OtherLayout));
    }

    fn app_image(desc_version: &str, len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len.max(APP_PREFIX_LEN)];
        v[0] = ESP_IMAGE_MAGIC;
        v[APP_DESC_OFFSET..APP_DESC_OFFSET + 4].copy_from_slice(&APP_DESC_MAGIC.to_le_bytes());
        let d = desc_version.as_bytes();
        v[APP_DESC_VERSION_OFFSET..APP_DESC_VERSION_OFFSET + d.len()].copy_from_slice(d);
        v.truncate(len);
        v
    }

    #[test]
    fn the_app_descriptor_names_the_layout() {
        let img = app_image("1.0.0-beta.9 8mb", 4096);
        // In odd pieces, as the network delivers them.
        let h = hashed(&img);
        assert_eq!(&h.prefix()[..], &img[..APP_PREFIX_LEN]);
        assert_eq!(
            app_desc_version(h.prefix()).as_deref(),
            Some("1.0.0-beta.9 8mb")
        );
        check_app_desc(h.prefix(), "8mb").unwrap(); // the check itself knows no layouts
        assert_eq!(
            check_app_desc(h.prefix(), "4mb"),
            Err(OtaError::WrongBuild {
                variant: "4mb".into(),
                found: "1.0.0-beta.9 8mb".into()
            })
        );
        check_app_desc(&app_image("0.3.1 4mb", 200), "4mb").unwrap();
        // Not "<version> <variant>": a bare variant, no space, IDF's default.
        for bad in ["4mb", " 4mb", "0.3.14mb", "v0.3-12-gabc"] {
            assert!(
                check_app_desc(&app_image(bad, 200), "4mb").is_err(),
                "{bad}"
            );
        }
        // No descriptor at all (the known-answer image), or too short.
        let e = check_app_desc(&kat_image(), "4mb").unwrap_err();
        assert!(e.to_string().contains("no app descriptor"), "{e}");
        assert!(check_app_desc(&img[..0x40], "8mb").is_err());
        // A short image leaves the rest of the prefix zero.
        let short = hashed(&img[..0x30]);
        assert!(short.prefix()[0x30..].iter().all(|&b| b == 0));
        assert!(check_app_desc(short.prefix(), "8mb").is_err());
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

    /// `tools/fw-sign.py sign`'s bundle from 1.0.0-beta.15 on: the KAT
    /// image E9 01 .. 3F with the key trailer (key 0) appended, the same
    /// seed and version.
    const KAT_HEADER_KEYED: &str = "4f44465731000000302e342e302d746573740000000000000000000000000000000000000000\
        000050000000fea6139e7b4aa6dddb11b6821992e4e57f327d7a26c080cdaf0d7a1d206d61a964367416b01697fa\
        500e0ba06cd4494d1188171a4e833903f00456d795b2ae6b4135ae26af774bd835c7b181c384c0671167332183605f\
        714ea08ccae2fab302";

    fn keyed_image(id: u8) -> Vec<u8> {
        let mut v = kat_image();
        v.extend_from_slice(&KEY_TRAILER_MAGIC);
        v.push(id);
        v.extend_from_slice(&[0; 7]);
        v
    }

    /// The order a firmware from before key ids (1.0.0-beta.14, whose
    /// `Header::parse`, `verify_header`, `ImageHasher`, `check_image` and
    /// `check_app_desc` these still are) checks a bundle in
    /// (`firmware/src/ota.rs` there): it must take a keyed one.
    fn beta14_accepts(header: &[u8], image: &[u8], key: &[u8; 32]) -> Result<Meta, OtaError> {
        let h = Header::parse(header)?;
        // Its variant check: the 4mb magic it knew, table v2.
        h.check_layout("4mb", 2)?;
        let meta = verify_header(&h, key)?;
        meta.check_fits(0x240000)?;
        if u64::from(meta.length) != image.len() as u64 {
            return Err(OtaError::LengthMismatch {
                header: meta.length,
                actual: image.len() as u64,
            });
        }
        let hh = hashed(image);
        let (len, first) = (hh.len(), hh.first());
        meta.check_image(len, first, &hh.finish())?;
        Ok(meta)
    }

    #[test]
    fn a_keyed_bundle_passes_the_beta14_checks() {
        let header = unhex(KAT_HEADER_KEYED);
        let image = keyed_image(0);
        let meta = beta14_accepts(&header, &image, &kat_pub()).unwrap();
        assert_eq!(meta.length, 80);
        // And the new firmware's: the key that verified is the one named.
        let (_, id) = verify_header_any(&Header::parse(&header).unwrap(), &[(0, &kat_pub())]).unwrap();
        let mut tail = KeyTail::default();
        for piece in image.chunks(5) {
            tail.update(piece);
        }
        assert_eq!(tail.key_id(), Some(0));
        check_key_id(id, tail.key_id()).unwrap();
        // The legacy KAT (no trailer) counts as key 0.
        let mut t = KeyTail::default();
        t.update(&kat_image());
        assert_eq!(t.key_id(), None);
        check_key_id(0, t.key_id()).unwrap();
        assert!(matches!(
            check_key_id(0, Some(1)),
            Err(OtaError::WrongKeyId { named: 1, signed_by: 0 })
        ));
        assert!(matches!(
            check_key_id(1, None),
            Err(OtaError::WrongKeyId { named: 0, signed_by: 1 })
        ));
    }

    #[test]
    fn key_tails_in_any_pieces() {
        let image = keyed_image(7);
        for n in 1..=image.len() {
            let mut t = KeyTail::default();
            for piece in image.chunks(n) {
                t.update(piece);
            }
            assert_eq!(t.key_id(), Some(7), "pieces of {n}");
        }
        assert_eq!(key_id_of(&image), Some(7));
        assert_eq!(key_id_of(&image[..image.len() - 1]), None);
        let mut bad = image.clone();
        *bad.last_mut().unwrap() = 1;
        assert_eq!(key_id_of(&bad), None);
        assert_eq!(key_id_of(b"short"), None);
    }

    #[test]
    fn several_keys() {
        let header = Header::parse(&unhex(KAT_HEADER_KEYED)).unwrap();
        let other = [9u8; 32];
        let other = ed25519_dalek::SigningKey::from_bytes(&other).verifying_key().to_bytes();
        let (_, id) = verify_header_any(&header, &[(1, &other), (0, &kat_pub())]).unwrap();
        assert_eq!(id, 0);
        assert_eq!(
            verify_header_any(&header, &[(1, &other)]).unwrap_err(),
            OtaError::BadSignature
        );
    }

    #[test]
    fn versions_in_order() {
        for (a, b) in [
            ("1.0.0-beta.14", "1.0.0-beta.15"),
            ("1.0.0-beta.9", "1.0.0-beta.10"),
            ("1.0.0-beta.15", "1.0.0"),
            ("0.9.9", "1.0.0-beta.1"),
            ("v1.0.0", "1.0.1"),
            ("1.0", "1.0.1"),
            ("1.0.0-alpha", "1.0.0-beta"),
            ("1.0.0-beta", "1.0.0-beta.1"),
            ("1.0.0-1", "1.0.0-alpha"),
            // The beta channel's order (docs/deploying.md "Betas"):
            // 1.0.0-beta.N < 1.0.0 < 1.0.1-beta.1 < 1.0.1.
            ("1.0.0-beta.16", "1.0.0"),
            ("1.0.0", "1.0.1-beta.1"),
            ("1.0.1-beta.1", "1.0.1-beta.2"),
            ("1.0.1-beta.1", "1.0.1"),
            ("1.0.0-beta.16", "1.0.1-beta.1"),
            ("1.0.0-beta.16", "1.0.0-rc.1"),
            ("1.0.9", "1.1.0-beta.1"),
        ] {
            assert!(version_older(a, b), "{a} < {b}");
            assert!(!version_older(b, a), "not {b} < {a}");
        }
        for v in ["1.0.0", "1.0.0-beta.15", "v2.3.4+abc"] {
            assert!(!version_older(v, v), "{v}");
        }
        assert!(!version_older("1.0.0+build", "1.0.0"));
        check_floor("1.0.0-beta.14", None).unwrap();
        check_floor("1.0.0-beta.14", Some("")).unwrap();
        check_floor("1.0.0-beta.15", Some("1.0.0-beta.15")).unwrap();
        assert_eq!(
            check_floor("1.0.0-beta.14", Some("1.0.0-beta.15")).unwrap_err().to_string(),
            "firmware 1.0.0-beta.14 is older than 1.0.0-beta.15, the oldest this player installs"
        );
    }

    /// `tools/fw-sign.py sign-www` with the KAT seed over the empty pack
    /// `www_pack::build("v9.9", [])`.
    const KAT_WWW: &str = "4f44575001004000880000009db293160000040176392e390000000000000000000000000000000000\
        00000000000000000000000000000000000000000000004f4457530000000072dcc9f23ebed4ebe6f51898489334602e\
        594749afce0bec9e5018fff10c7a1eaaa6c21245746539a0633fde1c9ce6cb6a0a59d3e6bb9b302af77898a7aa060b";

    /// What `sign-www` does, here, for packs the tests make up.
    fn sign_www(pack: &[u8], seed: [u8; 32], key_id: u8) -> Vec<u8> {
        use crate::www_pack::{FLAG_SIGNED, SIG_TRAILER_LEN};
        use ed25519_dalek::Signer;
        let mut d = pack.to_vec();
        d[19] |= FLAG_SIGNED;
        let total = (d.len() + SIG_TRAILER_LEN) as u32;
        d[8..12].copy_from_slice(&total.to_le_bytes());
        d[12..16].fill(0);
        let mut msg = www::DOMAIN.to_vec();
        msg.push(key_id);
        msg.extend_from_slice(&sha256(&d));
        let sig = ed25519_dalek::SigningKey::from_bytes(&seed).sign(&msg);
        d.extend_from_slice(&www::TRAILER_MAGIC);
        d.extend_from_slice(&[key_id, 0, 0, 0]);
        d.extend_from_slice(&sig.to_bytes());
        let crc = crate::www_pack::crc32(&d[crate::www_pack::HEADER_LEN..]);
        d[12..16].copy_from_slice(&crc.to_le_bytes());
        d
    }

    fn kat_seed() -> [u8; 32] {
        std::array::from_fn(|i| i as u8)
    }

    #[test]
    fn the_www_vector_verifies_and_matches() {
        let kat = unhex(KAT_WWW);
        assert_eq!(www::verify_bytes(&kat, &[(0, &kat_pub())]), Ok(0));
        let empty = crate::www_pack::build("v9.9", vec![]).unwrap();
        assert_eq!(sign_www(&empty, kat_seed(), 0), kat);
    }

    fn a_pack() -> Vec<u8> {
        use crate::www_pack::{build, PackFile};
        build(
            "v1.0.0-beta.15",
            vec![
                PackFile {
                    path: "index.html".into(),
                    data: b"<!doctype html>hello".to_vec(),
                    gzip: false,
                },
                PackFile {
                    path: "js/app.js".into(),
                    data: vec![7; 1001],
                    gzip: true,
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn a_signed_pack_is_a_pack_to_older_firmware() {
        use crate::www_pack::{Header as WHeader, Pack};
        let signed = sign_www(&a_pack(), kat_seed(), 0);
        // What 1.0.0-beta.14's `www::receive` and `load` check: the header
        // (byte 19 was reserved and never read), the total length, the
        // CRC, every entry.
        let h = WHeader::parse(&signed).unwrap();
        assert_eq!(h.total_len as usize, signed.len());
        assert_eq!(h.version(), "v1.0.0-beta.15");
        let pack = Pack::parse(&signed).unwrap();
        pack.verify().unwrap();
        assert_eq!(pack.get("index.html").unwrap().data, b"<!doctype html>hello");
        assert_eq!(pack.get("js/app.js").unwrap().data.len(), 1001);
        assert!(pack.get("js/app.js").unwrap().gzip);
        assert!(h.signed());
        assert!(!WHeader::parse(&a_pack()).unwrap().signed());
    }

    #[test]
    fn www_signatures() {
        let keys = [(0u8, &kat_pub())];
        let signed = sign_www(&a_pack(), kat_seed(), 0);
        assert_eq!(www::verify_bytes(&signed, &keys), Ok(0));
        // In pieces of every size, as an upload or a card read delivers it.
        for n in [1, 3, 12, 13, 64, 71, 72, 1000] {
            let mut h = www::PackHasher::new(signed.len());
            for p in signed.chunks(n) {
                h.update(p);
            }
            www::verify(&h.finish().unwrap(), &keys).unwrap();
        }
        // Signing again changes nothing (the trailer and flag replaced).
        // Unsigned: refused.
        assert_eq!(www::verify_bytes(&a_pack(), &keys), Err(OtaError::Unsigned));
        // A changed byte anywhere but the CRC field, with the CRC made to
        // match: only the signature can tell.
        for at in [4, 18, 19, 20, 64, 100, signed.len() - 80, signed.len() - 70, signed.len() - 1] {
            let mut b = signed.clone();
            b[at] ^= 1;
            let crc = crate::www_pack::crc32(&b[64..]);
            b[12..16].copy_from_slice(&crc.to_le_bytes());
            assert!(www::verify_bytes(&b, &keys).is_err(), "byte {at}");
        }
        // The CRC field is not signed (it covers the signature).
        let mut b = signed.clone();
        b[12] ^= 1;
        assert_eq!(www::verify_bytes(&b, &keys), Ok(0));
        // Another key; a key id this firmware does not know.
        let other = sign_www(&a_pack(), [5; 32], 0);
        assert_eq!(www::verify_bytes(&other, &keys), Err(OtaError::BadSignature));
        let id1 = sign_www(&a_pack(), kat_seed(), 1);
        assert_eq!(www::verify_bytes(&id1, &keys), Err(OtaError::UnknownKey(1)));
        // Cut short.
        let mut h = www::PackHasher::new(signed.len());
        h.update(&signed[..signed.len() - 1]);
        assert_eq!(h.finish().unwrap_err(), OtaError::Unsigned);
    }
}
