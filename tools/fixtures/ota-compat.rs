//! `make ota-compat` (tools/ota-compat.sh) copies this into a checkout of an
//! older release's `opendmd-core/tests/` and runs it there: that release's
//! own verification code, as its firmware runs it, against files signed
//! now. The firmware's order (firmware/src/ota.rs, www.rs of that
//! release): a bundle's header, its layout, the signature, that it fits,
//! the body length, the streamed SHA-256, the app descriptor; a www pack's
//! header, its length against the body, every entry, the CRC.
//!
//! OTA_COMPAT_BUNDLE, OTA_COMPAT_PUB (32 raw bytes), OTA_COMPAT_WWW: paths.

use opendmd_core::ota::{check_app_desc, verify_header, Header, ImageHasher, HEADER_LEN};
use opendmd_core::www_pack::{Header as WwwHeader, Pack};

fn read(var: &str) -> Vec<u8> {
    let p = std::env::var(var).unwrap_or_else(|_| panic!("{var} is not set"));
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn the_older_firmware_takes_this_bundle() {
    let data = read("OTA_COMPAT_BUNDLE");
    let key: [u8; 32] = read("OTA_COMPAT_PUB").try_into().expect("a 32-byte public key");
    let header = Header::parse(&data[..HEADER_LEN]).expect("the header parses");
    header.check_variant(header.variant()).expect("its layout");
    let meta = verify_header(&header, &key).expect("the signature verifies");
    meta.check_fits(0x240000).expect("it fits the 4mb factory partition");
    let image = &data[HEADER_LEN..];
    assert_eq!(meta.length as usize, image.len(), "the body is header + image");
    let mut h = ImageHasher::new();
    for piece in image.chunks(2048) {
        h.update(piece);
    }
    let (len, first, prefix) = (h.len(), h.first(), *h.prefix());
    meta.check_image(len, first, &h.finish()).expect("the image's length, magic and SHA-256");
    check_app_desc(&prefix, meta.variant).expect("the app descriptor");
    println!(
        "ota-compat: bundle {} ({} B image) verifies with this release's code; its last 16 B: {:?}",
        meta.version,
        meta.length,
        String::from_utf8_lossy(&image[image.len() - 16..image.len() - 8])
    );
}

#[test]
fn the_older_firmware_takes_this_www_pack() {
    let data = read("OTA_COMPAT_WWW");
    let header = WwwHeader::parse(&data).expect("the pack's header parses");
    assert_eq!(header.total_len as usize, data.len(), "the header's length is the body's");
    let pack = Pack::parse(&data).expect("every entry");
    pack.verify().expect("the CRC");
    assert!(pack.get("index.html").is_some(), "it has the app's index.html");
    println!(
        "ota-compat: www pack '{}' ({} B, {} entries) verifies with this release's code",
        header.version(),
        data.len(),
        pack.count()
    );
}
